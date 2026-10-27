//! The Windows side of the clipboard: one thread with a message-only window that receives
//! `WM_CLIPBOARDUPDATE` (an event per copy, no polling) and the hotkeys, reads what was
//! copied, and writes items back and pastes them.
//!
//! Hotkeys: the history hotkey opens the clipboard window. Before anything else it records
//! the *target* (the foreground window, its focused control and the text caret on screen),
//! so the window can open at the caret and paste back into that field; the caret comes from
//! UI Automation, then MSAA, then the classic system caret (see [`CaretFinder`]).
//! Ctrl+Alt+1…9 paste the Nth most recent item straight into the foreground app.
//!
//! A paste stack watches for the user's own Ctrl+V (or Shift+Insert) with a low-level
//! keyboard hook, installed only while a stack is active.
//!
//! Two threads. The window thread only ever holds the clipboard long enough to copy raw
//! bytes out (or a prepared item in), and never sleeps for more than a few ms, so other
//! apps aren't left waiting on us. Everything slow (image decode/encode, app icons, paste
//! prep, waiting for modifiers, the paste chord) runs on a worker thread. Copies and pastes
//! go through the worker's single queue, so events leave in the order they happened.
//!
//! Timers (all one-shot, so an idle clipboard costs nothing): a copy starts (or restarts)
//! the 75 ms settle timer, because apps often write in several passes; the clipboard is read
//! once when it fires. If another app is still holding it then, the retry timer tries again
//! after 100 ms and 250 ms. The sequence number of the last copy handled, and of our own
//! last write, make repeated notifications and stale timers no-ops.
//!
//! Privacy: content a password manager or other app marks with the standard exclusion
//! formats is never read, and our own writes carry the same marks so Win+V and other
//! clipboard managers don't record a paste-back.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::mpsc as std_mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc::UnboundedSender;
use windows::core::{w, Interface, Result, GUID, PCWSTR, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::Security::{GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TokenIntegrityLevel, TOKEN_MANDATORY_LABEL, TOKEN_QUERY};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, IStream, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, STATFLAG_NONAME, STATSTG, STREAM_SEEK_SET};
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::{CF_DIB, CF_DIBV5, CF_HDROP, CF_UNICODETEXT};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::Ole::{SafeArrayAccessData, SafeArrayDestroy, SafeArrayGetLBound, SafeArrayGetUBound, SafeArrayUnaccessData};
use windows::Win32::System::Threading::{GetCurrentProcess, GetCurrentProcessId, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::System::Variant::{VARIANT, VT_I4};
use windows::Win32::UI::Accessibility::{
    AccessibleObjectFromWindow, CUIAutomation8, IAccessible, IUIAutomation, IUIAutomation2, IUIAutomationTextPattern2, IUIAutomationTextRange, TextUnit_Character, UIA_TextPattern2Id,
};
use windows::Win32::UI::HiDpi::{SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::{DragQueryFileW, SHCreateMemStream, DROPFILES, HDROP};
use windows::Win32::UI::WindowsAndMessaging::*;

/// The default history hotkey: Win + Alt + V.
pub const HOTKEY_LABEL: &str = "Win+Alt+V";

/// The history hotkey, from settings ("Win+Alt+V", "Ctrl+Shift+V").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hotkey {
    /// `HOT_KEY_MODIFIERS` bits (MOD_WIN, MOD_ALT, ...), without MOD_NOREPEAT.
    pub modifiers: u32,
    pub vk: u32,
    pub label: String,
}

impl Default for Hotkey {
    fn default() -> Self {
        Self { modifiers: (MOD_WIN | MOD_ALT).0, vk: u32::from(b'V'), label: HOTKEY_LABEL.into() }
    }
}

impl Hotkey {
    /// "Mod+Mod+Key", case-insensitive. Modifiers: Win, Ctrl, Alt, Shift. Keys: A–Z, 0–9,
    /// F1–F24, a few named keys (Space, Insert, Home, ...) and punctuation the current layout
    /// types without modifiers ("Ctrl+`"). The label comes back normalised ("Win+Alt+V").
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        // "Ctrl++" means the plus key.
        let (mods, key) = match text.strip_suffix("++") {
            Some(mods) => (mods, "+"),
            None => text.rsplit_once('+').unwrap_or(("", text)),
        };
        let mut modifiers = 0;
        for part in mods.split('+').filter(|_| !mods.is_empty()) {
            modifiers |= match part.trim().to_ascii_lowercase().as_str() {
                "win" | "windows" | "super" | "meta" => MOD_WIN.0,
                "ctrl" | "control" => MOD_CONTROL.0,
                "alt" => MOD_ALT.0,
                "shift" => MOD_SHIFT.0,
                _ => return None,
            };
        }
        let (vk, name) = key_code(key.trim())?;
        // A bare letter or symbol would swallow normal typing; only function keys may go alone.
        if modifiers == 0 && !(0x70..=0x87).contains(&vk) {
            return None;
        }
        let mut label = String::new();
        for (bit, part) in [(MOD_WIN, "Win+"), (MOD_CONTROL, "Ctrl+"), (MOD_ALT, "Alt+"), (MOD_SHIFT, "Shift+")] {
            if modifiers & bit.0 != 0 {
                label.push_str(part);
            }
        }
        label.push_str(&name);
        Some(Self { modifiers, vk, label })
    }
}

/// A key name → its virtual-key code and display name.
fn key_code(key: &str) -> Option<(u32, String)> {
    let mut chars = key.chars();
    let first = chars.next()?;
    if chars.as_str().is_empty() {
        let upper = first.to_ascii_uppercase();
        if upper.is_ascii_alphanumeric() {
            return Some((u32::from(upper), upper.to_string()));
        }
        // Punctuation sits on different keys per layout, so ask the current one; only take
        // characters it types without Shift/Ctrl/Alt, or the label would name the wrong chord.
        let mut units = [0u16; 2];
        if first.encode_utf16(&mut units).len() != 1 {
            return None;
        }
        let scan = unsafe { VkKeyScanW(units[0]) } as u16;
        let (vk, shift_state) = (scan & 0xFF, scan >> 8);
        return (vk != 0xFF && shift_state == 0).then(|| (u32::from(vk), first.to_string()));
    }
    let lower = key.to_ascii_lowercase();
    if let Some(n) = lower.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()).filter(|n| (1..=24).contains(n)) {
        return Some((0x6F + n, format!("F{n}")));
    }
    let named: [(&[&str], VIRTUAL_KEY, &str); 11] = [
        (&["space"], VK_SPACE, "Space"),
        (&["insert", "ins"], VK_INSERT, "Insert"),
        (&["delete", "del"], VK_DELETE, "Delete"),
        (&["home"], VK_HOME, "Home"),
        (&["end"], VK_END, "End"),
        (&["pageup", "pgup"], VK_PRIOR, "PageUp"),
        (&["pagedown", "pgdn"], VK_NEXT, "PageDown"),
        (&["up"], VK_UP, "Up"),
        (&["down"], VK_DOWN, "Down"),
        (&["left"], VK_LEFT, "Left"),
        (&["right"], VK_RIGHT, "Right"),
    ];
    named.iter().find(|(names, ..)| names.contains(&lower.as_str())).map(|(_, vk, name)| (u32::from(vk.0), name.to_string()))
}

/// Per clipboard format: text, HTML and RTF bigger than this aren't kept (text over it skips
/// the whole copy rather than storing it cut short).
const MAX_FORMAT_BYTES: usize = 16 << 20;
/// Bigger images are skipped: an 8K screenshot is 33 MP, this is ~1.4 GB of RGBA work.
const MAX_IMAGE_PIXELS: u64 = 50_000_000;
const THUMB_PX: u32 = 320;
const WM_WRITE_READY: u32 = WM_APP + 3;
/// `wparam` 1: a paste stack started (install the keyboard hook); 0: it ended.
const WM_STACK: u32 = WM_APP + 4;
const HOTKEY_ID: i32 = 1;
/// Ctrl+Alt+1…9 are `RECENT_ID + 1` … `RECENT_ID + 9`.
const RECENT_ID: i32 = 10;
const RETRY_TIMER: usize = 1;
const SETTLE_TIMER: usize = 2;
const STACK_TIMER: usize = 3;
/// After the user's paste chord, wait this long before the next stack item replaces the
/// clipboard: the target reads it when it handles the key, which can lag the hook a little.
const STACK_SETTLE_MS: u32 = 200;
/// Whole budget for UI Automation and MSAA to find the caret; then the system caret is used.
const CARET_WAIT: Duration = Duration::from_millis(200);
/// UI Automation's own connection and transaction timeouts, so a hung app can't hold us.
const UIA_TIMEOUT_MS: u32 = 150;
/// Apps write the clipboard in passes (empty, text, then HTML...); read once it settles.
const SETTLE_MS: u32 = 75;
/// When another app still holds the clipboard: retry after 100 ms, then once more after 250 ms.
const RETRY_DELAYS_MS: [u32; 2] = [100, 250];
/// Integrity levels (the last sub-authority of the token's mandatory label).
const MEDIUM_INTEGRITY: u32 = 0x2000;
const HIGH_INTEGRITY: u32 = 0x3000;

#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    /// Plain text, and the rich versions the app offered alongside it (CF_HTML as UTF-8,
    /// RTF bytes) so it pastes back formatted.
    Text { text: String, html: Option<String>, rtf: Option<Vec<u8>> },
    /// Full image (PNG, alpha kept) and a small thumbnail (PNG).
    Image { png: Vec<u8>, thumb: Vec<u8> },
    /// `cut`: Explorer marked it a move ("Preferred DropEffect"); pasted back as a copy.
    Files { paths: Vec<String>, cut: bool },
}

