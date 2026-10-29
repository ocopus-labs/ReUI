//! The Win32 window: a normal, resizable, activating window with our own caption. The
//! system title bar is removed (the client area covers the whole frame), so the black
//! material runs edge to edge; DWM still gives it rounded corners, a shadow, snapping and
//! the dark border. We answer hit-testing for the resize edges and the drag band. Per-monitor
//! DPI v2: every size scales with the monitor it's on.
//!
//! The message loop draws: when the app has something to show (a change, or motion in
//! flight) it waits on the swap chain's frame-latency object as well as on messages, and
//! draws a frame each time the display can take one. At rest it only waits for messages.
//! The window has no redirection bitmap (the swap chain is its only content), and resizing
//! draws straight away, so the edges never show stale or stretched pixels.

use std::cell::RefCell;

use reui_design::tokens::settings as s;
use windows::core::{w, Result, BOOL, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{WaitForSingleObjectEx, INFINITE};
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::app::App;
use crate::keys;
use crate::pages::Page;
use crate::state::State;

pub const CLASS: PCWSTR = w!("ReUI.Settings");
pub const SANDBOX_CLASS: PCWSTR = w!("ReUI.Settings.Sandbox");
const WM_MOUSELEAVE: u32 = 0x02A3;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    static TRACKING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Runs `f` on the app unless it's already borrowed (a message sent from inside a handler).
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| cell.try_borrow_mut().ok().and_then(|mut app| app.as_mut().map(f)))
}

fn dpi_of(hwnd: HWND) -> u32 {
    unsafe { GetDpiForWindow(hwnd) }.max(96)
}

fn point(lparam: LPARAM, scale: f32) -> (f32, f32) {
    let x = (lparam.0 & 0xffff) as i16 as f32;
    let y = ((lparam.0 >> 16) & 0xffff) as i16 as f32;
    (x / scale, y / scale)
}

/// Windows' "Animation effects" setting.
fn animations_enabled() -> bool {
    let mut enabled = BOOL(1);
    unsafe { _ = SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, Some(&mut enabled as *mut BOOL as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)) };
    enabled.as_bool()
}

/// Opens the window (`visible`: shown and brought to the front; otherwise it stays hidden,
/// for screenshots).
pub fn create(state: State, page: Page, visible: bool) -> Result<HWND> {
    let class = if state.paths.sandbox { SANDBOX_CLASS } else { CLASS };
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            // The ReUI logo, linked in by build.rs.
            hIcon: logo(instance, SM_CXICON, SM_CYICON),
            hIconSm: logo(instance, SM_CXSMICON, SM_CYSMICON),
            lpszClassName: class,
            ..Default::default()
        };
        if RegisterClassExW(&wc) == 0 {
            return Err(windows::core::Error::from_thread());
        }
        // Centred on the monitor under the pointer, sized for its DPI.
        let mut pt = POINT::default();
        _ = GetCursorPos(&mut pt);
        let monitor = MonitorFromPoint(pt, MONITOR_DEFAULTTOPRIMARY);
        let (mut dpi, mut _y) = (96u32, 96u32);
        _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut _y);
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        _ = GetMonitorInfoW(monitor, &mut info);
        let work = info.rcWork;
        let scale = dpi as f32 / 96.0;
        let w = ((s::window::WIDTH * scale) as i32).min(work.right - work.left);
        let h = ((s::window::HEIGHT * scale) as i32).min(work.bottom - work.top);
        let x = work.left + (work.right - work.left - w) / 2;
        let y = work.top + (work.bottom - work.top - h) / 2;
        let hwnd = CreateWindowExW(WS_EX_NOREDIRECTIONBITMAP, class, w!("ReUI Settings"), WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN, x, y, w, h, None, None, Some(instance.into()), None)?;

        let dark = BOOL(1);
        _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const _ as *const _, 4);
        let round = DWMWCP_ROUND;
        _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &round as *const _ as *const _, 4);
        let border = colorref(s::color::BORDER);
        _ = DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR, &border as *const _ as *const _, 4);
        // Keeps DWM's shadow on a window without a system frame.
        _ = DwmExtendFrameIntoClientArea(hwnd, &MARGINS { cxLeftWidth: 0, cxRightWidth: 0, cyTopHeight: 1, cyBottomHeight: 0 });

        let app = App::new(hwnd, state, page, dpi_of(hwnd), !animations_enabled())?;
        APP.with(|cell| *cell.borrow_mut() = Some(app));
        // Recompute the frame now that WM_NCCALCSIZE removes it.
        _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER);
        let mut rc = RECT::default();
        _ = GetClientRect(hwnd, &mut rc);
        with_app(|app| app.resize(rc.right as u32, rc.bottom as u32));
        if visible {
            crate::wallpaper::load(hwnd);
            _ = ShowWindow(hwnd, SW_SHOWNORMAL);
            _ = SetForegroundWindow(hwnd);
        }
        Ok(hwnd)
    }
}

