//! The clipboard window's host: a WebView2 popup inside the island process (see
//! `docs/clipboard-window-contract.md`, where this is the "Host").
//!
//! - **Lifecycle.** Nothing exists until the first `open`. The window and WebView are then
//!   kept warm for [`WARM`] after the window hides (reopening is ~150 ms instead of ~400 ms
//!   cold) and destroyed after that, which ends the browser processes.
//! - **Content.** Only `https://reui.local/` is served, from the bundle embedded in the exe
//!   and the island's image store; every other request, navigation and new window is refused.
//! - **Focus.** The window takes focus so search typing and IME work (the hub allowed us to
//!   take the foreground at hotkey time) and gives it back to the remembered target with
//!   Ditto's `AttachThreadInput` sequence before a paste.
//!
//! WebView2 calls back on this thread from inside message dispatch, often re-entrantly
//! (inside `SetWindowPos`, `ShowWindow`...), so every input goes through one queue that is
//! drained whenever the host isn't already busy. The drag-out's modal loop runs outside it.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

use reui_design::tokens::color;
use reui_protocol::{ClientMsg, ImageId};
use serde_json::{json, Value};
use tokio::sync::mpsc::UnboundedSender;
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::*;
use windows::core::{w, Interface, Result, BOOL, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForWindow, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, SetFocus, VK_LBUTTON, VK_RBUTTON};
use windows::Win32::UI::Shell::SHCreateMemStream;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::dragout;
use crate::images;

/// The module whose window this is (`HubMsg::Window::surface`).
pub const SURFACE: &str = "clipboard";
const CLASS: PCWSTR = w!("ReUI.Clipboard");
const ORIGIN: &str = "https://reui.local/";

/// Default size in DIPs until the page asks for another (`host.resize`).
const DEFAULT_SIZE: (f32, f32) = (760.0, 430.0);
const MIN_SIZE: (f32, f32) = (320.0, 120.0);
/// Between the caret (or pointer) and the window, and kept from the work area's edges.
const GAP: f32 = 8.0;
/// How long a hidden window stays alive for a fast reopen.
const WARM: Duration = Duration::from_secs(60);
const FADE_IN: Duration = Duration::from_millis(120);
const FADE_OUT: Duration = Duration::from_millis(100);
/// How long to wait for the target to come back in front before a paste.
const RESTORE_WAIT: Duration = Duration::from_millis(300);

const TIMER_WARM: usize = 1;
const TIMER_FADE: usize = 2;

static INDEX_HTML: &str = include_str!("../web/clipboard/index.html");
static APP_CSS: &str = include_str!("../web/clipboard/app.css");
static APP_JS: &str = include_str!("../web/clipboard/app.js");

/// The page may only load itself and its images; no network, no inline script.
const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; connect-src 'none'; frame-src 'none'; form-action 'none'; base-uri 'none'";

/// What the hub link delivers to module windows (see `Pill::drain_hub`).
pub enum Inbound {
    Message { surface: String, message: Value },
    Disconnected,
}

impl Inbound {
    pub fn message(surface: String, message: &str) -> Self {
        Inbound::Message { surface, message: serde_json::from_str(message).unwrap_or(Value::Null) }
    }

    /// The message's `"type"`, or "" for anything else.
    pub fn kind(&self) -> &str {
        match self {
            Inbound::Message { message, .. } => kind(message),
            Inbound::Disconnected => "",
        }
    }
}

fn kind(message: &Value) -> &str {
    message.get("type").and_then(Value::as_str).unwrap_or("")
}

// ---- Routing (pure) ----

/// What the host does with a message from the page.
#[derive(Debug, PartialEq, Eq)]
enum PageRoute {
    Ready,
    Close,
    Resize,
    Drag,
    /// A `host.*` message this host doesn't know: never reaches the hub.
    Ignore,
    /// Hide, give focus back to the target, then forward with `"focused"`.
    Paste,
    /// Hide and give focus back, then forward as is.
    Copy,
    Forward,
}

fn page_route(kind: &str) -> PageRoute {
    match kind {
        "host.ready" => PageRoute::Ready,
        "host.close" => PageRoute::Close,
        "host.resize" => PageRoute::Resize,
        "host.drag" => PageRoute::Drag,
        k if k.starts_with("host.") || k.is_empty() => PageRoute::Ignore,
        "paste" | "stack" => PageRoute::Paste,
        "copy" => PageRoute::Copy,
        _ => PageRoute::Forward,
    }
}

/// What the host does with a message from the hub.
#[derive(Debug, PartialEq, Eq)]
enum HubRoute {
    Open,
    Close,
    /// The answer to a drag-out's `export`: the host's alone.
    Export,
    Page,
}

fn hub_route(kind: &str) -> HubRoute {
    match kind {
        "open" => HubRoute::Open,
        "close" => HubRoute::Close,
        "export" => HubRoute::Export,
        _ => HubRoute::Page,
    }
}

// ---- Resources (pure) ----

#[derive(Debug, PartialEq, Eq)]
enum Resource {
    /// Content type and body; `document` gets the CSP.
    File { mime: &'static str, body: &'static str, document: bool },
    Image(ImageId),
    NotFound,
    /// Anything outside our origin.
    Forbidden,
}

fn resource(uri: &str) -> Resource {
    let Some(rest) = uri.strip_prefix(ORIGIN) else { return Resource::Forbidden };
    let path = rest.split(['?', '#']).next().unwrap_or("");
    match path {
        "" | "index.html" => Resource::File { mime: "text/html; charset=utf-8", body: INDEX_HTML, document: true },
        "app.css" => Resource::File { mime: "text/css; charset=utf-8", body: APP_CSS, document: false },
        "app.js" => Resource::File { mime: "text/javascript; charset=utf-8", body: APP_JS, document: false },
        _ => match path.strip_prefix("image/").map(str::parse::<u64>) {
            Some(Ok(id)) => Resource::Image(ImageId(id)),
            _ => Resource::NotFound,
        },
    }
}

/// Content type from the first bytes (what the hub sends: PNG, JPEG, GIF, WebP, ICO, SVG).
fn image_mime(bytes: &[u8]) -> &'static str {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => "image/png",
        [0xFF, 0xD8, ..] => "image/jpeg",
        [b'G', b'I', b'F', ..] => "image/gif",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "image/webp",
        [0, 0, 1, 0, ..] => "image/x-icon",
        _ if String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).contains("<svg") => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

// ---- Placement (pure) ----

/// A screen rectangle in physical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub l: i32,
    pub t: i32,
    pub r: i32,
    pub b: i32,
}

impl Rect {
    fn w(&self) -> i32 {
        self.r - self.l
    }
    fn h(&self) -> i32 {
        self.b - self.t
    }
}