pub struct Captured {
    pub content: Content,
    /// The app it came from (display name), its executable's file name in lower case
    /// ("keepass.exe", for the ignored-apps list), and its icon (PNG).
    pub source: Option<String>,
    pub exe: Option<String>,
    pub icon: Option<Vec<u8>>,
    /// Pixel size of a copied image.
    pub size: Option<(u32, u32)>,
}

/// Where a paste goes back to: the window that was in front when the history hotkey was
/// pressed, its focused control, and the text caret in screen pixels (`None` when no caret
/// was found; the window then opens at the mouse). Windows are `HWND`s as integers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Target {
    pub hwnd: isize,
    pub focus: isize,
    /// Left, top, right, bottom.
    pub caret: Option<[i32; 4]>,
}

impl Target {
    /// The foreground window right now, without looking for the caret (a click on the
    /// island: the mouse is where the user is).
    pub fn now() -> Target {
        unsafe { capture_target(None) }
    }
}

pub enum OsEvent {
    Copied(Captured),
    /// The history hotkey, with the target captured before anything else could move focus.
    Hotkey(Target),
    /// Ctrl+Alt+`n`: paste the `n`th most recent item into `target`.
    RecentPaste { n: u32, target: isize },
    /// The user pasted (Ctrl+V / Shift+Insert outside ReUI) while a stack was active.
    StackPasted,
    /// Esc while a stack was active.
    StackCancel,
    /// A paste couldn't be typed into the focused app (it runs elevated, or focus moved);
    /// the content is on the clipboard for the user to paste themselves.
    PasteBlocked,
    /// Something the user should know about the OS side.
    Status(OsStatus),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsStatus {
    /// No clipboard listener: nothing is being recorded.
    ListenerFailed,
    /// The history hotkey is taken by another app.
    HotkeyTaken,
}

pub enum Command {
    /// Put this on the clipboard and paste it into `target` (an `HWND`; `None`: whatever is
    /// in front now). `plain`: text only (no HTML/RTF).
    Paste { content: Content, plain: bool, target: Option<isize> },
    /// Put this on the clipboard only.
    Copy(Content),
}

/// Handle to the clipboard threads.
pub struct Clipboard {
    jobs: std_mpsc::Sender<Job>,
    hwnd: isize,
}

impl Clipboard {
    /// A paste waits (briefly) for its target to be in front; if it never is, nothing is
    /// typed (`OsEvent::PasteBlocked`) and the item stays on the clipboard.
    pub fn send(&self, command: Command) {
        let job = match command {
            Command::Paste { content, plain, target } => Job::Write { content, plain, target: Some(target.unwrap_or_else(|| unsafe { GetForegroundWindow() }.0 as isize)) },
            Command::Copy(content) => Job::Write { content, plain: false, target: None },
        };
        _ = self.jobs.send(job);
    }

    /// Starts or stops watching for the user's paste chord (a paste stack). The hook lives
    /// only while a stack does: a keyboard hook sees every key, so it's never left running.
    pub fn watch_pastes(&self, on: bool) {
        _ = unsafe { PostMessageW(Some(HWND(self.hwnd as *mut c_void)), WM_STACK, WPARAM(on as usize), LPARAM(0)) };
    }
}

/// Registered clipboard formats (ids are global, so both threads share them; 0 = not
/// registered, never read or written).
#[derive(Clone, Copy)]
struct Formats {
    exclude: u32,
    viewer_ignore: u32,
    history: u32,
    png: u32,
    html: u32,
    rtf: u32,
    drop_effect: u32,
}

/// What the window thread copied out, before any decoding.
enum Raw {
    /// UTF-16LE bytes, and CF_HTML / RTF bytes; all without their NUL terminators.
    Text { utf16: Vec<u8>, html: Option<Vec<u8>>, rtf: Option<Vec<u8>> },
    Png(Vec<u8>),
    /// CF_DIBV5 or CF_DIB.
    Dib(Vec<u8>),
    Files { paths: Vec<String>, cut: bool },
}

/// Work for the worker thread, in arrival order.
enum Job {
    Copied { raw: Raw, owner: Option<String> },
    /// Put `content` on the clipboard; with a `target` window (`HWND` as an integer), also
    /// paste it there.
    Write { content: Content, plain: bool, target: Option<isize> },
}

/// A fully prepared clipboard write, handed to the window thread to set in one go.
struct Write {
    data: Arc<Vec<(u32, Vec<u8>)>>,
    reply: std_mpsc::Sender<std::result::Result<(), String>>,
}

struct State {
    hwnd: HWND,
    events: UnboundedSender<OsEvent>,
    jobs: std_mpsc::Sender<Job>,
    writes: std_mpsc::Receiver<Write>,
    formats: Formats,
    /// Sequence number of our own last write, so pasting doesn't record itself as a copy.
    own_write: u32,
    /// Last sequence number fully handled (read, skipped or excluded): repeated
    /// `WM_CLIPBOARDUPDATE`s and stale timers for it are ignored.
    last_seq: u32,
    /// Retries already spent on a clipboard another app is holding.
    retries: usize,
    /// Foreground window at the last copy: the source when the clipboard has no owner.
    foreground: HWND,
    caret: Option<CaretFinder>,
    /// The paste-stack keyboard hook, while a stack is active.
    hook: Option<HHOOK>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Starts the listener and worker threads; `None` only if the window or threads couldn't be
/// created. A failed listener or a taken hotkey is reported as `OsEvent::Status` instead,
/// since pasting from history still works without them.
pub fn start_with(events: UnboundedSender<OsEvent>, hotkey: Hotkey) -> Option<Clipboard> {
    let format = |name: PCWSTR| unsafe { RegisterClipboardFormatW(name) };
    let formats = Formats {
        exclude: format(w!("ExcludeClipboardContentFromMonitorProcessing")),
        viewer_ignore: format(w!("Clipboard Viewer Ignore")),
        history: format(w!("CanIncludeInClipboardHistory")),
        png: format(w!("PNG")),
        html: format(w!("HTML Format")),
        rtf: format(w!("Rich Text Format")),
        drop_effect: format(w!("Preferred DropEffect")),
    };
    let (job_tx, job_rx) = std_mpsc::channel();
    let (write_tx, write_rx) = std_mpsc::channel();
    let (ready_tx, ready_rx) = std_mpsc::channel();
    let window_events = events.clone();
    let window_jobs = job_tx.clone();
    std::thread::Builder::new()
        .name("clipboard".into())
        .spawn(move || {
            // Caret rectangles in physical pixels, whatever the monitor's scale.
            unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
            let hwnd = match unsafe { create_window() } {
                Ok(hwnd) => hwnd,
                Err(e) => {
                    eprintln!("clipboard: can't create the listener window: {e}");
                    _ = ready_tx.send(None);
                    return;
                }
            };
            unsafe {
                if let Err(e) = AddClipboardFormatListener(hwnd) {
                    eprintln!("clipboard: can't listen for copies: {e}");
                    _ = window_events.send(OsEvent::Status(OsStatus::ListenerFailed));
                }
                if let Err(e) = RegisterHotKey(Some(hwnd), HOTKEY_ID, HOT_KEY_MODIFIERS(hotkey.modifiers) | MOD_NOREPEAT, hotkey.vk) {
                    eprintln!("clipboard: {} is taken by another app ({e}); history opens from the island only", hotkey.label);
                    _ = window_events.send(OsEvent::Status(OsStatus::HotkeyTaken));
                }
                for n in 1..=9u8 {
                    if let Err(e) = RegisterHotKey(Some(hwnd), RECENT_ID + i32::from(n), MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, u32::from(b'0' + n)) {
                        eprintln!("clipboard: Ctrl+Alt+{n} is taken by another app ({e}); recent paste {n} is off");
                    }
                }
            }
            STATE.with(|s| {
                *s.borrow_mut() = Some(State {
                    hwnd,
                    events: window_events.clone(),
                    jobs: window_jobs,
                    writes: write_rx,
                    formats,
                    own_write: 0,
                    last_seq: 0,
                    retries: 0,
                    foreground: HWND::default(),
                    caret: CaretFinder::start(),
                    hook: None,
                })
            });
            _ = ready_tx.send(Some(hwnd.0 as isize));
            let mut msg = MSG::default();
            unsafe {
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    DispatchMessageW(&msg);
                }
            }
        })
        .ok()?;
    let hwnd = ready_rx.recv().ok()??;
    std::thread::Builder::new()
        .name("clipboard-worker".into())
        .spawn(move || Worker::new(events, write_tx, hwnd, formats).run(job_rx))
        .ok()?;
    Some(Clipboard { jobs: job_tx, hwnd })
}

unsafe fn create_window() -> Result<HWND> {
    let instance = GetModuleHandleW(None)?;
    let class = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: instance.into(), lpszClassName: w!("ReUI.Clipboard"), ..Default::default() };
    RegisterClassW(&class);
    CreateWindowExW(WINDOW_EX_STYLE(0), w!("ReUI.Clipboard"), w!("ReUI clipboard"), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, Some(instance.into()), None)
}

