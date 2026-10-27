//! Phase 0a spike: a native dynamic-island pill on Windows.UI.Composition.
//!
//! What it proves:
//! - spring morphs (compact / peek / expanded) that run inside DWM, not in our process
//! - Direct2D/DirectWrite content in composition surfaces
//! - click-through everywhere except the pill, without ever taking focus
//! - hiding while a fullscreen app is in front
//!
//! Flags: `--demo` toggles expanded/compact every 1.6 s (for measuring animation cost),
//! `--expanded` starts expanded, `--paused` starts with playback paused (bars stopped). Hover to peek, click to expand, right-click to quit.

mod draw;

use std::cell::RefCell;

use draw::{Button, Canvas, BAR_COUNT, BAR_GAP, BAR_H, BAR_RIGHT_PAD, BAR_W, COMPACT, EXPANDED, PEEK, TRACKS};
use windows::core::{h, w, Interface, Result, HSTRING};
use windows::Foundation::{IReference, PropertyValue, TimeSpan};
use windows::System::DispatcherQueueController;
use windows::UI::Color;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::WinRT::Composition::ICompositorDesktopInterop;
use windows::Win32::System::WinRT::*;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows_numerics::{Vector2, Vector3};

const TOP: f32 = 8.0;
const MARGIN: f32 = 24.0;
const CORNER: f32 = 34.0;

const TIMER_COLLAPSE: usize = 1;
const TIMER_DEMO: usize = 2;
const COLLAPSE_DELAY_MS: u32 = 450;

