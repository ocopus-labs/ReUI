//! ReUI island: the dynamic-island surface. Renders whatever the hub says is in front and
//! sends the user's actions back. Normally launched by `reui-hub`; it reconnects on its
//! own if the hub restarts. Rest the pointer on it to peek, click to expand; right-click it
//! (or the tray icon) for visibility settings and Quit.

#![windows_subsystem = "windows"]

mod ipc;
mod order;
mod pill;
mod render;
mod settings;
mod tray;

use std::cell::{Cell, RefCell};

use windows::core::{w, Result, BOOL};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::WinRT::*;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use reui_design::tokens::island;

use crate::pill::Pill;
use crate::settings::{Settings, Visibility};
use crate::render::{COMPACT_MAX_WIDTH, EXPANDED_SURFACE};

/// Slack around the island inside the (click-through) window.
const MARGIN: f32 = 24.0;

/// Windows' "Animation effects" setting; when off, every morph uses the reduced spring.
fn animations_enabled() -> bool {
    let mut enabled = BOOL(1);
    unsafe {
        _ = SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, Some(&mut enabled as *mut BOOL as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
    }
    enabled.as_bool()
}

thread_local! {
    static PILL: RefCell<Option<Pill>> = const { RefCell::new(None) };
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
}

fn open_menu(hwnd: HWND) {
    let mut current = Visibility::default();
    with_pill(|pill| current = pill.visibility());
    match tray::menu(hwnd, current) {
        Some(tray::Command::Visibility(v)) => with_pill(|pill| pill.set_visibility(v)),
        Some(tray::Command::Quit) => unsafe { _ = DestroyWindow(hwnd) },
        None => {}
    }
}

/// Run `f` on the pill unless it's already borrowed (re-entrant window messages).
fn with_pill(f: impl FnOnce(&mut Pill)) {
    PILL.with(|cell| {
        if let Ok(mut slot) = cell.try_borrow_mut() {
            if let Some(pill) = slot.as_mut() {
                f(pill);
            }
        }
    });
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && wparam.0 as u32 == WM_MOUSEMOVE {
        let info = &*(lparam.0 as *const MSLLHOOKSTRUCT);
        with_pill(|pill| pill.on_mouse_move(info.pt));
    }
    CallNextHookEx(None, code, wparam, lparam)
}

unsafe extern "system" fn foreground_hook(_: HWINEVENTHOOK, _: u32, hwnd: HWND, _: i32, _: i32, _: u32, _: u32) {
    with_pill(|pill| pill.on_foreground(hwnd));
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        ipc::WM_HUB => {
            with_pill(|pill| pill.drain_hub());
            LRESULT(0)
        }
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_LBUTTONUP => {
            let x = (lparam.0 & 0xffff) as i16 as f32;
            let y = ((lparam.0 >> 16) & 0xffff) as i16 as f32;
            with_pill(|pill| pill.on_click(x, y));
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam.0 >> 16) & 0xffff) as i16;
            with_pill(|pill| pill.on_wheel(delta));
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            open_menu(hwnd);
            LRESULT(0)
        }
        tray::WM_TRAY => {
            if tray::wants_menu(lparam) {
                open_menu(hwnd);
            }
            LRESULT(0)
        }
        WM_TIMER => {
            with_pill(|pill| pill.on_timer(wparam.0));
            LRESULT(0)
        }
        WM_DESTROY => {
            tray::remove(hwnd);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ if msg != 0 && msg == TASKBAR_CREATED.get() => {
            tray::add(hwnd);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn main() -> Result<()> {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)?;
        RoInitialize(RO_INIT_SINGLETHREADED)?;
    }
    // Keeps the compositor's dispatcher alive for the life of the message loop.
    let _queue = unsafe {
        CreateDispatcherQueueController(DispatcherQueueOptions {
            dwSize: size_of::<DispatcherQueueOptions>() as u32,
            threadType: DQTYPE_THREAD_CURRENT,
            apartmentType: DQTAT_COM_NONE,
        })?
    };

    // Primary monitor, top centre; wide enough for the expanded pill, or the widest compact
    // pill plus its minimal bubble, whichever is wider.
    let monitor = unsafe { MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY) };
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    let (mut dpi, mut _dpi_y) = (96u32, 96u32);
    unsafe {
        _ = GetMonitorInfoW(monitor, &mut info);
        GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut _dpi_y)?;
    }
    let scale = dpi as f32 / 96.0;
    let half = (EXPANDED_SURFACE.0 / 2.0).max(COMPACT_MAX_WIDTH / 2.0 + island::minimal::GAP + island::minimal::SIZE);
    let width = ((half + MARGIN) * 2.0 * scale).ceil() as i32;
    let height = ((island::TOP + EXPANDED_SURFACE.1 + MARGIN) * scale).ceil() as i32;
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
            lpszClassName: w!("ReUI.Island"),
            ..Default::default()
        };
        if RegisterClassExW(&class) == 0 {
            return Err(windows::core::Error::from_thread());
        }
        let ex = WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_NOREDIRECTIONBITMAP | WS_EX_LAYERED | WS_EX_TRANSPARENT;
        let hwnd = CreateWindowExW(ex, w!("ReUI.Island"), w!("ReUI"), WS_POPUP, left, mon.top, width, height, None, None, Some(instance.into()), None)?;
        SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA)?;
        hwnd
    };

    let link = ipc::spawn(hwnd);
    let settings = Settings::load();
    let mut pill = Pill::new(hwnd, scale, window, monitor, !animations_enabled(), settings.visibility, link.to_hub, link.events)?;
    pill.refresh_visibility();
    PILL.with(|cell| *cell.borrow_mut() = Some(pill));
    unsafe { _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
    TASKBAR_CREATED.set(tray::taskbar_created());
    tray::add(hwnd);

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

    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            DispatchMessageW(&msg);
        }
        _ = UnhookWindowsHookEx(mouse);
        _ = UnhookWinEvent(foreground);
    }
    PILL.with(|cell| cell.borrow_mut().take());
    Ok(())
}