/// Runs `f` on the window state. `try_borrow_mut`: clipboard calls can re-enter the wndproc
/// (EmptyClipboard sends WM_DESTROYCLIPBOARD to the previous owner, which may be us).
fn with_state(f: impl FnOnce(&mut State)) {
    STATE.with(|s| {
        if let Ok(mut s) = s.try_borrow_mut() {
            if let Some(state) = s.as_mut() {
                f(state);
            }
        }
    });
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_CLIPBOARDUPDATE => {
            let mut settle = true;
            with_state(|state| {
                // Our own write needs no read (and no timer).
                settle = GetClipboardSequenceNumber() != state.own_write;
                state.retries = 0;
                state.foreground = GetForegroundWindow();
            });
            if settle {
                // A fresh copy supersedes a pending retry; restarting the timer coalesces bursts.
                _ = KillTimer(Some(hwnd), RETRY_TIMER);
                SetTimer(Some(hwnd), SETTLE_TIMER, SETTLE_MS, None);
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == SETTLE_TIMER || wparam.0 == RETRY_TIMER => {
            _ = KillTimer(Some(hwnd), wparam.0);
            with_state(|state| capture(state));
            LRESULT(0)
        }
        WM_HOTKEY if wparam.0 as i32 == HOTKEY_ID => {
            with_state(|state| {
                // First, before anything can move focus: where the user was typing.
                let target = capture_target(state.caret.as_ref());
                // We got the hotkey, so we may pass the foreground on: the window (in the
                // island process) takes focus to search, then gives it back to the target.
                _ = AllowSetForegroundWindow(ASFW_ANY);
                _ = state.events.send(OsEvent::Hotkey(target));
            });
            LRESULT(0)
        }
        WM_HOTKEY if (RECENT_ID + 1..=RECENT_ID + 9).contains(&(wparam.0 as i32)) => {
            let n = (wparam.0 as i32 - RECENT_ID) as u32;
            let target = GetForegroundWindow().0 as isize;
            with_state(|state| _ = state.events.send(OsEvent::RecentPaste { n, target }));
            LRESULT(0)
        }
        WM_STACK => {
            with_state(|state| match (wparam.0 != 0, state.hook) {
                (true, None) => match SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), GetModuleHandleW(None).ok().map(Into::into), 0) {
                    Ok(hook) => state.hook = Some(hook),
                    Err(e) => eprintln!("clipboard: can't watch for pastes ({e}); the stack won't advance"),
                },
                (false, Some(hook)) => {
                    _ = UnhookWindowsHookEx(hook);
                    _ = KillTimer(Some(hwnd), STACK_TIMER);
                    state.hook = None;
                }
                _ => {}
            });
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == STACK_TIMER => {
            _ = KillTimer(Some(hwnd), STACK_TIMER);
            with_state(|state| {
                if state.hook.is_some() {
                    _ = state.events.send(OsEvent::StackPasted);
                }
            });
            LRESULT(0)
        }
        WM_WRITE_READY => {
            with_state(|state| {
                while let Ok(job) = state.writes.try_recv() {
                    let result = write(state.hwnd, &job.data);
                    if result.is_ok() {
                        state.own_write = GetClipboardSequenceNumber();
                        state.last_seq = state.own_write;
                    }
                    _ = job.reply.send(result.map_err(|e| e.to_string()));
                }
            });
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// The paste stack's keyboard hook: the user's own paste chord in another app (not ReUI's
/// window, not our injected keys) advances the stack once it has had time to land; Esc ends
/// it. Runs on the window thread, so it must stay quick.
unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && lparam.0 != 0 {
        let info = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        let message = wparam.0 as u32;
        if info.flags.0 & LLKHF_INJECTED.0 == 0 {
            let held = |vk: VIRTUAL_KEY| GetAsyncKeyState(i32::from(vk.0)) < 0;
            let up = message == WM_KEYUP || message == WM_SYSKEYUP;
            let chord = (info.vkCode == u32::from(b'V') && held(VK_CONTROL)) || (info.vkCode == u32::from(VK_INSERT.0) && held(VK_SHIFT));
            if up && chord && !is_reui(GetForegroundWindow()) {
                with_state(|state| _ = SetTimer(Some(state.hwnd), STACK_TIMER, STACK_SETTLE_MS, None));
            } else if (message == WM_KEYDOWN || message == WM_SYSKEYDOWN) && info.vkCode == u32::from(VK_ESCAPE.0) {
                with_state(|state| _ = state.events.send(OsEvent::StackCancel));
            }
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

/// One of ours: this process or another `reui-*` executable (the island and its window).
unsafe fn is_reui(window: HWND) -> bool {
    let mut pid = 0u32;
    GetWindowThreadProcessId(window, Some(&mut pid));
    pid == GetCurrentProcessId() || exe_path(window).is_some_and(|p| file_name_lower(&p).starts_with("reui-"))
}

/// The foreground window, its focused control, and (with a finder) the caret: UI
/// Automation or MSAA within [`CARET_WAIT`], else the classic system caret.
unsafe fn capture_target(finder: Option<&CaretFinder>) -> Target {
    let foreground = GetForegroundWindow();
    let thread = GetWindowThreadProcessId(foreground, None);
    let mut info = GUITHREADINFO { cbSize: size_of::<GUITHREADINFO>() as u32, ..Default::default() };
    let gui = (thread != 0 && GetGUIThreadInfo(thread, &mut info).is_ok()).then_some(info);
    let focus = gui.map(|g| g.hwndFocus).filter(|h| !h.is_invalid()).unwrap_or(foreground);
    // Some providers report an empty rectangle at the origin for "no caret".
    let caret = finder.and_then(|finder| finder.find(focus)).filter(|c| *c != [0, 0, 0, 0]);
    let caret = caret.or_else(|| {
        let g = gui.filter(|g| finder.is_some() && !g.hwndCaret.is_invalid() && g.rcCaret.bottom > g.rcCaret.top)?;
        let mut corners = [POINT { x: g.rcCaret.left, y: g.rcCaret.top }, POINT { x: g.rcCaret.right, y: g.rcCaret.bottom }];
        for corner in &mut corners {
            if !ClientToScreen(g.hwndCaret, corner).as_bool() {
                return None;
            }
        }
        Some([corners[0].x, corners[0].y, corners[1].x.max(corners[0].x + 1), corners[1].y])
    });
    Target { hwnd: foreground.0 as isize, focus: focus.0 as isize, caret }
}

type CaretRequest = (isize, std_mpsc::Sender<Option<[i32; 4]>>);

/// Asks UI Automation, then MSAA, where the text caret is, on its own thread: both talk to
/// the target app, and a hung app must cost the hotkey at most [`CARET_WAIT`]. A request
/// still stuck when the next one comes simply makes that one time out too.
struct CaretFinder {
    requests: std_mpsc::Sender<CaretRequest>,
}

impl CaretFinder {
    fn start() -> Option<CaretFinder> {
        let (requests, queue) = std_mpsc::channel::<CaretRequest>();
        std::thread::Builder::new()
            .name("clipboard-caret".into())
            .spawn(move || unsafe {
                SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
                _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                let uia: Option<IUIAutomation> = CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER).ok();
                if let Some(uia) = uia.as_ref().and_then(|u| u.cast::<IUIAutomation2>().ok()) {
                    _ = uia.SetConnectionTimeout(UIA_TIMEOUT_MS);
                    _ = uia.SetTransactionTimeout(UIA_TIMEOUT_MS);
                }
                for (focus, reply) in queue {
                    let caret = uia.as_ref().and_then(|uia| uia_caret(uia)).or_else(|| msaa_caret(HWND(focus as *mut c_void)));
                    _ = reply.send(caret);
                }
            })
            .ok()?;
        Some(CaretFinder { requests })
    }

    fn find(&self, focus: HWND) -> Option<[i32; 4]> {
        let (reply, answer) = std_mpsc::channel();
        self.requests.send((focus.0 as isize, reply)).ok()?;
        answer.recv_timeout(CARET_WAIT).ok().flatten()
    }
}

/// The focused element's caret range (TextPattern2). An empty range often has no
/// rectangle, so then the character after it is measured and its left edge used.
unsafe fn uia_caret(uia: &IUIAutomation) -> Option<[i32; 4]> {
    let element = uia.GetFocusedElement().ok()?;
    let pattern: IUIAutomationTextPattern2 = element.GetCurrentPatternAs(UIA_TextPattern2Id).ok()?;
    let mut active = windows::core::BOOL::default();
    let range = pattern.GetCaretRange(&mut active).ok()?;
    let rect = match range_rect(&range) {
        Some(rect) => rect,
        None => {
            let around = range.Clone().ok()?;
            around.ExpandToEnclosingUnit(TextUnit_Character).ok()?;
            range_rect(&around)?
        }
    };
    Some([rect[0], rect[1], rect[0] + 1, rect[3]])
}

/// The first bounding rectangle of a text range, as left, top, right, bottom.
unsafe fn range_rect(range: &IUIAutomationTextRange) -> Option<[i32; 4]> {
    let array = range.GetBoundingRectangles().ok()?;
    if array.is_null() {
        return None;
    }
    let (low, high) = (SafeArrayGetLBound(array, 1).unwrap_or(0), SafeArrayGetUBound(array, 1).unwrap_or(-1));
    let mut data: *mut c_void = std::ptr::null_mut();
    let mut rect = None;
    if high - low + 1 >= 4 && SafeArrayAccessData(array, &mut data).is_ok() {
        // Doubles: left, top, width, height, then the next rectangle.
        let v = std::slice::from_raw_parts(data as *const f64, 4);
        if v[3] > 0.0 {
            rect = Some([v[0].round() as i32, v[1].round() as i32, (v[0] + v[2]).round() as i32, (v[1] + v[3]).round() as i32]);
        }
        _ = SafeArrayUnaccessData(array);
    }
    _ = SafeArrayDestroy(array);
    rect
}

/// The MSAA caret object of the focused window: apps that draw their own caret but report
/// it to accessibility tools, and the classic caret's proxy.
unsafe fn msaa_caret(focus: HWND) -> Option<[i32; 4]> {
    if focus.is_invalid() {
        return None;
    }
    let mut raw: *mut c_void = std::ptr::null_mut();
    AccessibleObjectFromWindow(focus, OBJID_CARET.0 as u32, &IAccessible::IID, &mut raw).ok()?;
    if raw.is_null() {
        return None;
    }
    let accessible = IAccessible::from_raw(raw);
    let mut child = VARIANT::default();
    (*child.Anonymous.Anonymous).vt = VT_I4; // CHILDID_SELF (0)
    let (mut l, mut t, mut w, mut h) = (0, 0, 0, 0);
    accessible.accLocation(&mut l, &mut t, &mut w, &mut h, &child).ok()?;
    (h > 0 && (l, t) != (0, 0)).then_some([l, t, l + w.max(1), t + h])
}

/// Reads the current clipboard once and hands it to the worker. If another app still holds
/// it, a timer tries again (100 ms, then 250 ms) instead of sleeping on the message thread;
/// by then the content may have changed, which is fine: we read whatever is current.
unsafe fn capture(state: &mut State) {
    let seq = GetClipboardSequenceNumber();
    if seq == state.own_write || seq == state.last_seq {
        return;
    }
    match read(state.hwnd, state.formats) {
        Read::Busy => match RETRY_DELAYS_MS.get(state.retries) {
            Some(&delay) => {
                state.retries += 1;
                SetTimer(Some(state.hwnd), RETRY_TIMER, delay, None);
            }
            None => {
                state.retries = 0;
                eprintln!("clipboard: another app kept the clipboard open; copy {seq} not recorded");
            }
        },
        Read::Skip => state.last_seq = seq,
        Read::Got(raw, owner) => {
            state.last_seq = seq;
            // No owner (some apps write without owning the clipboard) or one we can't
            // query: credit the app that was in front when the copy happened.
            let owner = owner.and_then(|o| exe_path(o)).or_else(|| exe_path(state.foreground));
            _ = state.jobs.send(Job::Copied { raw, owner });
        }
    }
}

/// Closes the clipboard when dropped.
struct Open;

impl Open {
    /// Another app may still hold the clipboard right after it changes. A couple of very
    /// short tries only: the caller retries later rather than blocking the message loop.
    unsafe fn new(hwnd: HWND) -> Option<Open> {
        for attempt in 0..3 {
            if attempt > 0 {
                std::thread::sleep(Duration::from_millis(5));
            }
            if OpenClipboard(Some(hwnd)).is_ok() {
                return Some(Open);
            }
        }
        None
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        unsafe { _ = CloseClipboard() };
    }
}

enum Read {
    /// Another app has the clipboard open.
    Busy,
    /// Nothing we keep, too large, or excluded.
    Skip,
    /// Raw content and the clipboard owner's window.
    Got(Raw, Option<HWND>),
}

/// Copies the raw bytes of the formats we keep and closes the clipboard right away;
/// decoding happens on the worker.
unsafe fn read(hwnd: HWND, formats: Formats) -> Read {
    let Some(open) = Open::new(hwnd) else { return Read::Busy };
    if has(formats.exclude) || has(formats.viewer_ignore) {
        return Read::Skip;
    }
    if has(formats.history) {
        // `CanIncludeInClipboardHistory` = 0 means private; unreadable counts as private too.
        let allowed = with_global(formats.history, |b| b.len() >= 4 && u32::from_le_bytes([b[0], b[1], b[2], b[3]]) != 0);
        if allowed != Some(true) {
            return Read::Skip;
        }
    }
    let owner = GetClipboardOwner().ok().filter(|h| !h.is_invalid());
    let raw = read_content(formats);
    drop(open);
    match raw {
        Some(raw) => Read::Got(raw, owner),
        None => Read::Skip,
    }
}

/// Whether a format is offered (doesn't make a delayed-render owner render it).
unsafe fn has(format: u32) -> bool {
    format != 0 && IsClipboardFormatAvailable(format).is_ok()
}

/// Files beat text (Explorer offers both), text beats pictures (Office offers both), but
/// blank text falls through to the picture (Office shapes carry an empty CF_UNICODETEXT).
/// Only the formats we keep are asked for: each `GetClipboardData` can force a delayed render.
unsafe fn read_content(formats: Formats) -> Option<Raw> {
    if has(u32::from(CF_HDROP.0)) {
        if let Some(paths) = read_files() {
            let effect = if has(formats.drop_effect) { with_global(formats.drop_effect, |b| Some(u32::from_le_bytes(b.get(..4)?.try_into().ok()?))).flatten() } else { None };
            return Some(Raw::Files { paths, cut: effect.is_some_and(is_move) });
        }
    }
    if has(u32::from(CF_UNICODETEXT.0)) {
        match read_capped(u32::from(CF_UNICODETEXT.0), 2) {
            Capped::TooLarge => {
                eprintln!("clipboard: copied text is too large (over {} MB); not recorded", MAX_FORMAT_BYTES >> 20);
                return None;
            }
            Capped::Bytes(utf16) if !is_blank_utf16(&utf16) => {
                let rich = |format: u32, name: &str| match if has(format) { read_capped(format, 1) } else { Capped::Missing } {
                    Capped::Bytes(bytes) if !bytes.is_empty() => Some(bytes),
                    Capped::TooLarge => {
                        eprintln!("clipboard: copied {name} is too large; kept as plain text");
                        None
                    }
                    _ => None,
                };
                return Some(Raw::Text { utf16, html: rich(formats.html, "HTML"), rtf: rich(formats.rtf, "RTF") });
            }
            _ => {}
        }
    }
    if has(formats.png) {
        if let Some(png) = read_png(formats.png) {
            return Some(Raw::Png(png));
        }
    }
    // Windows synthesises CF_DIBV5 from CF_DIB (and back), so CF_DIB is the rare fallback.
    for format in [CF_DIBV5, CF_DIB] {
        if has(u32::from(format.0)) {
            if let Some(dib) = read_dib(u32::from(format.0)) {
                return Some(Raw::Dib(dib));
            }
        }
    }
    None
}

/// "Preferred DropEffect" says move (cut in Explorer) rather than copy.
fn is_move(effect: u32) -> bool {
    const DROPEFFECT_COPY: u32 = 1;
    const DROPEFFECT_MOVE: u32 = 2;
    effect & DROPEFFECT_MOVE != 0 && effect & DROPEFFECT_COPY == 0
}

/// Runs `f` on the bytes of a clipboard format while it's locked (the clipboard must be open).
unsafe fn with_global<T>(format: u32, f: impl FnOnce(&[u8]) -> T) -> Option<T> {
    let handle = GetClipboardData(format).ok()?;
    let global = HGLOBAL(handle.0);
    let ptr = GlobalLock(global) as *const u8;
    if ptr.is_null() {
        return None;
    }
    let result = f(std::slice::from_raw_parts(ptr, GlobalSize(global)));
    _ = GlobalUnlock(global);
    Some(result)
}

enum Capped {
    Missing,
    TooLarge,
    Bytes(Vec<u8>),
}

/// A NUL-terminated clipboard string (`unit`: 1 byte, or 2 for UTF-16) without its terminator
/// and the allocation's padding; refused past the byte cap rather than cut short.
unsafe fn read_capped(format: u32, unit: usize) -> Capped {
    with_global(format, |bytes| {
        let bytes = until_nul(bytes, unit);
        if bytes.len() > MAX_FORMAT_BYTES {
            Capped::TooLarge
        } else {
            Capped::Bytes(bytes.to_vec())
        }
    })
    .unwrap_or(Capped::Missing)
}

fn until_nul(bytes: &[u8], unit: usize) -> &[u8] {
    let whole = bytes.len() - bytes.len() % unit;
    let end = bytes.chunks_exact(unit).position(|c| c.iter().all(|&b| b == 0)).map_or(whole, |i| i * unit);
    &bytes[..end]
}

fn is_blank_utf16(bytes: &[u8]) -> bool {
    bytes.chunks_exact(2).all(|c| char::from_u32(u32::from(u16::from_le_bytes([c[0], c[1]]))).is_some_and(char::is_whitespace))
}

/// CF_HTML bytes → the payload as received (header, offsets and all). The offsets count
/// UTF-8 bytes, so it's kept verbatim rather than re-encoded or trimmed to the fragment.
fn html_from_clipboard(bytes: &[u8]) -> Option<String> {
    let bytes = until_nul(bytes, 1);
    (!bytes.is_empty()).then(|| String::from_utf8_lossy(bytes).into_owned())
}

fn html_to_clipboard(html: &str) -> Vec<u8> {
    html.bytes().chain([0]).collect()
}

unsafe fn read_files() -> Option<Vec<String>> {
    let handle = GetClipboardData(u32::from(CF_HDROP.0)).ok()?;
    let drop = HDROP(handle.0);
    let count = DragQueryFileW(drop, u32::MAX, None);
    let files: Vec<String> = (0..count)
        .filter_map(|i| {
            let len = DragQueryFileW(drop, i, None) as usize;
            let mut buffer = vec![0u16; len + 1];
            (DragQueryFileW(drop, i, Some(&mut buffer)) > 0).then(|| String::from_utf16_lossy(&buffer[..len]))
        })
        .collect();
    (!files.is_empty()).then_some(files)
}

fn within_cap(w: u64, h: u64) -> bool {
    let ok = w * h <= MAX_IMAGE_PIXELS;
    if !ok {
        eprintln!("clipboard: skipped a {w}×{h} image (over {} MP)", MAX_IMAGE_PIXELS / 1_000_000);
    }
    ok
}

/// The registered "PNG" bytes (alpha intact), up to the end of the file; `None` if it isn't
/// a PNG or is over the size cap (checked from the header, before copying).
unsafe fn read_png(format: u32) -> Option<Vec<u8>> {
    with_global(format, |png| {
        let (w, h) = png_size(png)?;
        within_cap(u64::from(w), u64::from(h)).then(|| png[..png_end(png).unwrap_or(png.len())].to_vec())
    })
    .flatten()
}

/// The DIB bytes, unless the header says the image is over the size cap (checked before
/// copying, so a huge image costs nothing).
unsafe fn read_dib(format: u32) -> Option<Vec<u8>> {
    with_global(format, |dib| {
        let (w, h) = dib_size(dib)?;
        within_cap(w, h).then(|| dib.to_vec())
    })
    .flatten()
}

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    if png.get(..8)? != PNG_SIGNATURE || png.get(12..16)? != b"IHDR" {
        return None;
    }
    let be = |at: usize| png.get(at..at + 4).map(|b| u32::from_be_bytes(b.try_into().unwrap()));
    Some((be(16)?, be(20)?))
}

/// PNG chunks (type, offset of the chunk) up to and including IEND.
fn png_chunks(png: &[u8]) -> impl Iterator<Item = (&[u8], usize)> {
    let mut at = PNG_SIGNATURE.len();
    let mut done = false;
    std::iter::from_fn(move || {
        if done {
            return None;
        }
        let head = png.get(at..at + 8)?;
        let len = u32::from_be_bytes(head[0..4].try_into().unwrap()) as usize;
        let chunk = (&head[4..8], at);
        done = chunk.0 == b"IEND";
        at = at.saturating_add(12).saturating_add(len);
        Some(chunk)
    })
}

/// Length of the PNG file in a (padded) allocation.
fn png_end(png: &[u8]) -> Option<usize> {
    png_chunks(png).find(|(kind, _)| *kind == b"IEND").map(|(_, at)| at + 12).filter(|&end| end <= png.len())
}

/// Grey+alpha or RGBA, or a tRNS chunk (palette or colour-key transparency).
fn png_has_alpha(png: &[u8]) -> bool {
    if matches!(png.get(25), Some(4 | 6)) {
        return true;
    }
    png_chunks(png).take_while(|(kind, _)| *kind != b"IDAT").any(|(kind, _)| kind == b"tRNS")
}

/// Width and height from a DIB header (BITMAPCOREHEADER or BITMAPINFOHEADER and later).
fn dib_size(dib: &[u8]) -> Option<(u64, u64)> {
    let header_size = u32::from_le_bytes(dib.get(0..4)?.try_into().ok()?);
    if header_size == 12 {
        let w = u16::from_le_bytes(dib.get(4..6)?.try_into().ok()?);
        let h = u16::from_le_bytes(dib.get(6..8)?.try_into().ok()?);
        return Some((u64::from(w), u64::from(h)));
    }
    if header_size < 40 {
        return None;
    }
    let w = i32::from_le_bytes(dib.get(4..8)?.try_into().ok()?);
    let h = i32::from_le_bytes(dib.get(8..12)?.try_into().ok()?);
    Some((u64::from(w.unsigned_abs()), u64::from(h.unsigned_abs())))
}

/// A 32-bit DIB's pixels as top-down straight BGRA.
#[derive(Debug, PartialEq)]
struct Pixels {
    w: u32,
    h: u32,
    bgra: Vec<u8>,
    /// Some pixel had a non-zero alpha. Most apps leave the fourth byte at 0 ("reserved"),
    /// so an all-zero alpha means opaque, and `bgra` then carries 255 there.
    alpha: bool,
}

/// Decodes a 32 bpp DIB (BI_RGB, BI_BITFIELDS or BI_ALPHABITFIELDS, any header version);
/// `None` for other bit depths, which go through WIC's BMP decoder instead (no alpha).
fn dib_pixels(dib: &[u8]) -> Option<Pixels> {
    let u32_at = |at: usize| dib.get(at..at + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    let header = u32_at(0)? as usize;
    if header < 40 || u16::from_le_bytes(dib.get(14..16)?.try_into().ok()?) != 32 {
        return None;
    }
    let (w, h) = (u32_at(4)? as i32, u32_at(8)? as i32);
    let (top_down, w, h) = (h < 0, w.unsigned_abs(), h.unsigned_abs());
    if w == 0 || h == 0 {
        return None;
    }
    // Masks live in the header from V2 (52 bytes) on, else right after a 40-byte header.
    let (mut masks, extra) = match u32_at(16)? {
        0 => ([0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000], 0),
        3 | 6 if header >= 52 => ([u32_at(40)?, u32_at(44)?, u32_at(48)?, if header >= 56 { u32_at(52)? } else { 0 }], 0),
        3 => ([u32_at(40)?, u32_at(44)?, u32_at(48)?, 0], 12),
        6 => ([u32_at(40)?, u32_at(44)?, u32_at(48)?, u32_at(52)?], 16),
        _ => return None,
    };
    if masks[3] == 0 {
        // No alpha mask given: the bits the colours don't use (the "reserved" byte).
        masks[3] = !(masks[0] | masks[1] | masks[2]);
    }
    let offset = header + extra + u32_at(32)? as usize * 4;
    let stride = w as usize * 4;
    let pixels = dib.get(offset..offset.checked_add(stride.checked_mul(h as usize)?)?)?;
    let channel = |px: u32, mask: u32| -> u8 {
        if mask == 0 {
            return 0;
        }
        let shift = mask.trailing_zeros();
        let (value, max) = (u64::from((px & mask) >> shift), u64::from(mask >> shift));
        ((value * 255 + max / 2) / max) as u8
    };
    let mut bgra = Vec::with_capacity(stride * h as usize);
    let mut alpha = false;
    for row in 0..h as usize {
        let src = if top_down { row } else { h as usize - 1 - row };
        for px in pixels[src * stride..(src + 1) * stride].chunks_exact(4) {
            let px = u32::from_le_bytes(px.try_into().unwrap());
            let a = channel(px, masks[3]);
            alpha |= a != 0;
            bgra.extend_from_slice(&[channel(px, masks[2]), channel(px, masks[1]), channel(px, masks[0]), a]);
        }
    }
    if !alpha {
        bgra.chunks_exact_mut(4).for_each(|px| px[3] = 255);
    }
    Some(Pixels { w, h, bgra, alpha })
}

/// Top-down straight BGRA → a bottom-up 32 bpp DIB. `v5`: CF_DIBV5 with an alpha mask (sRGB);
/// else CF_DIB, flattened onto white with the reserved byte 0, for apps that ignore alpha.
fn dib_from_bgra(w: u32, h: u32, bgra: &[u8], v5: bool) -> Vec<u8> {
    let stride = w as usize * 4;
    let mut dib = Vec::with_capacity(124 + bgra.len());
    let put = |dib: &mut Vec<u8>, values: &[u32]| values.iter().for_each(|v| dib.extend_from_slice(&v.to_le_bytes()));
    // Size, width, height (positive: bottom-up), planes 1 | bit count 32, compression, image
    // size, pixels per metre ×2, colours used, colours important.
    put(&mut dib, &[if v5 { 124 } else { 40 }, w, h, 1 | (32 << 16), if v5 { 3 } else { 0 }, (stride * h as usize) as u32, 0, 0, 0, 0]);
    if v5 {
        // RGBA masks, LCS_sRGB, 36 bytes of endpoints + 3 gammas, LCS_GM_IMAGES, profile ×2, reserved.
        put(&mut dib, &[0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000, 0x7352_4742]);
        put(&mut dib, &[0; 12]);
        put(&mut dib, &[4, 0, 0, 0]);
    }
    for row in bgra.chunks_exact(stride).rev() {
        if v5 {
            dib.extend_from_slice(row);
        } else {
            for px in row.chunks_exact(4) {
                let a = u32::from(px[3]);
                let over_white = |c: u8| ((u32::from(c) * a + 255 * (255 - a) + 127) / 255) as u8;
                dib.extend_from_slice(&[over_white(px[0]), over_white(px[1]), over_white(px[2]), 0]);
            }
        }
    }
    dib
}

/// Path of the process that owns `window`.
unsafe fn exe_path(window: HWND) -> Option<String> {
    if window.is_invalid() {
        return None;
    }
    let mut pid = 0u32;
    GetWindowThreadProcessId(window, Some(&mut pid));
    if pid == 0 {
        return None;
    }
    let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
    let mut buffer = [0u16; 512];
    let mut len = buffer.len() as u32;
    let named = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &mut len);
    _ = CloseHandle(process);
    named.ok()?;
    Some(String::from_utf16_lossy(&buffer[..len as usize]))
}

fn file_name_lower(path: &str) -> String {
    path.rsplit(['\\', '/']).next().unwrap_or(path).to_lowercase()
}

/// Sets every prepared format in one short open/empty/set/close.
unsafe fn write(hwnd: HWND, data: &[(u32, Vec<u8>)]) -> Result<()> {
    let _open = Open::new(hwnd).ok_or_else(windows::core::Error::empty)?;
    EmptyClipboard()?;
    for (format, bytes) in data {
        set_global(*format, bytes)?;
    }
    Ok(())
}

unsafe fn set_global(format: u32, bytes: &[u8]) -> Result<()> {
    let global = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1))?;
    let ptr = GlobalLock(global) as *mut u8;
    if ptr.is_null() {
        _ = GlobalFree(Some(global));
        return Err(windows::core::Error::from_thread());
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
    _ = GlobalUnlock(global);
    // On success the clipboard owns the memory; on failure it's still ours.
    if let Err(e) = SetClipboardData(format, Some(HANDLE(global.0))) {
        _ = GlobalFree(Some(global));
        return Err(e);
    }
    Ok(())
}

/// The slow half: image codecs, icons, paste prep and key input, off the window thread.
struct Worker {
    events: UnboundedSender<OsEvent>,
    writes: std_mpsc::Sender<Write>,
    hwnd: isize,
    formats: Formats,
    wic: Option<IWICImagingFactory>,
    /// App icons by executable path (icons don't change while we run).
    icons: HashMap<String, Option<Vec<u8>>>,
}

impl Worker {
    fn new(events: UnboundedSender<OsEvent>, writes: std_mpsc::Sender<Write>, hwnd: isize, formats: Formats) -> Worker {
        let wic = unsafe {
            _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()
        };
        Worker { events, writes, hwnd, formats, wic, icons: HashMap::new() }
    }

    fn run(mut self, jobs: std_mpsc::Receiver<Job>) {
        // A panic on one odd copy (a malformed image, say) must not end capture: this thread
        // carries every copy and paste, and losing it would silently stop both.
        while let Ok(job) = jobs.recv() {
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.handle(job))).is_err() {
                eprintln!("clipboard: handling a copy or paste failed unexpectedly; skipped it");
            }
        }
    }

    fn handle(&mut self, job: Job) {
        {
            match job {
                Job::Copied { raw, owner } => {
                    if let Some(captured) = self.captured(raw, owner) {
                        _ = self.events.send(OsEvent::Copied(captured));
                    }
                }
                Job::Write { content, plain, target } => match self.put(&content, plain) {
                    Ok(()) => {
                        if let Some(target) = target {
                            self.type_paste(HWND(target as *mut c_void));
                        }
                    }
                    Err(e) => eprintln!("clipboard: can't put the item back: {e}"),
                },
            }
        }
    }

    fn captured(&mut self, raw: Raw, owner: Option<String>) -> Option<Captured> {
        let mut size = None;
        let content = match raw {
            Raw::Text { utf16, html, rtf } => {
                let units: Vec<u16> = utf16.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                Content::Text { text: String::from_utf16_lossy(&units), html: html.as_deref().and_then(html_from_clipboard), rtf }
            }
            Raw::Files { paths, cut } => Content::Files { paths, cut },
            Raw::Png(_) | Raw::Dib(_) => {
                let wic = self.wic.as_ref()?;
                let image = match raw {
                    Raw::Png(png) => unsafe { png_image(wic, png) },
                    Raw::Dib(dib) => unsafe { dib_image(wic, &dib) },
                    _ => unreachable!(),
                };
                match image {
                    Ok((content, w, h)) => {
                        size = Some((w, h));
                        content
                    }
                    Err(e) => {
                        eprintln!("clipboard: can't read the copied image: {e}");
                        return None;
                    }
                }
            }
        };
        let icon = owner.as_deref().and_then(|exe| {
            let wic = self.wic.clone()?;
            self.icons.entry(exe.to_string()).or_insert_with(|| app_icon(&wic, exe)).clone()
        });
        let exe = owner.as_deref().map(file_name_lower);
        Some(Captured { content, source: owner.as_deref().map(app_name), exe, icon, size })
    }

    /// Decodes and builds every format first, then has the window thread set them; a busy
    /// clipboard gets a few retries here, where sleeping blocks nothing.
    fn put(&self, content: &Content, plain: bool) -> std::result::Result<(), String> {
        let data = Arc::new(self.prepare(content, plain).map_err(|e| e.to_string())?);
        let mut last = String::new();
        for attempt in 0..3 {
            if attempt > 0 {
                std::thread::sleep(Duration::from_millis(50));
            }
            let (reply, result) = std_mpsc::channel();
            self.writes.send(Write { data: data.clone(), reply }).map_err(|_| "the clipboard thread is gone")?;
            unsafe { PostMessageW(Some(HWND(self.hwnd as *mut c_void)), WM_WRITE_READY, WPARAM(0), LPARAM(0)) }.map_err(|e| e.to_string())?;
            match result.recv_timeout(Duration::from_secs(2)) {
                Ok(Ok(())) => return Ok(()),
                Ok(Err(e)) => last = e,
                Err(_) => return Err("the clipboard thread didn't answer".into()),
            }
        }
        Err(format!("the clipboard stayed busy ({last})"))
    }

    /// Every format, most descriptive first (apps take the first one they understand).
    fn prepare(&self, content: &Content, plain: bool) -> Result<Vec<(u32, Vec<u8>)>> {
        let mut data = Vec::new();
        let mut push = |format: u32, bytes: Vec<u8>| {
            if format != 0 {
                data.push((format, bytes));
            }
        };
        match content {
            Content::Text { text, html, rtf } => {
                if !plain {
                    if let Some(html) = html {
                        push(self.formats.html, html_to_clipboard(html));
                    }
                    if let Some(rtf) = rtf {
                        push(self.formats.rtf, rtf.iter().copied().chain([0]).collect());
                    }
                }
                let units: Vec<u16> = text.encode_utf16().chain([0]).collect();
                push(u32::from(CF_UNICODETEXT.0), bytemuck_u16(&units).to_vec());
            }
            Content::Files { paths, .. } => {
                let header = DROPFILES { pFiles: size_of::<DROPFILES>() as u32, pt: POINT::default(), fNC: false.into(), fWide: true.into() };
                let mut bytes = unsafe { std::slice::from_raw_parts(&header as *const DROPFILES as *const u8, size_of::<DROPFILES>()) }.to_vec();
                for file in paths {
                    let units: Vec<u16> = file.encode_utf16().chain([0]).collect();
                    bytes.extend_from_slice(bytemuck_u16(&units));
                }
                bytes.extend_from_slice(&[0, 0]);
                push(u32::from(CF_HDROP.0), bytes);
                // Always a copy: replaying a cut would move the files out from under the history.
                push(self.formats.drop_effect, 1u32.to_le_bytes().to_vec());
            }
            Content::Image { png, .. } => {
                let wic = self.wic.as_ref().ok_or_else(windows::core::Error::empty)?;
                let (w, h, bgra) = unsafe { png_to_bgra(wic, png)? };
                push(self.formats.png, png.clone());
                push(u32::from(CF_DIBV5.0), dib_from_bgra(w, h, &bgra, true));
                push(u32::from(CF_DIB.0), dib_from_bgra(w, h, &bgra, false));
            }
        }
        // Keep the paste-back out of Win+V history and other clipboard managers (and us).
        push(self.formats.history, 0u32.to_le_bytes().to_vec());
        push(self.formats.exclude, vec![0]);
        Ok(data)
    }

    /// Types the paste chord into `target` once it's in front and the user has let go of the
    /// modifiers (up to 500 ms; a still-held Shift/Alt/Win from the hotkey would turn Ctrl+V
    /// into a different shortcut). Reports `PasteBlocked` instead if focus is elsewhere, or
    /// the target runs at a higher integrity level (Windows would drop our input).
    fn type_paste(&self, target: HWND) {
        std::thread::sleep(Duration::from_millis(40));
        let held = || [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN].iter().any(|vk| unsafe { GetAsyncKeyState(i32::from(vk.0)) } < 0);
        let in_front = || !target.is_invalid() && unsafe { GetForegroundWindow() } == target;
        let deadline = Instant::now() + Duration::from_millis(500);
        while (held() || !in_front()) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        if !in_front() || !unsafe { can_type_into(target) } {
            _ = self.events.send(OsEvent::PasteBlocked);
            return;
        }
        let exe = unsafe { exe_path(target) }.map(|p| file_name_lower(&p)).unwrap_or_default();
        let mut class = [0u16; 64];
        let len = unsafe { GetClassNameW(target, &mut class) }.max(0) as usize;
        let inputs = match chord_for(&exe, &String::from_utf16_lossy(&class[..len])) {
            Chord::CtrlV => [key(VK_CONTROL, false), key(VK_V, false), key(VK_V, true), key(VK_CONTROL, true)],
            Chord::ShiftInsert => [key(VK_SHIFT, false), key(VK_INSERT, false), key(VK_INSERT, true), key(VK_SHIFT, true)],
        };
        unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Chord {
    CtrlV,
    ShiftInsert,
}

/// Classic consoles and PuTTY-style terminals read Ctrl+V as a control character; they all
/// paste on Shift+Insert. Windows Terminal and everything else get Ctrl+V.
fn chord_for(exe: &str, class: &str) -> Chord {
    const SHIFT_INSERT: [&str; 4] = ["conhost.exe", "mintty.exe", "putty.exe", "kitty.exe"];
    if class == "ConsoleWindowClass" || SHIFT_INSERT.iter().any(|e| exe.eq_ignore_ascii_case(e)) {
        Chord::ShiftInsert
    } else {
        Chord::CtrlV
    }
}

/// One key event, with its scan code too: RDP and VM windows forward scan codes, not VKs.
fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    let scan = unsafe { MapVirtualKeyW(u32::from(vk.0), MAPVK_VK_TO_VSC) } as u16;
    let mut flags = if up { KEYEVENTF_KEYUP.0 } else { 0 };
    // Insert shares its scan code with numpad 0; the extended bit picks the real Insert.
    if vk == VK_INSERT {
        flags |= KEYEVENTF_EXTENDEDKEY.0;
    }
    INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: KEYBD_EVENT_FLAGS(flags), ..Default::default() } } }
}