#[derive(Clone, Copy, PartialEq, Debug)]
enum State {
    Compact,
    Peek,
    Expanded,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Run `f` on the app unless it is already borrowed (re-entrant window messages).
fn with_app(f: impl FnOnce(&mut App)) {
    APP.with(|cell| {
        if let Ok(mut slot) = cell.try_borrow_mut() {
            if let Some(app) = slot.as_mut() {
                f(app);
            }
        }
    });
}

struct App {
    hwnd: HWND,
    scale: f32,
    window: RECT,
    monitor: HMONITOR,
    _queue: DispatcherQueueController,
    compositor: Compositor,
    _target: DesktopWindowTarget,
    root: ContainerVisual,
    props: CompositionPropertySet,
    bars: Vec<SpriteVisual>,
    bar_brush: CompositionColorBrush,
    canvas: Canvas,
    compact_surface: CompositionDrawingSurface,
    expanded_surface: CompositionDrawingSurface,
    state: State,
    hovered: bool,
    hidden: bool,
    clickthrough: bool,
    playing: bool,
    track: usize,
}

impl App {
    fn new(hwnd: HWND, scale: f32, window: RECT, monitor: HMONITOR, queue: DispatcherQueueController) -> Result<Self> {
        let s = scale;
        let compositor = Compositor::new()?;
        let target = unsafe { compositor.cast::<ICompositorDesktopInterop>()?.CreateDesktopWindowTarget(hwnd, true)? };
        let root = compositor.CreateContainerVisual()?;
        target.SetRoot(&root)?;

        // The pill's width and height are the only animated values; everything else is an
        // expression over them, so a morph is two springs evaluated by DWM.
        let props = compositor.CreatePropertySet()?;
        props.InsertScalar(h!("w"), COMPACT.0 * s)?;
        props.InsertScalar(h!("h"), COMPACT.1 * s)?;
        let win_w = (window.right - window.left) as f32;
        let expr = |text: &str, scalars: &[(&str, f32)]| -> Result<ExpressionAnimation> {
            let e = compositor.CreateExpressionAnimationWithExpression(&HSTRING::from(text))?;
            e.SetReferenceParameter(h!("p"), &props)?;
            for (k, v) in scalars {
                e.SetScalarParameter(&HSTRING::from(*k), *v)?;
            }
            Ok(e)
        };
        // Normalised expansion progress: 0 at peek size, 1 when fully expanded.
        let t = format!("Clamp((p.h - {hp}) / ({he} - {hp}), 0, 1)", hp = PEEK.1 * s, he = EXPANDED.1 * s);

        let pill = compositor.CreateSpriteVisual()?;
        pill.SetBrush(&compositor.CreateColorBrushWithColor(Color { A: 255, R: 0, G: 0, B: 0 })?)?;
        pill.StartAnimation(h!("Size"), &expr("Vector2(p.w, p.h)", &[])?)?;
        pill.StartAnimation(h!("Offset"), &expr("Vector3((W - p.w) / 2, T, 0)", &[("W", win_w), ("T", TOP * s)])?)?;
        let shape = compositor.CreateRoundedRectangleGeometry()?;
        shape.StartAnimation(h!("Size"), &expr("Vector2(p.w, p.h)", &[])?)?;
        shape.StartAnimation(h!("CornerRadius"), &expr("Vector2(Min(p.h / 2, R), Min(p.h / 2, R))", &[("R", CORNER * s)])?)?;
        pill.SetClip(&compositor.CreateGeometricClipWithGeometry(&shape)?)?;
        root.Children()?.InsertAtTop(&pill)?;

        let canvas = Canvas::new(&compositor, s)?;
        let compact_surface = canvas.surface(COMPACT)?;
        let expanded_surface = canvas.surface(EXPANDED)?;

        // Compact layer: stays centred, fades out quickly as the pill grows past peek size.
        let compact = compositor.CreateContainerVisual()?;
        compact.SetSize(Vector2::new(COMPACT.0 * s, COMPACT.1 * s))?;
        let max_dy = (PEEK.1 - COMPACT.1) / 2.0 * s;
        compact.StartAnimation(h!("Offset"), &expr("Vector3((p.w - Wc) / 2, Min((p.h - Hc) / 2, D), 0)", &[("Wc", COMPACT.0 * s), ("Hc", COMPACT.1 * s), ("D", max_dy)])?)?;
        compact.StartAnimation(h!("Opacity"), &expr(&format!("Clamp(1 - {t} * 4, 0, 1)"), &[])?)?;
        compact.Children()?.InsertAtTop(&surface_sprite(&compositor, &compact_surface, COMPACT, s)?)?;

        let bar_brush = compositor.CreateColorBrush()?;
        let mut bars = Vec::new();
        let x0 = COMPACT.0 - BAR_RIGHT_PAD - draw::bars_width();
        for i in 0..BAR_COUNT {
            let bar = compositor.CreateSpriteVisual()?;
            bar.SetBrush(&bar_brush)?;
            bar.SetSize(Vector2::new(BAR_W * s, BAR_H * s))?;
            bar.SetOffset(Vector3::new((x0 + i as f32 * (BAR_W + BAR_GAP)) * s, (COMPACT.1 - BAR_H) / 2.0 * s, 0.0))?;
            bar.SetCenterPoint(Vector3::new(BAR_W / 2.0 * s, BAR_H / 2.0 * s, 0.0))?;
            compact.Children()?.InsertAtTop(&bar)?;
            bars.push(bar);
        }
        pill.Children()?.InsertAtTop(&compact)?;

        // Expanded layer: fades and scales in during the second half of the morph.
        let expanded = surface_sprite(&compositor, &expanded_surface, EXPANDED, s)?;
        expanded.SetCenterPoint(Vector3::new(EXPANDED.0 / 2.0 * s, 0.0, 0.0))?;
        expanded.StartAnimation(h!("Offset"), &expr("Vector3((p.w - We) / 2, 0, 0)", &[("We", EXPANDED.0 * s)])?)?;
        expanded.StartAnimation(h!("Opacity"), &expr(&format!("Clamp({t} * 1.6 - 0.6, 0, 1)"), &[])?)?;
        expanded.StartAnimation(h!("Scale"), &expr(&format!("Vector3(0.92 + 0.08 * {t}, 0.92 + 0.08 * {t}, 1)"), &[])?)?;
        pill.Children()?.InsertAtTop(&expanded)?;

        let mut app = Self {
            hwnd,
            scale,
            window,
            monitor,
            _queue: queue,
            compositor,
            _target: target,
            root,
            props,
            bars,
            bar_brush,
            canvas,
            compact_surface,
            expanded_surface,
            state: State::Compact,
            hovered: false,
            hidden: false,
            clickthrough: true,
            playing: true,
            track: 0,
        };
        app.redraw()?;
        app.set_bars(true)?;
        Ok(app)
    }