impl From<RECT> for Rect {
    fn from(r: RECT) -> Self {
        Rect { l: r.left, t: r.top, r: r.right, b: r.bottom }
    }
}

/// The window's size in pixels at `dpi`, no larger than the work area less a gap all round.
fn size_px(dips: (f32, f32), dpi: u32, work: Rect, gap: i32) -> (i32, i32) {
    let s = dpi as f32 / 96.0;
    let w = ((dips.0 * s).round() as i32).min(work.w() - 2 * gap).max(1);
    let h = ((dips.1 * s).round() as i32).min(work.h() - 2 * gap).max(1);
    (w, h)
}

/// Below the anchor (the caret, or the pointer as an empty rectangle), left-aligned to it;
/// above it when below would leave the work area; on the roomier side, clamped, when neither
/// fits. Always inside the work area horizontally.
fn place(anchor: Rect, size: (i32, i32), work: Rect, gap: i32) -> Rect {
    let (w, h) = size;
    let below = anchor.b + gap;
    let above = anchor.t - gap - h;
    let t = if below + h <= work.b - gap {
        below
    } else if above >= work.t + gap {
        above
    } else if work.b - anchor.b >= anchor.t - work.t {
        work.b - gap - h
    } else {
        work.t + gap
    };
    let t = t.clamp(work.t, (work.b - h).max(work.t));
    let l = anchor.l.clamp(work.l + gap, (work.r - gap - w).max(work.l + gap));
    Rect { l, t, r: l + w, b: t + h }
}

// ---- Warmth (pure) ----

/// When the hidden window may be torn down.
#[derive(Default, Debug)]
struct Warmth {
    hidden_at: Option<Instant>,
}

impl Warmth {
    fn shown(&mut self) {
        self.hidden_at = None;
    }
    fn hidden(&mut self, now: Instant) {
        self.hidden_at = Some(now);
    }
    /// Time left before teardown: `None` while shown, zero once due.
    fn remaining(&self, now: Instant) -> Option<Duration> {
        self.hidden_at.map(|t| WARM.saturating_sub(now.saturating_duration_since(t)))
    }
}

/// Window opacity during a fade, from `from` towards shown (1) or hidden (0).
fn fade_opacity(from: f32, showing: bool, elapsed: Duration) -> (f32, bool) {
    let total = if showing { FADE_IN } else { FADE_OUT };
    let to = if showing { 1.0 } else { 0.0 };
    let t = (elapsed.as_secs_f32() / total.as_secs_f32()).min(1.0);
    // Ease out (cubic) on the way in, ease in on the way out.
    let k = if showing { 1.0 - (1.0 - t).powi(3) } else { t * t };
    (from + (to - from) * k, t >= 1.0)
}

// ---- Host ----

struct Target {
    hwnd: HWND,
    focus: HWND,
}

struct Web {
    controller: ICoreWebView2Controller,
    webview: ICoreWebView2,
}

struct Fade {
    start: Instant,
    from: f32,
    showing: bool,
}

enum Input {
    Hub(Value),
    Disconnected,
    Page(String),
    Env(u64, Result<ICoreWebView2Environment>),
    Controller(u64, Result<ICoreWebView2Controller>),
    Crashed(u64),
    /// Our window lost activation to this one.
    Deactivated(HWND),
    CloseRequested,
    Resized,
    Moved,
    Dpi(u32),
    Focus,
    Timer(usize),
    DragEnded,
}

/// Work the host hands back to run outside its borrow.
enum After {
    Drag(dragout::Export),
}

struct Host {
    to_hub: UnboundedSender<ClientMsg>,
    hwnd: Option<HWND>,
    env: Option<ICoreWebView2Environment>,
    web: Option<Web>,
    creating: bool,
    /// Bumped whenever the WebView is torn down, so late callbacks for it are ignored.
    generation: u64,
    /// The page said `host.ready`; until then messages for it wait in `outbox`.
    ready: bool,
    outbox: Vec<String>,
    /// The user sees the window (shown and not closing).
    visible: bool,
    /// When it last appeared: activation can bounce for a moment while it takes the
    /// foreground, which must not read as "clicked elsewhere".
    shown_at: Option<Instant>,
    target: Option<Target>,
    anchor: Rect,
    size: (f32, f32),
    dpi: u32,
    warm: Warmth,
    /// The card a drag-out is waiting on the hub's `export` for.
    drag: Option<u64>,
    dragging: bool,
    fade: Option<Fade>,
    opacity: f32,
    /// The latest view, replayed if the page has to be reloaded (renderer crash).
    last_view: Option<String>,
}

thread_local! {
    static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
    static QUEUE: RefCell<VecDeque<Input>> = const { RefCell::new(VecDeque::new()) };
}

/// Registers the window class; nothing else is created until the first `open`.
pub fn init(to_hub: UnboundedSender<ClientMsg>) -> Result<()> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let [r, g, b, _] = color::island::BACKGROUND.to_rgba8();
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: CreateSolidBrush(COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)),
            lpszClassName: CLASS,
            ..Default::default()
        };
        if RegisterClassExW(&class) == 0 {
            return Err(windows::core::Error::from_thread());
        }
    }
    HOST.with(|cell| {
        *cell.borrow_mut() = Some(Host {
            to_hub,
            hwnd: None,
            env: None,
            web: None,
            creating: false,
            generation: 0,
            ready: false,
            outbox: Vec::new(),
            visible: false,
            shown_at: None,
            target: None,
            anchor: Rect::default(),
            size: DEFAULT_SIZE,
            dpi: 96,
            warm: Warmth::default(),
            drag: None,
            dragging: false,
            fade: None,
            opacity: 1.0,
            last_view: None,
        })
    });
    Ok(())
}

/// Messages from the hub link, in stream order.
pub fn deliver(inbound: Vec<Inbound>) {
    for item in inbound {
        match item {
            Inbound::Message { surface, message } if surface == SURFACE => dispatch(Input::Hub(message)),
            Inbound::Message { .. } => {}
            Inbound::Disconnected => dispatch(Input::Disconnected),
        }
    }
}

/// Tears everything down (the island is quitting).
pub fn shutdown() {
    QUEUE.with(|q| q.borrow_mut().clear());
    HOST.with(|cell| {
        if let Some(mut host) = cell.borrow_mut().take() {
            host.destroy();
        }
    });
}

fn dispatch(input: Input) {
    QUEUE.with(|q| q.borrow_mut().push_back(input));
    pump();
}