/// The integrity level of a process's token.
unsafe fn integrity(process: HANDLE) -> Option<u32> {
    let mut token = HANDLE::default();
    OpenProcessToken(process, TOKEN_QUERY, &mut token).ok()?;
    let mut len = 0u32;
    _ = GetTokenInformation(token, TokenIntegrityLevel, None, 0, &mut len);
    // u64s: the buffer holds a pointer-aligned TOKEN_MANDATORY_LABEL.
    let mut buffer = vec![0u64; (len as usize).div_ceil(8).max(1)];
    let got = GetTokenInformation(token, TokenIntegrityLevel, Some(buffer.as_mut_ptr() as *mut c_void), len, &mut len);
    _ = CloseHandle(token);
    got.ok()?;
    let sid = (*(buffer.as_ptr() as *const TOKEN_MANDATORY_LABEL)).Label.Sid;
    let count = *GetSidSubAuthorityCount(sid);
    (count > 0).then(|| *GetSidSubAuthority(sid, u32::from(count) - 1))
}

/// UIPI drops input sent to a window of a higher integrity level (an elevated app). If its
/// token can't even be read, assume it's elevated unless we are too.
unsafe fn can_type_into(target: HWND) -> bool {
    let ours = integrity(GetCurrentProcess()).unwrap_or(MEDIUM_INTEGRITY);
    let mut pid = 0u32;
    GetWindowThreadProcessId(target, Some(&mut pid));
    let theirs = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok().and_then(|process| {
        let level = integrity(process);
        _ = CloseHandle(process);
        level
    });
    match theirs {
        Some(level) => level <= ours,
        None => ours >= HIGH_INTEGRITY,
    }
}