    fn redraw(&mut self) -> Result<()> {
        let track = &TRACKS[self.track];
        self.canvas.draw_compact(&self.compact_surface, track)?;
        self.canvas.draw_expanded(&self.expanded_surface, track, self.playing)?;
        let (r, g, b) = track.from;
        self.bar_brush.SetColor(Color { A: 255, R: (r * 255.0) as u8, G: (g * 255.0) as u8, B: (b * 255.0) as u8 })
    }

    /// Bars loop in DWM while playing and are fully stopped when paused, so a paused pill costs nothing.
    fn set_bars(&self, playing: bool) -> Result<()> {
        for (i, bar) in self.bars.iter().enumerate() {
            if playing {
                let a = self.compositor.CreateScalarKeyFrameAnimation()?;
                a.InsertKeyFrame(0.0, 0.3)?;
                a.InsertKeyFrame(0.5, 1.0)?;
                a.InsertKeyFrame(1.0, 0.3)?;
                a.SetDuration(ms([520, 680, 600, 740][i % 4]))?;
                a.SetIterationBehavior(AnimationIterationBehavior::Forever)?;
                bar.StartAnimation(h!("Scale.Y"), &a)?;
            } else {
                bar.StopAnimation(h!("Scale.Y"))?;
                bar.SetScale(Vector3::new(1.0, 0.3, 1.0))?;
            }
        }
        Ok(())
    }

    fn set_state(&mut self, state: State) {
        if self.state == state {
            return;
        }
        self.state = state;
        // (size, damping ratio, period ms): bouncier on the way out, settled on the way back.
        let (size, damping, period) = match state {
            State::Compact => (COMPACT, 0.82, 55.0),
            State::Peek => (PEEK, 0.6, 45.0),
            State::Expanded => (EXPANDED, 0.68, 60.0),
        };
        let result = self
            .spring(h!("w"), size.0 * self.scale, damping, period)
            .and_then(|_| self.spring(h!("h"), size.1 * self.scale, damping + 0.04, period * 1.1));
        if let Err(e) = result {
            eprintln!("spring failed: {e}");
        }
    }

    fn spring(&self, key: &HSTRING, to: f32, damping: f32, period_ms: f32) -> Result<()> {
        let spring = self.compositor.CreateSpringScalarAnimation()?;
        spring.SetFinalValue(&PropertyValue::CreateSingle(to)?.cast::<IReference<f32>>()?)?;
        spring.SetDampingRatio(damping)?;
        spring.SetPeriod(TimeSpan { Duration: (period_ms * 10_000.0) as i64 })?;
        self.props.StartAnimation(key, &spring)
    }