fn pump() {
    loop {
        let after = HOST.with(|cell| {
            // Busy: the outer pump picks the input up when the current one is done.
            let Ok(mut slot) = cell.try_borrow_mut() else { return None };
            let host = slot.as_mut()?;
            while let Some(input) = QUEUE.with(|q| q.borrow_mut().pop_front()) {
                if let Some(after) = host.handle(input) {
                    return Some(after);
                }
            }
            None
        });
        match after {
            Some(After::Drag(export)) => {
                if let Err(e) = dragout::run(export) {
                    eprintln!("clipboard window: drag failed: {e}");
                }
                QUEUE.with(|q| q.borrow_mut().push_back(Input::DragEnded));
            }
            None => break,
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_ACTIVATE => {
            if wparam.0 & 0xffff == WA_INACTIVE as usize {
                dispatch(Input::Deactivated(HWND(lparam.0 as *mut _)));
            }
            LRESULT(0)
        }
        WM_SETFOCUS => {
            dispatch(Input::Focus);
            LRESULT(0)
        }
        WM_SIZE => {
            dispatch(Input::Resized);
            LRESULT(0)
        }
        WM_MOVE => {
            dispatch(Input::Moved);
            LRESULT(0)
        }
        WM_DPICHANGED => {
            dispatch(Input::Dpi((wparam.0 & 0xffff) as u32));
            LRESULT(0)
        }
        WM_TIMER => {
            dispatch(Input::Timer(wparam.0));
            LRESULT(0)
        }
        WM_CLOSE => {
            dispatch(Input::CloseRequested);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

impl Host {
    fn handle(&mut self, input: Input) -> Option<After> {
        match input {
            Input::Hub(message) => return self.on_hub(message),
            Input::Page(text) => return self.on_page(&text),
            Input::Disconnected => {
                self.close(true);
            }
            Input::Env(generation, result) if generation == self.generation => self.on_env(result),
            Input::Controller(generation, result) if generation == self.generation => self.on_controller(result),
            Input::Env(..) | Input::Controller(..) => {}
            Input::Crashed(generation) if generation == self.generation => {
                eprintln!("clipboard window: WebView process failed; reloading");
                self.drop_web();
                if self.visible {
                    self.outbox.extend(self.last_view.clone());
                    self.outbox.push(json!({ "type": "host.shown" }).to_string());
                    self.ensure_web();
                }
            }
            Input::Crashed(_) => {}
            Input::Deactivated(other) => {
                // Our own popups (and the WebView's) don't count; neither does a drop target
                // activating itself mid-drag. Clicking elsewhere chose elsewhere: no restore.
                let settling = self.shown_at.is_some_and(|t| t.elapsed() < Duration::from_millis(400));
                if self.visible && !self.dragging && !settling && !self.owns(other) {
                    self.close(false);
                }
            }
            Input::CloseRequested => {
                self.close(true);
            }
            Input::Resized => self.fit_webview(),
            Input::Moved => {
                if let Some(web) = &self.web {
                    unsafe { _ = web.controller.NotifyParentWindowPositionChanged() };
                }
            }
            Input::Dpi(dpi) => {
                // Placement already sized for the new monitor; anything else (a scale change
                // in Settings) re-places at the new DPI.
                if dpi != self.dpi && self.visible {
                    self.place();
                }
            }
            Input::Focus => self.focus_webview(),
            Input::Timer(TIMER_FADE) => self.fade_tick(),
            Input::Timer(TIMER_WARM) => self.warm_tick(),
            Input::Timer(_) => {}
            Input::DragEnded => self.dragging = false,
        }
        None
    }

    // ---- Hub and page ----

    fn on_hub(&mut self, message: Value) -> Option<After> {
        if !message.is_object() {
            return None;
        }
        match hub_route(kind(&message)) {
            HubRoute::Open => self.open(&message),
            HubRoute::Close => {
                self.close(true);
            }
            HubRoute::Export => return self.export(&message),
            // A hidden page gets a full view with the next `open`.
            HubRoute::Page if self.visible => {
                let json = message.to_string();
                if kind(&message) == "state" {
                    self.last_view = Some(json.clone());
                }
                self.post(json);
            }
            HubRoute::Page => {}
        }
        None
    }

    fn on_page(&mut self, text: &str) -> Option<After> {
        let Ok(mut message) = serde_json::from_str::<Value>(text) else { return None };
        // The page may post either an object or its JSON text.
        if let Some(inner) = message.as_str().and_then(|s| serde_json::from_str::<Value>(s).ok()) {
            message = inner;
        }
        match page_route(kind(&message)) {
            PageRoute::Ready => {
                self.ready = true;
                for json in std::mem::take(&mut self.outbox) {
                    self.post(json);
                }
            }
            PageRoute::Close => {
                self.close(true);
            }
            PageRoute::Resize => {
                let get = |k: &str, default: f32| message.get(k).and_then(Value::as_f64).map_or(default, |v| v as f32);
                self.size = (get("w", self.size.0).max(MIN_SIZE.0), get("h", self.size.1).max(MIN_SIZE.1));
                if self.visible {
                    self.place();
                }
            }
            PageRoute::Drag => {
                if let Some(id) = message.get("id").and_then(Value::as_u64) {
                    self.drag = Some(id);
                    self.send(json!({ "type": "export", "id": id }));
                }
            }
            PageRoute::Ignore => {}
            PageRoute::Paste => {
                let focused = self.close(true);
                message["focused"] = focused.into();
                self.send(message);
            }
            PageRoute::Copy => {
                self.close(true);
                self.send(message);
            }
            PageRoute::Forward => self.send(message),
        }
        None
    }

    fn send(&self, message: Value) {
        _ = self.to_hub.send(ClientMsg::Window { surface: SURFACE.into(), message: message.to_string() });
    }

    /// To the page now, or once it says `host.ready`.
    fn post(&mut self, json: String) {
        match &self.web {
            Some(web) if self.ready => unsafe {
                if let Err(e) = web.webview.PostWebMessageAsJson(&HSTRING::from(json)) {
                    eprintln!("clipboard window: post failed: {e}");
                }
            },
            _ => self.outbox.push(json),
        }
    }

    // ---- Open and close ----

    fn open(&mut self, message: &Value) {
        let target = &message["target"];
        let handle = |k: &str| HWND(target.get(k).and_then(Value::as_i64).unwrap_or(0) as isize as *mut _);
        self.target = Some(Target { hwnd: handle("hwnd"), focus: handle("focus") });
        self.anchor = match target.get("caret").and_then(Value::as_array).map(|c| c.iter().map(|v| v.as_f64().unwrap_or(0.0) as i32).collect::<Vec<_>>()) {
            Some(c) if c.len() == 4 => Rect { l: c[0], t: c[1], r: c[2], b: c[3] },
            _ => {
                let mut pt = POINT::default();
                unsafe { _ = GetCursorPos(&mut pt) };
                Rect { l: pt.x, t: pt.y, r: pt.x, b: pt.y }
            }
        };
        let hwnd = match self.ensure_window() {
            Ok(hwnd) => hwnd,
            Err(e) => {
                eprintln!("clipboard window: can't create the window: {e}");
                return;
            }
        };
        // A full view supersedes anything the page hasn't seen yet.
        if !self.ready {
            self.outbox.clear();
        }
        // Script runs again before the view arrives (it was suspended while hidden).
        if let Some(w3) = self.web.as_ref().and_then(|web| web.webview.cast::<ICoreWebView2_3>().ok()) {
            unsafe { _ = w3.Resume() };
        }
        let view = json!({ "type": "state", "view": message.get("view").cloned().unwrap_or(Value::Null) }).to_string();
        self.last_view = Some(view.clone());
        self.post(view);
        self.ensure_web();
        unsafe { _ = KillTimer(Some(hwnd), TIMER_WARM) };
        self.warm.shown();
        self.place();
        if !self.visible {
            self.show(hwnd);
        }
        take_foreground(hwnd);
        self.focus_webview();
        self.post(json!({ "type": "host.shown" }).to_string());
        self.send(json!({ "type": "opened" }));
    }

    fn show(&mut self, hwnd: HWND) {
        self.visible = true;
        self.shown_at = Some(Instant::now());
        let animate = crate::animations_enabled();
        if animate {
            self.fade = Some(Fade { start: Instant::now(), from: if self.fade.is_some() { self.opacity } else { 0.0 }, showing: true });
            self.set_opacity(hwnd, self.fade.as_ref().unwrap().from);
            unsafe { SetTimer(Some(hwnd), TIMER_FADE, 8, None) };
        } else {
            self.fade = None;
            self.set_opacity(hwnd, 1.0);
        }
        unsafe { _ = ShowWindow(hwnd, SW_SHOW) };
        if let Some(web) = &self.web {
            unsafe {
                _ = web.controller.SetIsVisible(true);
                if let Ok(w3) = web.webview.cast::<ICoreWebView2_3>() {
                    _ = w3.Resume();
                }
                if let Ok(w19) = web.webview.cast::<ICoreWebView2_19>() {
                    _ = w19.SetMemoryUsageTargetLevel(COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL);
                }
            }
        }
    }

    /// Hides the window; with `restore`, first gives focus back to the target. Returns
    /// whether the target is in front afterwards (what a paste needs).
    fn close(&mut self, restore: bool) -> bool {
        let target_front = |t: &Option<Target>| t.as_ref().is_some_and(|t| unsafe { GetForegroundWindow() } == t.hwnd);
        if !self.visible {
            return target_front(&self.target);
        }
        self.visible = false;
        self.drag = None;
        let focused = if restore { self.target.as_ref().is_some_and(restore_focus) } else { target_front(&self.target) };
        self.post(json!({ "type": "host.hidden" }).to_string());
        self.send(json!({ "type": "closed" }));
        let Some(hwnd) = self.hwnd else { return focused };
        // Focus is already back where it belongs; the fade is only cosmetic.
        if crate::animations_enabled() {
            self.fade = Some(Fade { start: Instant::now(), from: self.opacity, showing: false });
            unsafe { SetTimer(Some(hwnd), TIMER_FADE, 8, None) };
        } else {
            self.finish_hide(hwnd);
        }
        self.warm.hidden(Instant::now());
        unsafe { SetTimer(Some(hwnd), TIMER_WARM, WARM.as_millis() as u32, None) };
        focused
    }

    fn finish_hide(&mut self, hwnd: HWND) {
        self.fade = None;
        unsafe {
            _ = KillTimer(Some(hwnd), TIMER_FADE);
            _ = ShowWindow(hwnd, SW_HIDE);
        }
        self.set_opacity(hwnd, 1.0);
        if let Some(web) = &self.web {
            unsafe {
                _ = web.controller.SetIsVisible(false);
                // Hidden and idle: stop its timers and let it trim memory until reopened.
                if let Ok(w3) = web.webview.cast::<ICoreWebView2_3>() {
                    _ = w3.TrySuspend(&TrySuspendCompletedHandler::create(Box::new(|_, _| Ok(()))));
                }
                if let Ok(w19) = web.webview.cast::<ICoreWebView2_19>() {
                    _ = w19.SetMemoryUsageTargetLevel(COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW);
                }
            }
        }
    }

    fn fade_tick(&mut self) {
        let (Some(hwnd), Some(fade)) = (self.hwnd, self.fade.as_ref()) else { return };
        let (opacity, done) = fade_opacity(fade.from, fade.showing, fade.start.elapsed());
        let showing = fade.showing;
        self.set_opacity(hwnd, opacity);
        if !done {
            return;
        }
        if showing {
            self.fade = None;
            unsafe { _ = KillTimer(Some(hwnd), TIMER_FADE) };
        } else {
            self.finish_hide(hwnd);
        }
    }

    /// Layered only while partly transparent, so the WebView renders normally at rest.
    fn set_opacity(&mut self, hwnd: HWND, opacity: f32) {
        self.opacity = opacity;
        unsafe {
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let layered = ex & WS_EX_LAYERED.0 as isize != 0;
            if opacity < 1.0 {
                if !layered {
                    SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | WS_EX_LAYERED.0 as isize);
                }
                _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), (opacity * 255.0).round() as u8, LWA_ALPHA);
            } else if layered {
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex & !(WS_EX_LAYERED.0 as isize));
                _ = RedrawWindow(Some(hwnd), None, None, RDW_ERASE | RDW_INVALIDATE | RDW_FRAME | RDW_ALLCHILDREN);
            }
        }
    }

    fn warm_tick(&mut self) {
        let Some(hwnd) = self.hwnd else { return };
        unsafe { _ = KillTimer(Some(hwnd), TIMER_WARM) };
        match self.warm.remaining(Instant::now()) {
            Some(left) if left.is_zero() && !self.visible => self.destroy(),
            Some(left) => unsafe {
                SetTimer(Some(hwnd), TIMER_WARM, left.as_millis().max(1) as u32, None);
            },
            None => {}
        }
    }

    // ---- Window, placement, focus ----

    fn ensure_window(&mut self) -> Result<HWND> {
        if let Some(hwnd) = self.hwnd {
            return Ok(hwnd);
        }
        let hwnd = unsafe {
            let instance = GetModuleHandleW(None)?;
            let hwnd = CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_TOPMOST, CLASS, w!("Clipboard"), WS_POPUP | WS_CLIPCHILDREN, 0, 0, 1, 1, None, None, Some(instance.into()), None)?;
            let round = DWMWCP_ROUND;
            _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &round as *const _ as *const _, size_of_val(&round) as u32);
            let dark = BOOL(1);
            _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const _ as *const _, size_of_val(&dark) as u32);
            hwnd
        };
        self.dpi = unsafe { GetDpiForWindow(hwnd) };
        self.hwnd = Some(hwnd);
        Ok(hwnd)
    }

    /// Sizes and positions the window against the anchor on the anchor's monitor, at that
    /// monitor's DPI.
    fn place(&mut self) {
        let Some(hwnd) = self.hwnd else { return };
        unsafe {
            let monitor = MonitorFromPoint(POINT { x: self.anchor.l, y: self.anchor.t }, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            if !GetMonitorInfoW(monitor, &mut info).as_bool() {
                return;
            }
            let (mut dpi, mut _dpi_y) = (96u32, 96u32);
            _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut _dpi_y);
            let work = Rect::from(info.rcWork);
            let gap = (GAP * dpi as f32 / 96.0).round() as i32;
            let r = place(self.anchor, size_px(self.size, dpi, work, gap), work, gap);
            // Set first: moving onto another monitor sends WM_DPICHANGED, which then
            // matches and is ignored.
            self.dpi = dpi;
            let flags = SWP_NOACTIVATE | SWP_NOOWNERZORDER;
            _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), r.l, r.t, r.w(), r.h(), flags);
            // Crossing to a monitor of another DPI may have rescaled it on the way.
            let mut now = RECT::default();
            if GetWindowRect(hwnd, &mut now).is_ok() && Rect::from(now) != r {
                _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), r.l, r.t, r.w(), r.h(), flags);
            }
        }
        self.fit_webview();
    }

    fn fit_webview(&self) {
        let (Some(hwnd), Some(web)) = (self.hwnd, &self.web) else { return };
        let mut client = RECT::default();
        unsafe {
            if GetClientRect(hwnd, &mut client).is_ok() {
                _ = web.controller.SetBounds(client);
            }
        }
    }

    fn focus_webview(&self) {
        if let (true, Some(web)) = (self.visible, &self.web) {
            unsafe { _ = web.controller.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC) };
        }
    }

    /// Our window, or a popup owned by it (menus, the WebView's own popups).
    fn owns(&self, other: HWND) -> bool {
        let Some(hwnd) = self.hwnd else { return false };
        !other.is_invalid() && unsafe { other == hwnd || GetAncestor(other, GA_ROOTOWNER) == hwnd || GetAncestor(other, GA_ROOT) == hwnd }
    }

    // ---- Drag-out ----

    fn export(&mut self, message: &Value) -> Option<After> {
        let id = message.get("id").and_then(Value::as_u64);
        if id.is_none() || self.drag != id || !self.visible {
            return None;
        }
        self.drag = None;
        // Released before the hub answered: that was a click, not a drag.
        let button = if unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0 { VK_RBUTTON } else { VK_LBUTTON };
        if unsafe { GetAsyncKeyState(button.0 as i32) } as u16 & 0x8000 == 0 {
            return None;
        }
        let export = dragout::Export {
            text: message.get("text").and_then(Value::as_str).map(str::to_owned),
            png: message.get("png").and_then(Value::as_u64).and_then(|id| images::get(ImageId(id)).map(|b| (id, b.to_vec()))),
            files: message.get("files").and_then(Value::as_array).map(|f| f.iter().filter_map(Value::as_str).map(Into::into).collect()).unwrap_or_default(),
        };
        self.dragging = true;
        Some(After::Drag(export))
    }

    // ---- WebView ----

    fn ensure_web(&mut self) {
        if self.web.is_some() || self.creating {
            return;
        }
        self.creating = true;
        if let Some(env) = self.env.clone() {
            self.on_env(Ok(env));
            return;
        }
        let generation = self.generation;
        let handler = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(move |result, env| {
            dispatch(Input::Env(generation, result.and_then(|_| env.ok_or_else(|| E_POINTER.into()))));
            Ok(())
        }));
        let folder = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir).join("ReUI").join("webview");
        // The browser paints this before our first frame, so there's no white flash.
        std::env::set_var("WEBVIEW2_DEFAULT_BACKGROUND_COLOR", "FF000000");
        let options = CoreWebView2EnvironmentOptions::default();
        // Local content only: no SmartScreen lookups, no out-of-process PDF/web UI helpers.
        unsafe { options.set_additional_browser_arguments("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection".into()) };
        let options: ICoreWebView2EnvironmentOptions = options.into();
        let started = unsafe { CreateCoreWebView2EnvironmentWithOptions(PCWSTR::null(), &HSTRING::from(folder.as_os_str()), &options, &handler) };
        if let Err(e) = started {
            self.unavailable(e);
        }
    }

    fn on_env(&mut self, result: Result<ICoreWebView2Environment>) {
        let started = result.and_then(|env| {
            let hwnd = self.hwnd.ok_or_else(|| windows::core::Error::from(E_UNEXPECTED))?;
            let generation = self.generation;
            let handler = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(move |result, controller| {
                dispatch(Input::Controller(generation, result.and_then(|_| controller.ok_or_else(|| E_POINTER.into()))));
                Ok(())
            }));
            unsafe { env.CreateCoreWebView2Controller(hwnd, &handler)? };
            self.env = Some(env);
            Ok(())
        });
        if let Err(e) = started {
            self.unavailable(e);
        }
    }

    fn on_controller(&mut self, result: Result<ICoreWebView2Controller>) {
        self.creating = false;
        let web = result.and_then(|controller| self.configure(controller));
        match web {
            Ok(web) => {
                self.web = Some(web);
                self.fit_webview();
                if let Some(web) = &self.web {
                    unsafe { _ = web.controller.SetIsVisible(self.visible) };
                }
                self.focus_webview();
            }
            Err(e) => self.unavailable(e),
        }
    }

    fn configure(&self, controller: ICoreWebView2Controller) -> Result<Web> {
        let env = self.env.clone().ok_or_else(|| windows::core::Error::from(E_UNEXPECTED))?;
        let generation = self.generation;
        unsafe {
            let webview = controller.CoreWebView2()?;
            let [r, g, b, _] = color::island::BACKGROUND.to_rgba8();
            controller.cast::<ICoreWebView2Controller2>()?.SetDefaultBackgroundColor(COREWEBVIEW2_COLOR { A: 255, R: r, G: g, B: b })?;

            let settings = webview.Settings()?;
            settings.SetAreDevToolsEnabled(false)?;
            settings.SetAreDefaultContextMenusEnabled(false)?;
            settings.SetIsZoomControlEnabled(false)?;
            settings.SetIsStatusBarEnabled(false)?;
            settings.SetAreDefaultScriptDialogsEnabled(false)?;
            settings.SetIsBuiltInErrorPageEnabled(false)?;
            settings.SetAreHostObjectsAllowed(false)?;
            if let Ok(s) = settings.cast::<ICoreWebView2Settings3>() {
                s.SetAreBrowserAcceleratorKeysEnabled(false)?;
            }
            if let Ok(s) = settings.cast::<ICoreWebView2Settings4>() {
                s.SetIsPasswordAutosaveEnabled(false)?;
                s.SetIsGeneralAutofillEnabled(false)?;
            }
            if let Ok(s) = settings.cast::<ICoreWebView2Settings5>() {
                s.SetIsPinchZoomEnabled(false)?;
            }
            if let Ok(s) = settings.cast::<ICoreWebView2Settings6>() {
                s.SetIsSwipeNavigationEnabled(false)?;
            }

            let mut token = 0i64;
            // Every request, ours or not, is answered here: nothing reaches the network.
            webview.AddWebResourceRequestedFilter(w!("*"), COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL)?;
            webview.add_WebResourceRequested(
                &WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else { return Ok(()) };
                    let mut uri = PWSTR::null();
                    args.Request()?.Uri(&mut uri)?;
                    let uri = CoTaskMemPWSTR::from(uri).to_string();
                    args.SetResponse(&respond(&env, &uri)?)
                })),
                &mut token,
            )?;
            let navigation = || {
                NavigationStartingEventHandler::create(Box::new(|_, args| {
                    let Some(args) = args else { return Ok(()) };
                    let mut uri = PWSTR::null();
                    args.Uri(&mut uri)?;
                    if !CoTaskMemPWSTR::from(uri).to_string().starts_with(ORIGIN) {
                        args.SetCancel(true)?;
                    }
                    Ok(())
                }))
            };
            webview.add_NavigationStarting(&navigation(), &mut token)?;
            webview.add_FrameNavigationStarting(&navigation(), &mut token)?;
            webview.add_NewWindowRequested(
                &NewWindowRequestedEventHandler::create(Box::new(|_, args| args.map_or(Ok(()), |a| a.SetHandled(true)))),
                &mut token,
            )?;
            webview.add_PermissionRequested(
                &PermissionRequestedEventHandler::create(Box::new(|_, args| args.map_or(Ok(()), |a| a.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY)))),
                &mut token,
            )?;
            if let Ok(w4) = webview.cast::<ICoreWebView2_4>() {
                w4.add_DownloadStarting(&DownloadStartingEventHandler::create(Box::new(|_, args| args.map_or(Ok(()), |a| a.SetCancel(true)))), &mut token)?;
            }
            webview.add_WebMessageReceived(
                &WebMessageReceivedEventHandler::create(Box::new(|_, args| {
                    let Some(args) = args else { return Ok(()) };
                    let mut json = PWSTR::null();
                    args.WebMessageAsJson(&mut json)?;
                    dispatch(Input::Page(CoTaskMemPWSTR::from(json).to_string()));
                    Ok(())
                })),
                &mut token,
            )?;
            webview.add_ProcessFailed(
                &ProcessFailedEventHandler::create(Box::new(move |_, _| {
                    dispatch(Input::Crashed(generation));
                    Ok(())
                })),
                &mut token,
            )?;
            webview.Navigate(&HSTRING::from(ORIGIN))?;
            Ok(Web { controller, webview })
        }
    }

    /// The runtime is missing or failed to start (W-9): close, and the hub can fall back.
    fn unavailable(&mut self, e: windows::core::Error) {
        eprintln!("clipboard window: WebView2 unavailable: {e}");
        self.creating = false;
        self.close(true);
        self.drop_web();
    }

    /// Closes the WebView (its processes exit once nothing holds the environment).
    fn drop_web(&mut self) {
        if let Some(web) = self.web.take() {
            unsafe { _ = web.controller.Close() };
        }
        self.env = None;
        self.creating = false;
        self.ready = false;
        self.outbox.clear();
        self.generation += 1;
    }

    fn destroy(&mut self) {
        self.close(false);
        self.drop_web();
        if let Some(hwnd) = self.hwnd.take() {
            unsafe { _ = DestroyWindow(hwnd) };
        }
        self.fade = None;
        self.opacity = 1.0;
        self.warm = Warmth::default();
    }
}