unsafe fn decode(wic: &IWICImagingFactory, bytes: &[u8]) -> Result<IWICBitmapFrameDecode> {
    let stream = wic.CreateStream()?;
    stream.InitializeFromMemory(bytes)?;
    wic.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnLoad)?.GetFrame(0)
}

unsafe fn source_size(source: &IWICBitmapSource) -> Result<(u32, u32)> {
    let (mut w, mut h) = (0u32, 0u32);
    source.GetSize(&mut w, &mut h)?;
    Ok((w, h))
}

/// A copied PNG is kept byte for byte once it's known to decode; only the thumbnail is new.
unsafe fn png_image(wic: &IWICImagingFactory, png: Vec<u8>) -> Result<(Content, u32, u32)> {
    let frame = decode(wic, &png)?;
    let (w, h) = source_size(&frame)?;
    if !within_cap(u64::from(w), u64::from(h)) {
        return Err(windows::core::Error::empty());
    }
    let thumb = scaled_png(wic, &frame, THUMB_PX, png_has_alpha(&png))?;
    drop(frame);
    Ok((Content::Image { png, thumb }, w, h))
}

/// CF_DIBV5 / CF_DIB → (PNG and thumbnail, width, height). 32 bpp keeps its alpha (taken as
/// straight, which is what browsers and Office write); other depths are opaque.
unsafe fn dib_image(wic: &IWICImagingFactory, dib: &[u8]) -> Result<(Content, u32, u32)> {
    let Some(px) = dib_pixels(dib) else { return bmp_image(wic, dib) };
    if !within_cap(u64::from(px.w), u64::from(px.h)) {
        return Err(windows::core::Error::empty());
    }
    let source: IWICBitmapSource = wic.CreateBitmapFromMemory(px.w, px.h, &GUID_WICPixelFormat32bppBGRA, px.w * 4, &px.bgra)?.cast()?;
    let format = if px.alpha { GUID_WICPixelFormat32bppBGRA } else { GUID_WICPixelFormat24bppBGR };
    let png = encode_png(wic, &convert(wic, &source, &format)?, format)?;
    let thumb = scaled_png(wic, &source, THUMB_PX, px.alpha)?;
    Ok((Content::Image { png, thumb }, px.w, px.h))
}

