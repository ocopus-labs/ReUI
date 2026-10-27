//! ReUI island: the dynamic-island surface. Renders whatever the hub says is in front and
//! sends the user's actions back. Normally launched by `reui-hub`; it reconnects on its
//! own if the hub restarts. Rest the pointer on it to peek, click to expand; right-click it
//! (or the tray icon) for visibility settings and Quit.

#![windows_subsystem = "windows"]

mod audio;
mod images;
mod ipc;
mod keys;
mod music;
mod order;
mod panel;
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
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::*;

use reui_design::tokens::island;

use crate::pill::{MenuEntry, Pill};
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
        Some(tray::Command::Module(module, action)) => with_pill(|pill| pill.command(module, action)),
        Some(tray::Command::Quit) => unsafe { _ = DestroyWindow(hwnd) },
        None => {}
    }
}

/// Right-click: a card's own menu over a card that has one, the island's menu elsewhere.
fn right_click(hwnd: HWND, x: f32, y: f32) {
    let mut entries = None;
    with_pill(|pill| entries = pill.card_menu(x, y));
    match entries {
        Some(entries) => card_menu(hwnd, entries),
        None => open_menu(hwnd),
    }
}

/// A menu from the open card (a card's actions, a slider's output devices). It runs
/// outside the pill's borrow so hooks and timers keep working meanwhile.
fn card_menu(hwnd: HWND, entries: Vec<MenuEntry>) {
    if entries.is_empty() {
        return;
    }
    with_pill(|pill| pill.menu_opened());
    let items: Vec<(&str, bool)> = entries.iter().map(|(label, _, _, checked)| (label.as_str(), *checked)).collect();
    let (picked, previous) = tray::item_menu(hwnd, &items);
    with_pill(|pill| {
        pill.menu_closed(previous);
        if let Some(i) = picked {
            let (_, action, closes, _) = &entries[i];
            pill.act(action.clone(), *closes);
        }
    });
}

fn client_point(lparam: LPARAM) -> (f32, f32) {
    ((lparam.0 & 0xffff) as i16 as f32, ((lparam.0 >> 16) & 0xffff) as i16 as f32)
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
    if code >= 0 {
        let info = &*(lparam.0 as *const MSLLHOOKSTRUCT);
        match wparam.0 as u32 {
            WM_MOUSEMOVE => with_pill(|pill| pill.on_mouse_move(info.pt)),
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN => with_pill(|pill| pill.on_mouse_down(info.pt)),
            _ => {}
        }
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
        keys::WM_KEYS => {
            with_pill(|pill| pill.on_keys());
            LRESULT(0)
        }
        // A press on a bar in the open card drags it, with the mouse captured so the drag
        // follows the pointer off the island; anything else acts on release.
        WM_LBUTTONDOWN => {
            let (x, y) = client_point(lparam);
            let mut dragging = false;
            with_pill(|pill| dragging = pill.on_press(x, y));
            if dragging {
                SetCapture(hwnd);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = client_point(lparam);
            let shift = wparam.0 & 0x0004 != 0; // MK_SHIFT
            let (mut dragged, mut menu) = (false, None);
            with_pill(|pill| {
                dragged = pill.on_release(x, y);
                if !dragged {
                    menu = pill.click_menu(x, y);
                }
            });
            if dragged {
                _ = ReleaseCapture();
            } else if let Some(entries) = menu {
                card_menu(hwnd, entries);
            } else {
                with_pill(|pill| pill.on_click(x, y, shift));
            }
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            with_pill(|pill| pill.on_capture_lost());
            LRESULT(0)
        }
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            let delta = ((wparam.0 >> 16) & 0xffff) as i16;
            // Wheel messages carry screen coordinates.
            let (x, y) = client_point(lparam);
            with_pill(|pill| pill.on_wheel(delta, msg == WM_MOUSEHWHEEL, POINT { x: x as i32, y: y as i32 }));
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            let (x, y) = client_point(lparam);
            right_click(hwnd, x, y);
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