    fn set_clickthrough(&mut self, on: bool) {
        if self.clickthrough == on {
            return;
        }
        self.clickthrough = on;
        unsafe {
            let ex = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE);
            let flag = WS_EX_TRANSPARENT.0 as isize;
            SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, if on { ex | flag } else { ex & !flag });
        }
    }

    /// Screen rectangle that counts as "on the pill". Extends to the top edge so slamming
    /// the cursor into the top of the screen still hits it.
    fn hit_rect(&self) -> (f32, f32, f32, f32) {
        let size = if self.state == State::Expanded { EXPANDED } else { PEEK };
        let s = self.scale;
        let win_w = (self.window.right - self.window.left) as f32;
        let left = self.window.left as f32 + (win_w - size.0 * s) / 2.0;
        let top = self.window.top as f32;
        (left, top, left + size.0 * s, top + (TOP + size.1) * s)
    }

    fn on_mouse_move(&mut self, pt: POINT) {
        if self.hidden {
            return;
        }
        let (l, t, r, b) = self.hit_rect();
        let (x, y) = (pt.x as f32, pt.y as f32);
        let inside = x >= l && x < r && y >= t && y < b;
        if inside == self.hovered {
            return;
        }
        self.hovered = inside;
        if inside {
            unsafe { _ = KillTimer(Some(self.hwnd), TIMER_COLLAPSE) };
            self.set_clickthrough(false);
            if self.state == State::Compact {
                self.set_state(State::Peek);
            }
        } else {
            self.set_clickthrough(true);
            match self.state {
                State::Expanded => unsafe { _ = SetTimer(Some(self.hwnd), TIMER_COLLAPSE, COLLAPSE_DELAY_MS, None) },
                State::Peek => self.set_state(State::Compact),
                State::Compact => {}
            }
        }
    }

    /// `x`, `y` are client pixels.
    fn on_click(&mut self, x: f32, y: f32) {
        if self.state != State::Expanded {
            self.set_state(State::Expanded);
            return;
        }
        let s = self.scale;
        let win_w = (self.window.right - self.window.left) as f32;
        let px = (x - (win_w - EXPANDED.0 * s) / 2.0) / s;
        let py = (y - TOP * s) / s;
        let Some(button) = draw::button_at(px, py) else { return };
        match button {
            Button::Prev => self.track = (self.track + TRACKS.len() - 1) % TRACKS.len(),
            Button::Next => self.track = (self.track + 1) % TRACKS.len(),
            Button::Play => {
                self.playing = !self.playing;
                if let Err(e) = self.set_bars(self.playing) {
                    eprintln!("bars failed: {e}");
                }
            }
        }
        if let Err(e) = self.redraw() {
            eprintln!("redraw failed: {e}");
        }
    }

    fn on_timer(&mut self, id: usize) {
        match id {
            TIMER_COLLAPSE => {
                unsafe { _ = KillTimer(Some(self.hwnd), TIMER_COLLAPSE) };
                if !self.hovered {
                    self.set_state(State::Compact);
                }
            }
            TIMER_DEMO => {
                let next = if self.state == State::Expanded { State::Compact } else { State::Expanded };
                self.set_state(next);
            }
            _ => {}
        }
    }

    fn on_foreground(&mut self, fg: HWND) {
        let hide = is_fullscreen(fg, self.monitor);
        if hide == self.hidden {
            return;
        }
        self.hidden = hide;
        _ = self.root.SetIsVisible(!hide);
        if hide {
            self.hovered = false;
            self.set_clickthrough(true);
            self.set_state(State::Compact);
        }
    }
}

fn surface_sprite(c: &Compositor, surface: &CompositionDrawingSurface, dips: (f32, f32), s: f32) -> Result<SpriteVisual> {
    let sprite = c.CreateSpriteVisual()?;
    sprite.SetSize(Vector2::new((dips.0 * s).ceil(), (dips.1 * s).ceil()))?;
    sprite.SetBrush(&c.CreateSurfaceBrushWithSurface(surface)?)?;
    Ok(sprite)
}

fn ms(v: i64) -> TimeSpan {
    TimeSpan { Duration: v * 10_000 }
}

/// A foreground window covering its whole monitor (and not the desktop) counts as fullscreen.
fn is_fullscreen(fg: HWND, monitor: HMONITOR) -> bool {
    unsafe {
        if let Ok(state) = SHQueryUserNotificationState() {
            if state == QUNS_RUNNING_D3D_FULL_SCREEN || state == QUNS_PRESENTATION_MODE {
                return true;
            }
        }
        if fg.is_invalid() || MonitorFromWindow(fg, MONITOR_DEFAULTTONULL) != monitor {
            return false;
        }
        let mut class = [0u16; 64];
        let len = GetClassNameW(fg, &mut class) as usize;
        let class = String::from_utf16_lossy(&class[..len]);
        if matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd") {
            return false;
        }
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let mut rect = RECT::default();
        GetMonitorInfoW(monitor, &mut info).as_bool()
            && GetWindowRect(fg, &mut rect).is_ok()
            && rect.left <= info.rcMonitor.left
            && rect.top <= info.rcMonitor.top
            && rect.right >= info.rcMonitor.right
            && rect.bottom >= info.rcMonitor.bottom
    }
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && wparam.0 as u32 == WM_MOUSEMOVE {
        let info = &*(lparam.0 as *const MSLLHOOKSTRUCT);
        with_app(|app| app.on_mouse_move(info.pt));
    }
    CallNextHookEx(None, code, wparam, lparam)
}