/// Any other DIB (palettes, 16/24 bpp, RLE) through WIC's BMP decoder: a DIB is a BMP
/// without its 14-byte file header.
unsafe fn bmp_image(wic: &IWICImagingFactory, dib: &[u8]) -> Result<(Content, u32, u32)> {
    if dib.len() < 40 {
        return Err(windows::core::Error::empty());
    }
    let header_size = u32::from_le_bytes(dib[0..4].try_into().unwrap());
    let bit_count = u16::from_le_bytes(dib[14..16].try_into().unwrap());
    let compression = u32::from_le_bytes(dib[16..20].try_into().unwrap());
    let colors_used = u32::from_le_bytes(dib[32..36].try_into().unwrap());
    let palette = if bit_count <= 8 { (if colors_used > 0 { colors_used } else { 1 << bit_count }) * 4 } else { colors_used * 4 };
    let masks = match (compression, header_size) {
        (3, 40) => 12,
        (6, 40) => 16,
        _ => 0,
    };
    let mut bmp = Vec::with_capacity(14 + dib.len());
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&((14 + dib.len()) as u32).to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&(14 + header_size + palette + masks).to_le_bytes());
    bmp.extend_from_slice(dib);

    let frame = decode(wic, &bmp)?;
    let (w, h) = source_size(&frame)?;
    if !within_cap(u64::from(w), u64::from(h)) {
        return Err(windows::core::Error::empty());
    }
    let source = convert(wic, &frame, &GUID_WICPixelFormat24bppBGR)?;
    let png = encode_png(wic, &source, GUID_WICPixelFormat24bppBGR)?;
    let thumb = scaled_png(wic, &source, THUMB_PX, false)?;
    Ok((Content::Image { png, thumb }, w, h))
}