fn colorref(c: reui_design::Color) -> COLORREF {
    let [r, g, b, _] = c.to_rgba8();
    COLORREF(u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16))
}

pub fn run() {
    let mut msg = MSG::default();
    'running: loop {
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    break 'running;
                }
                _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        let (needs, waitable) = with_app(|app| (app.needs_frame(), app.waitable())).unwrap_or((false, None));
        match (needs, waitable) {
            // The first frame makes the swap chain.
            (true, None) => _ = with_app(|app| app.frame()),
            (true, Some(handle)) => {
                // Input first: a frame only when the display can take one.
                let woke = unsafe { MsgWaitForMultipleObjectsEx(Some(&[handle]), INFINITE, QS_ALLINPUT, MWMO_INPUTAVAILABLE) };
                if woke == WAIT_OBJECT_0 {
                    with_app(|app| app.frame());
                }
            }
            (false, _) => unsafe {
                _ = WaitMessage();
            },
        }
    }
    keys::stop();
    APP.with(|cell| cell.borrow_mut().take());
}

/// `--shots`: every page drawn offscreen into `dir`, without showing the window.
pub fn shots(state: State, dir: &std::path::Path) -> Result<()> {
    create(state, Page::General, false)?;
    let done = APP.with(|cell| cell.borrow_mut().as_mut().map(|app| crate::shots::run(app, dir)));
    keys::stop();
    APP.with(|cell| cell.borrow_mut().take());
    done.unwrap_or(Ok(()))
}

/// Draws now, outside the loop's pacing (the system asked: resizing, uncovering).
fn paint_now() {
    with_app(|app| {
        if let Some(handle) = app.waitable() {
            unsafe { _ = WaitForSingleObjectEx(handle, 50, true) };
        }
        app.invalidate();
        crate::perf::reason("paint");
        app.frame();
    });
}

/// Brings an already-open Settings window to the front.
pub fn activate_existing(sandbox: bool) -> bool {
    unsafe {
        let Ok(hwnd) = FindWindowW(if sandbox { SANDBOX_CLASS } else { CLASS }, PCWSTR::null()) else { return false };
        if IsIconic(hwnd).as_bool() {
            _ = ShowWindow(hwnd, SW_RESTORE);
        }
        _ = SetForegroundWindow(hwnd);
        true
    }
}