unsafe extern "system" fn foreground_hook(_: HWINEVENTHOOK, _: u32, hwnd: HWND, _: i32, _: i32, _: u32, _: u32) {
    with_app(|app| app.on_foreground(hwnd));
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_LBUTTONUP => {
            let x = (lparam.0 & 0xffff) as i16 as f32;
            let y = ((lparam.0 >> 16) & 0xffff) as i16 as f32;
            with_app(|app| app.on_click(x, y));
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_TIMER => {
            with_app(|app| app.on_timer(wparam.0));
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let demo = args.iter().any(|a| a == "--demo");
    let start_expanded = args.iter().any(|a| a == "--expanded");
    let start_paused = args.iter().any(|a| a == "--paused");

    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)?;
        RoInitialize(RO_INIT_SINGLETHREADED)?;
    }
    let queue = unsafe {
        CreateDispatcherQueueController(DispatcherQueueOptions {
            dwSize: size_of::<DispatcherQueueOptions>() as u32,
            threadType: DQTYPE_THREAD_CURRENT,
            apartmentType: DQTAT_COM_NONE,
        })?
    };

    // Primary monitor, top centre. Window is sized for the expanded pill plus slack;
    // only the pill itself ever receives input.
    let monitor = unsafe { MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY) };
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    let (mut dpi, mut _dpi_y) = (96u32, 96u32);
    unsafe {
        _ = GetMonitorInfoW(monitor, &mut info);
        GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut _dpi_y)?;
    }
    let scale = dpi as f32 / 96.0;
    let width = ((EXPANDED.0 + 2.0 * MARGIN) * scale).ceil() as i32;
    let height = ((TOP + EXPANDED.1 + MARGIN) * scale).ceil() as i32;
    let mon = info.rcMonitor;
    let left = mon.left + (mon.right - mon.left - width) / 2;
    let window = RECT { left, top: mon.top, right: left + width, bottom: mon.top + height };

    let hwnd = unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: w!("ReUI.Pill"),
            ..Default::default()
        };
        if RegisterClassExW(&class) == 0 {
            return Err(windows::core::Error::from_thread());
        }
        let ex = WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_NOREDIRECTIONBITMAP | WS_EX_LAYERED | WS_EX_TRANSPARENT;
        let hwnd = CreateWindowExW(ex, w!("ReUI.Pill"), w!("ReUI"), WS_POPUP, left, mon.top, width, height, None, None, Some(instance.into()), None)?;
        SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA)?;
        hwnd
    };

    let mut app = App::new(hwnd, scale, window, monitor, queue)?;
    if start_paused {
        app.playing = false;
        app.set_bars(false)?;
        app.redraw()?;
    }
    APP.with(|cell| *cell.borrow_mut() = Some(app));
    unsafe { _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
    if start_expanded {
        with_app(|app| app.set_state(State::Expanded));
    }
    if demo {
        unsafe { SetTimer(Some(hwnd), TIMER_DEMO, 1600, None) };
    }

    let (mouse, foreground) = unsafe {
        let instance = GetModuleHandleW(None)?;
        let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), Some(instance.into()), 0)?;
        let foreground = SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(foreground_hook),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        );
        (mouse, foreground)
    };

    println!("ReUI pill spike running (pid {}). Hover to peek, click to expand, right-click to quit.", std::process::id());

    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            DispatchMessageW(&msg);
        }
        _ = UnhookWindowsHookEx(mouse);
        _ = UnhookWinEvent(foreground);
    }
    APP.with(|cell| cell.borrow_mut().take());
    Ok(())
}