/// A PNG no larger than `max_px` on its long side; with `alpha`, scaled premultiplied (no
/// dark fringes) and written as 32 bpp BGRA, else 24 bpp.
unsafe fn scaled_png(wic: &IWICImagingFactory, source: &IWICBitmapSource, max_px: u32, alpha: bool) -> Result<Vec<u8>> {
    let (w, h) = source_size(source)?;
    let k = (max_px as f32 / w.max(h).max(1) as f32).min(1.0);
    let working = if alpha { GUID_WICPixelFormat32bppPBGRA } else { GUID_WICPixelFormat24bppBGR };
    let scaler = wic.CreateBitmapScaler()?;
    scaler.Initialize(&convert(wic, source, &working)?, ((w as f32 * k) as u32).max(1), ((h as f32 * k) as u32).max(1), WICBitmapInterpolationModeFant)?;
    let format = if alpha { GUID_WICPixelFormat32bppBGRA } else { GUID_WICPixelFormat24bppBGR };
    encode_png(wic, &convert(wic, &scaler.cast()?, &format)?, format)
}

/// Any encoded image (a page's og:image) → small PNG thumbnail. Initialises COM on the
/// calling thread, so it can run from a blocking-pool thread.
pub fn thumbnail(bytes: &[u8], max_px: u32) -> Option<Vec<u8>> {
    unsafe {
        _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let frame = decode(&wic, bytes).ok()?;
        scaled_png(&wic, &frame, max_px, false).ok()
    }
}

/// The app's own icon as a 32×32 PNG with transparency.
fn app_icon(wic: &IWICImagingFactory, exe: &str) -> Option<Vec<u8>> {
    unsafe {
        let path: Vec<u16> = exe.encode_utf16().chain([0]).collect();
        let mut info = windows::Win32::UI::Shell::SHFILEINFOW::default();
        let found = windows::Win32::UI::Shell::SHGetFileInfoW(
            PCWSTR(path.as_ptr()),
            windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut info),
            size_of::<windows::Win32::UI::Shell::SHFILEINFOW>() as u32,
            windows::Win32::UI::Shell::SHGFI_ICON | windows::Win32::UI::Shell::SHGFI_LARGEICON,
        );
        if found == 0 || info.hIcon.is_invalid() {
            return None;
        }
        let bitmap = wic.CreateBitmapFromHICON(info.hIcon);
        _ = DestroyIcon(info.hIcon);
        let source: IWICBitmapSource = bitmap.ok()?.cast().ok()?;
        let source = convert(wic, &source, &GUID_WICPixelFormat32bppBGRA).ok()?;
        encode_png(wic, &source, GUID_WICPixelFormat32bppBGRA).ok()
    }
}

unsafe fn convert(wic: &IWICImagingFactory, source: &IWICBitmapSource, format: &GUID) -> Result<IWICBitmapSource> {
    let converter = wic.CreateFormatConverter()?;
    converter.Initialize(source, format, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeMedianCut)?;
    converter.cast()
}

unsafe fn encode_png(wic: &IWICImagingFactory, source: &IWICBitmapSource, pixel_format: GUID) -> Result<Vec<u8>> {
    let stream: IStream = SHCreateMemStream(None).ok_or_else(windows::core::Error::empty)?;
    let encoder = wic.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
    encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
    let mut frame = None;
    encoder.CreateNewFrame(&mut frame, std::ptr::null_mut())?;
    let frame = frame.ok_or_else(windows::core::Error::empty)?;
    frame.Initialize(None)?;
    let (w, h) = source_size(source)?;
    frame.SetSize(w, h)?;
    let mut format = pixel_format;
    frame.SetPixelFormat(&mut format)?;
    frame.WriteSource(source, std::ptr::null())?;
    frame.Commit()?;
    encoder.Commit()?;

    let mut stat = STATSTG::default();
    stream.Stat(&mut stat, STATFLAG_NONAME)?;
    stream.Seek(0, STREAM_SEEK_SET, None)?;
    let mut bytes = vec![0u8; stat.cbSize as usize];
    let mut read = 0u32;
    stream.Read(bytes.as_mut_ptr() as *mut c_void, bytes.len() as u32, Some(&mut read)).ok()?;
    bytes.truncate(read as usize);
    Ok(bytes)
}

/// PNG → (width, height, top-down straight BGRA), for the DIB formats of a paste-back.
unsafe fn png_to_bgra(wic: &IWICImagingFactory, png: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    let frame = decode(wic, png)?;
    let source = convert(wic, &frame, &GUID_WICPixelFormat32bppBGRA)?;
    let (w, h) = source_size(&source)?;
    let mut bgra = vec![0u8; w as usize * h as usize * 4];
    source.CopyPixels(std::ptr::null(), w * 4, &mut bgra)?;
    Ok((w, h, bgra))
}