/// The response for one request from the page.
fn respond(env: &ICoreWebView2Environment, uri: &str) -> Result<ICoreWebView2WebResourceResponse> {
    let image;
    let (status, reason, mime, body, document): (i32, &str, &str, Cow<[u8]>, bool) = match resource(uri) {
        Resource::File { mime, body, document } => (200, "OK", mime, Cow::Borrowed(body.as_bytes()), document),
        Resource::Image(id) => match images::get(id) {
            Some(bytes) => {
                image = bytes;
                (200, "OK", image_mime(&image), Cow::Borrowed(&image[..]), false)
            }
            None => (404, "Not Found", "text/plain", Cow::Borrowed(&[][..]), false),
        },
        Resource::NotFound => (404, "Not Found", "text/plain", Cow::Borrowed(&[][..]), false),
        Resource::Forbidden => (403, "Forbidden", "text/plain", Cow::Borrowed(&[][..]), false),
    };
    // Clipboard contents never go to the disk cache.
    let mut headers = format!("Content-Type: {mime}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff");
    if document {
        headers.push_str(&format!("\r\nContent-Security-Policy: {CSP}"));
    }
    unsafe {
        let stream = SHCreateMemStream(Some(&body));
        env.CreateWebResourceResponse(stream.as_ref(), status, &HSTRING::from(reason), &HSTRING::from(headers))
    }
}