fn hit_test(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    let scale = dpi_of(hwnd) as f32 / 96.0;
    let mut pt = POINT { x: (lparam.0 & 0xffff) as i16 as i32, y: ((lparam.0 >> 16) & 0xffff) as i16 as i32 };
    let mut rc = RECT::default();
    unsafe {
        _ = ScreenToClient(hwnd, &mut pt);
        _ = GetClientRect(hwnd, &mut rc);
    }
    let (x, y) = (pt.x as f32 / scale, pt.y as f32 / scale);
    let (w, h) = (rc.right as f32 / scale, rc.bottom as f32 / scale);
    let maximized = unsafe { IsZoomed(hwnd).as_bool() };
    if !maximized {
        let b = s::window::RESIZE_BORDER;
        let (left, right, top, bottom) = (x < b, x >= w - b, y < b, y >= h - b);
        let code = match (left, right, top, bottom) {
            (true, _, true, _) => Some(HTTOPLEFT),
            (_, true, true, _) => Some(HTTOPRIGHT),
            (true, _, _, true) => Some(HTBOTTOMLEFT),
            (_, true, _, true) => Some(HTBOTTOMRIGHT),
            (true, ..) => Some(HTLEFT),
            (_, true, ..) => Some(HTRIGHT),
            (_, _, true, _) => Some(HTTOP),
            (.., true) => Some(HTBOTTOM),
            _ => None,
        };
        if let Some(code) = code {
            return LRESULT(code as isize);
        }
    }
    let caption = with_app(|app| app.is_caption(x, y)).unwrap_or(false);
    LRESULT(if caption { HTCAPTION } else { HTCLIENT } as isize)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let scale = || dpi_of(hwnd) as f32 / 96.0;
    match msg {
        WM_NCCALCSIZE if wparam.0 != 0 => {
            // The client area is the whole window; maximised, Windows puts the frame
            // off-screen, so pull it back in.
            if IsZoomed(hwnd).as_bool() {
                let params = &mut *(lparam.0 as *mut NCCALCSIZE_PARAMS);
                let dpi = dpi_of(hwnd);
                let frame = GetSystemMetricsForDpi(SM_CXFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
                let r = &mut params.rgrc[0];
                r.left += frame;
                r.top += frame;
                r.right -= frame;
                r.bottom -= frame;
            }
            LRESULT(0)
        }
        WM_NCHITTEST => hit_test(hwnd, lparam),
        WM_GETMINMAXINFO => {
            let info = &mut *(lparam.0 as *mut MINMAXINFO);
            let scale = scale();
            info.ptMinTrackSize = POINT { x: (s::window::MIN_WIDTH * scale) as i32, y: (s::window::MIN_HEIGHT * scale) as i32 };
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let r = &*(lparam.0 as *const RECT);
            with_app(|app| app.set_dpi(u32::from((wparam.0 & 0xffff) as u16)));
            _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            LRESULT(0)
        }
        WM_SIZE => {
            let (w, h) = ((lparam.0 & 0xffff) as u32, ((lparam.0 >> 16) & 0xffff) as u32);
            if w > 0 && h > 0 {
                with_app(|app| app.resize(w, h));
                paint_now();
            }
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            BeginPaint(hwnd, &mut ps);
            _ = EndPaint(hwnd, &ps);
            paint_now();
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if !TRACKING.get() {
                let mut track = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                _ = TrackMouseEvent(&mut track);
                TRACKING.set(true);
            }
            let (x, y) = point(lparam, scale());
            with_app(|app| app.mouse_move(x, y));
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            TRACKING.set(false);
            with_app(|app| app.mouse_leave());
            LRESULT(0)
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            let (x, y) = point(lparam, scale());
            if with_app(|app| app.mouse_down(x, y)).unwrap_or(false) {
                SetCapture(hwnd);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = point(lparam, scale());
            if GetCapture() == hwnd {
                _ = ReleaseCapture();
            }
            with_app(|app| app.mouse_up(x, y));
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam.0 >> 16) & 0xffff) as i16;
            with_app(|app| app.wheel(delta));
            LRESULT(0)
        }
        WM_SYSKEYDOWN => {
            if with_app(|app| app.sys_key(wparam.0 as u32)).unwrap_or(false) {
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_XBUTTONUP => {
            let button = ((wparam.0 >> 16) & 0xffff) as u16;
            with_app(|app| app.x_button(button));
            LRESULT(1)
        }
        WM_KEYDOWN => {
            let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
            let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
            if with_app(|app| app.key(wparam.0 as u32, shift, ctrl)).unwrap_or(false) {
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_CHAR => {
            if let Some(c) = char::from_u32(wparam.0 as u32) {
                with_app(|app| app.char(c));
            }
            LRESULT(0)
        }
        WM_SETCURSOR if (lparam.0 & 0xffff) as u32 == HTCLIENT => {
            let field = with_app(|app| app.over_field()).unwrap_or(false);
            _ = SetCursor(LoadCursorW(None, if field { IDC_IBEAM } else { IDC_ARROW }).ok());
            LRESULT(1)
        }
        WM_ACTIVATE => {
            let active = (wparam.0 & 0xffff) as u32 != WA_INACTIVE;
            with_app(|app| app.activated(active));
            LRESULT(0)
        }
        WM_TIMER => {
            with_app(|app| app.on_timer(wparam.0));
            LRESULT(0)
        }
        keys::WM_CHORD => {
            with_app(|app| app.on_chord(wparam.0 as u32, lparam.0 as u32));
            LRESULT(0)
        }
        keys::WM_RECORD_END => {
            keys::stop();
            LRESULT(0)
        }
        WM_SETTINGCHANGE => {
            if crate::wallpaper::changed(wparam) {
                crate::wallpaper::load(hwnd);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        crate::wallpaper::WM_WALLPAPER => {
            with_app(|app| app.on_wallpaper());
            LRESULT(0)
        }
        crate::updates::WM_UPDATES => {
            with_app(|app| app.on_updates());
            LRESULT(0)
        }
        WM_DESTROY => {
            with_app(|app| app.closing());
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// The ReUI logo (resource 1) at a system icon size; Windows' default icon if it's missing.
unsafe fn logo(instance: HMODULE, w: SYSTEM_METRICS_INDEX, h: SYSTEM_METRICS_INDEX) -> HICON {
    match LoadImageW(Some(instance.into()), PCWSTR(reui_design::icon::ICON_ID as usize as *const u16), IMAGE_ICON, GetSystemMetrics(w), GetSystemMetrics(h), LR_SHARED) {
        Ok(icon) => HICON(icon.0),
        Err(_) => LoadIconW(None, IDI_APPLICATION).unwrap_or_default(),
    }
}