/// `C:\...\chrome.exe` → "Chrome".
pub fn app_name(path: &str) -> String {
    let file = path.rsplit(['\\', '/']).next().unwrap_or(path);
    let stem = file.strip_suffix(".exe").or_else(|| file.strip_suffix(".EXE")).unwrap_or(file);
    let known = [
        ("chrome", "Chrome"),
        ("msedge", "Edge"),
        ("firefox", "Firefox"),
        ("code", "VS Code"),
        ("explorer", "File Explorer"),
        ("windowsterminal", "Terminal"),
        ("snippingtool", "Snipping Tool"),
        ("screenclippinghost", "Snipping Tool"),
        ("winword", "Word"),
        ("excel", "Excel"),
        ("powerpnt", "PowerPoint"),
        ("notepad", "Notepad"),
        ("figma", "Figma"),
        ("slack", "Slack"),
        ("discord", "Discord"),
    ];
    let lower = stem.to_ascii_lowercase();
    if let Some((_, name)) = known.iter().find(|(k, _)| *k == lower) {
        return name.to_string();
    }
    let mut chars = stem.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

fn bytemuck_u16(units: &[u16]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(units.as_ptr() as *const u8, units.len() * 2) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads (never changes) the foreground window and its caret; prints what it found.
    #[test]
    #[ignore]
    fn targets_are_captured_quickly() {
        let finder = CaretFinder::start().unwrap();
        for _ in 0..2 {
            let start = Instant::now();
            let target = unsafe { capture_target(Some(&finder)) };
            let took = start.elapsed();
            println!("{target:?} in {took:?}");
            assert!(target.hwnd != 0 && took < CARET_WAIT + Duration::from_millis(100));
        }
        assert_eq!(Target::now().caret, None);
    }

    #[test]
    fn source_apps_read_naturally() {
        assert_eq!(app_name(r"C:\Program Files\Google\Chrome\Application\chrome.exe"), "Chrome");
        assert_eq!(app_name(r"C:\Users\me\AppData\Local\Programs\Microsoft VS Code\Code.exe"), "VS Code");
        assert_eq!(app_name(r"C:\Tools\obsidian.exe"), "Obsidian");
        assert_eq!(file_name_lower(r"C:\Tools\KeePass.EXE"), "keepass.exe");
    }

    #[test]
    fn dib_sizes_from_headers() {
        let mut info = vec![0u8; 40];
        info[0..4].copy_from_slice(&40u32.to_le_bytes());
        info[4..8].copy_from_slice(&7680i32.to_le_bytes());
        info[8..12].copy_from_slice(&(-4320i32).to_le_bytes());
        assert_eq!(dib_size(&info), Some((7680, 4320)));
        let mut core = vec![0u8; 12];
        core[0..4].copy_from_slice(&12u32.to_le_bytes());
        core[4..6].copy_from_slice(&10u16.to_le_bytes());
        core[6..8].copy_from_slice(&20u16.to_le_bytes());
        assert_eq!(dib_size(&core), Some((10, 20)));
        assert_eq!(dib_size(&[0; 8]), None);
    }

    #[test]
    fn hotkeys_parse_and_normalise() {
        assert_eq!(Hotkey::parse("win+alt+v"), Some(Hotkey::default()));
        assert_eq!(Hotkey::parse(" Alt + Win + V "), Some(Hotkey::default()));
        let key = Hotkey::parse("shift+CTRL+v").unwrap();
        assert_eq!((key.modifiers, key.vk, key.label.as_str()), ((MOD_CONTROL | MOD_SHIFT).0, u32::from(b'V'), "Ctrl+Shift+V"));
        let key = Hotkey::parse("alt+f9").unwrap();
        assert_eq!((key.modifiers, key.vk, key.label.as_str()), (MOD_ALT.0, 0x78, "Alt+F9"));
        assert_eq!(Hotkey::parse("F24").map(|k| (k.vk, k.label)), Some((0x87, "F24".into())));
        assert_eq!(Hotkey::parse("Ctrl+Shift+5").map(|k| k.label), Some("Ctrl+Shift+5".into()));
        assert_eq!(Hotkey::parse("ctrl+pgup").map(|k| (k.vk, k.label)), Some((0x21, "Ctrl+PageUp".into())));
        // Layout-dependent: only check the label when the layout types it unshifted.
        if let Some(key) = Hotkey::parse("ctrl+`") {
            assert_eq!(key.label, "Ctrl+`");
        }
        for bad in ["", "V", "Ctrl+", "Hyper+V", "Ctrl+F25", "Ctrl+F0", "Ctrl+Banana", "Win+Alt+VV", "+V"] {
            assert_eq!(Hotkey::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn chords_per_target() {
        assert_eq!(chord_for("conhost.exe", ""), Chord::ShiftInsert);
        assert_eq!(chord_for("cmd.exe", "ConsoleWindowClass"), Chord::ShiftInsert);
        assert_eq!(chord_for("PuTTY.exe", "PuTTY"), Chord::ShiftInsert);
        assert_eq!(chord_for("mintty.exe", "mintty"), Chord::ShiftInsert);
        assert_eq!(chord_for("windowsterminal.exe", "CASCADIA_HOSTING_WINDOW_CLASS"), Chord::CtrlV);
        assert_eq!(chord_for("notepad.exe", "Notepad"), Chord::CtrlV);
        assert_eq!(chord_for("", ""), Chord::CtrlV);
    }

    #[test]
    fn cf_html_round_trips() {
        let html = "Version:0.9\r\nStartHTML:0000000105\r\nEndHTML:0000000199\r\nStartFragment:0000000141\r\nEndFragment:0000000163\r\n<html><body>\r\n<!--StartFragment--><b>héllo</b><!--EndFragment-->\r\n</body></html>";
        let mut clipboard = html_to_clipboard(html);
        assert_eq!(clipboard.last(), Some(&0));
        clipboard.extend_from_slice(&[0; 7]); // allocation padding
        assert_eq!(html_from_clipboard(&clipboard).as_deref(), Some(html));
        assert_eq!(html_from_clipboard(&[0, 0]), None);
    }

    #[test]
    fn text_helpers() {
        let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<u8>>();
        let mut padded = utf16("hi");
        padded.extend_from_slice(&[0, 0, 7, 7]);
        assert_eq!(until_nul(&padded, 2), utf16("hi").as_slice());
        assert!(is_blank_utf16(&utf16(" \r\n\t\u{3000}")));
        assert!(!is_blank_utf16(&utf16("  😀 ")));
        assert!(is_move(2) && !is_move(5) && !is_move(1) && !is_move(3));
    }

    fn png_with(color_type: u8, extra: &[&[u8; 4]]) -> Vec<u8> {
        let mut png = PNG_SIGNATURE.to_vec();
        let mut chunk = |kind: &[u8; 4], data: &[u8]| {
            png.extend_from_slice(&(data.len() as u32).to_be_bytes());
            png.extend_from_slice(kind);
            png.extend_from_slice(data);
            png.extend_from_slice(&[0; 4]);
        };
        let mut ihdr = [0u8; 13];
        ihdr[0..4].copy_from_slice(&640u32.to_be_bytes());
        ihdr[4..8].copy_from_slice(&480u32.to_be_bytes());
        ihdr[8] = 8;
        ihdr[9] = color_type;
        chunk(b"IHDR", &ihdr);
        for kind in extra {
            chunk(kind, &[1, 2, 3]);
        }
        chunk(b"IDAT", &[0; 5]);
        chunk(b"IEND", &[]);
        png
    }

    #[test]
    fn png_headers() {
        let rgba = png_with(6, &[]);
        assert_eq!(png_size(&rgba), Some((640, 480)));
        assert!(png_has_alpha(&rgba));
        assert!(!png_has_alpha(&png_with(2, &[])));
        assert!(png_has_alpha(&png_with(3, &[b"PLTE", b"tRNS"])));
        let mut padded = rgba.clone();
        padded.extend_from_slice(&[0; 9]);
        assert_eq!(png_end(&padded), Some(rgba.len()));
        assert_eq!(png_size(b"GIF89a.........................."), None);
    }

    /// A 2×2 bottom-up 32 bpp DIB with a 40-byte header.
    fn dib40(compression: u32, masks: &[u32], pixels: [u32; 4]) -> Vec<u8> {
        let mut dib = Vec::new();
        for v in [40, 2, 2, 1 | (32 << 16), compression, 16, 0, 0, 0, 0].iter().chain(masks).chain(&pixels) {
            dib.extend_from_slice(&v.to_le_bytes());
        }
        dib
    }

    #[test]
    fn dib_alpha_detection() {
        // Reserved byte all zero: opaque, alpha forced to 255. Rows are bottom-up.
        let opaque = dib_pixels(&dib40(0, &[], [0x00112233, 0x00445566, 0x00778899, 0x00AABBCC])).unwrap();
        assert!(!opaque.alpha);
        assert_eq!(&opaque.bgra[..8], &[0x99, 0x88, 0x77, 255, 0xCC, 0xBB, 0xAA, 255]);
        // One non-zero alpha anywhere: honoured everywhere.
        let alpha = dib_pixels(&dib40(0, &[], [0x00112233, 0x80445566, 0, 0])).unwrap();
        assert!(alpha.alpha);
        assert_eq!(&alpha.bgra[8..16], &[0x33, 0x22, 0x11, 0, 0x66, 0x55, 0x44, 0x80]);
        // BI_BITFIELDS with swapped red/blue masks after a 40-byte header.
        let swapped = dib_pixels(&dib40(3, &[0x0000_00FF, 0x0000_FF00, 0x00FF_0000], [0x00112233, 0, 0, 0])).unwrap();
        assert_eq!(&swapped.bgra[8..12], &[0x11, 0x22, 0x33, 255]);
        // Not 32 bpp: left to WIC.
        let mut dib24 = dib40(0, &[], [0; 4]);
        dib24[14] = 24;
        assert_eq!(dib_pixels(&dib24), None);
    }

    #[test]
    fn dibv5_round_trips_alpha() {
        let bgra: Vec<u8> = vec![10, 20, 30, 255, 40, 50, 60, 128, 70, 80, 90, 0, 1, 2, 3, 64, 5, 6, 7, 255, 8, 9, 10, 32];
        let v5 = dib_from_bgra(3, 2, &bgra, true);
        assert_eq!(v5.len(), 124 + 24);
        assert_eq!(dib_size(&v5), Some((3, 2)));
        assert_eq!(dib_pixels(&v5), Some(Pixels { w: 3, h: 2, bgra: bgra.clone(), alpha: true }));
        // CF_DIB: flattened onto white, read back as opaque.
        let flat = dib_pixels(&dib_from_bgra(3, 2, &bgra, false)).unwrap();
        assert!(!flat.alpha);
        assert_eq!(&flat.bgra[..12], &[10, 20, 30, 255, 147, 152, 157, 255, 255, 255, 255, 255]);
    }
}