/// Brings our window in front. The hub allowed it at hotkey time; if Windows still refuses,
/// borrow the foreground thread's input state for the call.
fn take_foreground(hwnd: HWND) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY};
    unsafe {
        if SetForegroundWindow(hwnd).as_bool() && GetForegroundWindow() == hwnd {
            return;
        }
        // Windows lets the process that received the latest input take the foreground; give
        // ourselves that input with an unassigned key (0xE8), which no app acts on.
        let key = |flags| INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(0xE8), dwFlags: flags, ..Default::default() } } };
        let inputs = [key(Default::default()), key(KEYEVENTF_KEYUP)];
        SendInput(&inputs, size_of::<INPUT>() as i32);
        if SetForegroundWindow(hwnd).as_bool() && GetForegroundWindow() == hwnd {
            return;
        }
        let ours = GetCurrentThreadId();
        let theirs = GetWindowThreadProcessId(GetForegroundWindow(), None);
        let attached = theirs != 0 && theirs != ours && AttachThreadInput(ours, theirs, true).as_bool();
        _ = BringWindowToTop(hwnd);
        _ = SetForegroundWindow(hwnd);
        if attached {
            _ = AttachThreadInput(ours, theirs, false);
        }
    }
}

/// Ditto's sequence: share the target's input state, bring it to the top and in front,
/// focus the control that had focus, then wait for it to actually be in front.
fn restore_focus(target: &Target) -> bool {
    unsafe {
        if target.hwnd.is_invalid() || !IsWindow(Some(target.hwnd)).as_bool() {
            return false;
        }
        let ours = GetCurrentThreadId();
        let theirs = GetWindowThreadProcessId(target.hwnd, None);
        let attached = theirs != 0 && theirs != ours && AttachThreadInput(ours, theirs, true).as_bool();
        _ = BringWindowToTop(target.hwnd);
        _ = SetForegroundWindow(target.hwnd);
        if attached {
            if !target.focus.is_invalid() && IsWindow(Some(target.focus)).as_bool() {
                _ = SetFocus(Some(target.focus));
            }
            _ = AttachThreadInput(ours, theirs, false);
        }
        let deadline = Instant::now() + RESTORE_WAIT;
        loop {
            if GetForegroundWindow() == target.hwnd {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: Rect = Rect { l: 0, t: 0, r: 1920, b: 1040 };

    fn caret(x: i32, y: i32) -> Rect {
        Rect { l: x, t: y, r: x + 2, b: y + 20 }
    }

    #[test]
    fn opens_below_the_caret_left_aligned() {
        let r = place(caret(300, 200), (760, 430), WORK, 8);
        assert_eq!(r, Rect { l: 300, t: 228, r: 1060, b: 658 });
    }

    #[test]
    fn flips_above_near_the_bottom() {
        let r = place(caret(300, 900), (760, 430), WORK, 8);
        assert_eq!((r.t, r.b), (900 - 8 - 430, 900 - 8));
    }

    #[test]
    fn clamps_at_the_right_edge() {
        let r = place(caret(1800, 200), (760, 430), WORK, 8);
        assert_eq!(r.r, 1920 - 8);
        assert_eq!(r.l, 1920 - 8 - 760);
    }

    #[test]
    fn neither_side_fits_takes_the_roomier_one() {
        let short = Rect { l: 0, t: 0, r: 1920, b: 600 };
        let low = place(caret(10, 400), (760, 430), short, 8);
        assert_eq!(low.t, 8, "more room above: pinned to the top");
        let high = place(caret(10, 150), (760, 430), short, 8);
        assert_eq!(high.b, 600 - 8, "more room below: pinned to the bottom");
    }

    #[test]
    fn works_on_a_monitor_left_of_and_above_the_primary() {
        let work = Rect { l: -2560, t: -300, r: 0, b: 1140 };
        let r = place(caret(-2600, 1100), (1140, 645), work, 12);
        assert_eq!(r.l, -2560 + 12, "clamped into the left monitor");
        assert_eq!(r.b, 1100 - 12, "flipped above the caret");
        assert!(r.t >= work.t);
    }

    #[test]
    fn pointer_anchor_uses_the_same_rules() {
        let at = Rect { l: 500, t: 1030, r: 500, b: 1030 };
        let r = place(at, (760, 430), WORK, 8);
        assert_eq!(r.b, 1030 - 8);
    }

    #[test]
    fn size_scales_with_dpi_and_fits_the_work_area() {
        assert_eq!(size_px(DEFAULT_SIZE, 96, WORK, 8), (760, 430));
        assert_eq!(size_px(DEFAULT_SIZE, 144, WORK, 12), (1140, 645));
        let small = Rect { l: 0, t: 0, r: 1024, b: 600 };
        assert_eq!(size_px(DEFAULT_SIZE, 192, small, 16), (1024 - 32, 600 - 32));
    }

    #[test]
    fn page_messages_route() {
        assert_eq!(page_route("host.ready"), PageRoute::Ready);
        assert_eq!(page_route("host.close"), PageRoute::Close);
        assert_eq!(page_route("host.resize"), PageRoute::Resize);
        assert_eq!(page_route("host.drag"), PageRoute::Drag);
        assert_eq!(page_route("host.whatever"), PageRoute::Ignore);
        assert_eq!(page_route(""), PageRoute::Ignore);
        assert_eq!(page_route("paste"), PageRoute::Paste);
        assert_eq!(page_route("stack"), PageRoute::Paste);
        assert_eq!(page_route("copy"), PageRoute::Copy);
        for forwarded in ["query", "layout", "pin", "unpin", "delete", "undo", "collect", "collection.create", "edit", "preview", "ocr", "pause", "export"] {
            assert_eq!(page_route(forwarded), PageRoute::Forward, "{forwarded}");
        }
    }

    #[test]
    fn hub_messages_route() {
        assert_eq!(hub_route("open"), HubRoute::Open);
        assert_eq!(hub_route("close"), HubRoute::Close);
        assert_eq!(hub_route("export"), HubRoute::Export);
        assert_eq!(hub_route("state"), HubRoute::Page);
        assert_eq!(hub_route("preview"), HubRoute::Page);
    }

    #[test]
    fn resources_map_to_the_bundle_and_images() {
        assert!(matches!(resource("https://reui.local/"), Resource::File { document: true, .. }));
        assert!(matches!(resource("https://reui.local/index.html?x=1"), Resource::File { document: true, .. }));
        assert!(matches!(resource("https://reui.local/app.css"), Resource::File { mime: "text/css; charset=utf-8", .. }));
        assert!(matches!(resource("https://reui.local/app.js#x"), Resource::File { mime: "text/javascript; charset=utf-8", .. }));
        assert_eq!(resource("https://reui.local/image/991"), Resource::Image(ImageId(991)));
        assert_eq!(resource("https://reui.local/image/abc"), Resource::NotFound);
        assert_eq!(resource("https://reui.local/favicon.ico"), Resource::NotFound);
        assert_eq!(resource("https://example.com/"), Resource::Forbidden);
        assert_eq!(resource("https://reui.local.evil.com/"), Resource::Forbidden);
        assert_eq!(resource("http://reui.local/"), Resource::Forbidden);
    }

    #[test]
    fn image_types_are_sniffed() {
        assert_eq!(image_mime(&[0x89, b'P', b'N', b'G', 13, 10]), "image/png");
        assert_eq!(image_mime(&[0xFF, 0xD8, 0xFF]), "image/jpeg");
        assert_eq!(image_mime(b"<?xml?><svg xmlns=''>"), "image/svg+xml");
    }

    #[test]
    fn warmth_expires_a_minute_after_hiding() {
        let t0 = Instant::now();
        let mut warm = Warmth::default();
        assert_eq!(warm.remaining(t0), None);
        warm.hidden(t0);
        assert_eq!(warm.remaining(t0 + Duration::from_secs(20)), Some(Duration::from_secs(40)));
        assert_eq!(warm.remaining(t0 + Duration::from_secs(61)), Some(Duration::ZERO));
        warm.shown();
        assert_eq!(warm.remaining(t0 + Duration::from_secs(120)), None);
    }

    #[test]
    fn fades_end_where_they_should() {
        assert_eq!(fade_opacity(0.0, true, Duration::ZERO), (0.0, false));
        assert_eq!(fade_opacity(0.0, true, FADE_IN), (1.0, true));
        assert_eq!(fade_opacity(1.0, false, FADE_OUT * 2), (0.0, true));
        let (mid, done) = fade_opacity(0.4, true, FADE_IN / 2);
        assert!(mid > 0.4 && mid < 1.0 && !done);
    }
    /// End to end against the real runtime: creates the window and WebView, loads the
    /// embedded page, waits for `host.ready`, screenshots it at rest and half transparent
    /// (`%TEMP%\reui-clip-*.png`), then closes and gives focus back.
    #[test]
    #[ignore = "opens a real WebView2 window on the desktop for a few seconds"]
    fn webview_smoke() {
        use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
        use windows::Win32::UI::HiDpi::{SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
        unsafe {
            SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        init(tx).unwrap();
        let fg = unsafe { GetForegroundWindow() };
        let item = json!({ "id": 1, "kind": "text", "text": "Hello from the host smoke test\nsecond line", "rich": false, "title": null, "site": null, "url": null,
            "image": null, "icon": null, "app": "Smoke", "age": "now", "meta": null, "color": null, "files": null, "pinned": false, "collections": [],
            "cut": false, "missing": false, "lines": 2, "ocr": false });
        let view = json!({ "query": "", "collection": "history", "filter": { "type": null, "app": null }, "layout": "strip",
            "collections": [{ "id": "history", "name": "History", "count": 1, "color": null, "builtin": true }],
            "types": [], "apps": [], "items": [item], "total": 1, "paused": false, "undo": false,
            "stack": { "active": false, "remaining": 0 }, "empty": null });
        let open = json!({ "type": "open", "target": { "hwnd": fg.0 as i64, "focus": 0, "caret": [200, 200, 202, 220] }, "view": view });
        let started = Instant::now();
        deliver(vec![Inbound::Message { surface: SURFACE.into(), message: open }]);
        let ready = || HOST.with(|h| h.borrow().as_ref().is_some_and(|h| h.ready));
        while !ready() && started.elapsed() < Duration::from_secs(20) {
            pump_for(Duration::from_millis(20));
        }
        assert!(ready(), "page never said host.ready");
        eprintln!("cold open to host.ready: {:?}", started.elapsed());
        pump_for(Duration::from_millis(800));
        let hwnd = HOST.with(|h| h.borrow().as_ref().unwrap().hwnd.unwrap());
        let rest = capture(hwnd, "rest");
        HOST.with(|h| h.borrow_mut().as_mut().unwrap().set_opacity(hwnd, 0.5));
        pump_for(Duration::from_millis(300));
        let half = capture(hwnd, "half");
        HOST.with(|h| h.borrow_mut().as_mut().unwrap().set_opacity(hwnd, 1.0));
        pump_for(Duration::from_millis(300));
        let back = capture(hwnd, "back");
        eprintln!("lit pixels: rest {rest:.3}, half {half:.3}, back {back:.3}");
        deliver(vec![Inbound::Message { surface: SURFACE.into(), message: json!({ "type": "close" }) }]);
        pump_for(Duration::from_millis(400));
        let mut sent = Vec::new();
        while let Ok(ClientMsg::Window { message, .. }) = rx.try_recv() {
            sent.push(message);
        }
        eprintln!("to hub: {sent:?}");
        assert!(sent.iter().any(|m| m.contains("\"opened\"")) && sent.iter().any(|m| m.contains("\"closed\"")));
        assert!(!sent.iter().any(|m| m.contains("host.")), "host.* never reaches the hub");
        assert!(rest > 0.01, "nothing rendered");
        // A minute later the warm window goes away entirely.
        HOST.with(|h| h.borrow_mut().as_mut().unwrap().warm.hidden(Instant::now() - WARM));
        dispatch(Input::Timer(TIMER_WARM));
        assert!(HOST.with(|h| h.borrow().as_ref().is_some_and(|h| h.hwnd.is_none() && h.web.is_none() && h.env.is_none())));
        shutdown();
    }

    fn pump_for(d: Duration) {
        let end = Instant::now() + d;
        let mut msg = MSG::default();
        while Instant::now() < end {
            unsafe {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Screen pixels under the window, saved as PNG; returns the share that isn't black.
    fn capture(hwnd: HWND, name: &str) -> f32 {
        use windows::Win32::Graphics::Imaging::*;
        use windows::Win32::System::Com::*;
        unsafe {
            let mut r = RECT::default();
            GetWindowRect(hwnd, &mut r).unwrap();
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            let screen = GetDC(None);
            let dc = CreateCompatibleDC(Some(screen));
            let bmp = CreateCompatibleBitmap(screen, w, h);
            let old = SelectObject(dc, bmp.into());
            BitBlt(dc, 0, 0, w, h, Some(screen), r.left, r.top, SRCCOPY).unwrap();
            let mut info = BITMAPINFO::default();
            info.bmiHeader = BITMAPINFOHEADER { biSize: size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() };
            let mut px = vec![0u8; (w * h * 4) as usize];
            GetDIBits(dc, bmp, 0, h as u32, Some(px.as_mut_ptr() as *mut _), &mut info, DIB_RGB_COLORS);
            SelectObject(dc, old);
            _ = DeleteObject(bmp.into());
            _ = DeleteDC(dc);
            ReleaseDC(None, screen);
            px.chunks_exact_mut(4).for_each(|p| p[3] = 255);
            let lit = px.chunks_exact(4).filter(|p| p[0] as u32 + p[1] as u32 + p[2] as u32 > 60).count() as f32 / (w * h) as f32;
            let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).unwrap();
            let path = std::env::temp_dir().join(format!("reui-clip-{name}.png"));
            let stream = wic.CreateStream().unwrap();
            stream.InitializeFromFilename(&HSTRING::from(path.as_os_str()), 0x4000_0000 /* GENERIC_WRITE */).unwrap();
            let encoder = wic.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null()).unwrap();
            encoder.Initialize(&stream, WICBitmapEncoderNoCache).unwrap();
            let mut frame = None;
            encoder.CreateNewFrame(&mut frame, std::ptr::null_mut()).unwrap();
            let frame = frame.unwrap();
            frame.Initialize(None).unwrap();
            frame.SetSize(w as u32, h as u32).unwrap();
            let mut format = GUID_WICPixelFormat32bppBGRA;
            frame.SetPixelFormat(&mut format).unwrap();
            frame.WritePixels(h as u32, (w * 4) as u32, &px).unwrap();
            frame.Commit().unwrap();
            encoder.Commit().unwrap();
            eprintln!("saved {}", path.display());
            lit
        }
    }
}
