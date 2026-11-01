//! ReUI island: the dynamic-island surface. Renders whatever the hub says is in front and
//! sends the user's actions back. Normally launched by `reui-hub`; it reconnects on its
//! own if the hub restarts. Rest the pointer on it to peek, click to expand; right-click it
//! (or the tray icon) for the short menu; a left-click on the tray icon opens Settings.
//!
//! One island per window: one window in most modes, one per display in "All displays"
//! (`islands.rs`, docs/PRD-all-displays.md). What there's one of (the hub link, the tray,
//! settings, Windows' broadcasts) lives on a hidden host window (`host.rs`); this file is
//! the island windows' own messages and the process-wide hooks.

#![windows_subsystem = "windows"]

mod desktop;
mod app_rules;
mod capture;
mod drag_guard;
mod dragdrop;
mod features;
mod fullscreen;
mod host;
mod images;
mod islands;
mod menus;
mod open_cards;
mod intent;
mod ipc;
mod order;
mod pill;
mod place;
mod quiet;
mod render;
mod settings;
mod theme;
mod tray;

use std::cell::Cell;

use windows::core::{Result, BOOL};
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::WinRT::*;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, ReleaseCapture, SetCapture, VK_MENU};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::features::clipboard_keys;
use crate::islands::{each_pill, with_pill};

/// Debug builds: a test script's pointer position (see the window procedure).
#[cfg(debug_assertions)]
const WM_TEST_POINTER: u32 = WM_APP + 20;
/// Debug builds: a test script's drag over the island (wparam 1: it comes over the pill's
/// middle, 0: it leaves, 2: it's let go), so the drop target and the shelf's intake can be
/// checked without a real drag (`Pill::test_drag`).
#[cfg(debug_assertions)]
const WM_TEST_DROP: u32 = WM_APP + 23;
/// Debug builds: a test script's "this process is in front" (wparam: its id), for app rules.
#[cfg(debug_assertions)]
const WM_TEST_FOREGROUND: u32 = WM_APP + 21;
/// Debug builds: a test script picks entry `wparam` of the quiet menu for the moment at
/// client point `lparam`, without showing a menu at the user's pointer.
#[cfg(debug_assertions)]
const WM_TEST_QUIET: u32 = WM_APP + 22;

/// Windows' "Animation effects" setting; when off, every morph uses the reduced spring.
fn animations_enabled() -> bool {
    let mut enabled = BOOL(1);
    unsafe {
        _ = SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, Some(&mut enabled as *mut BOOL as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
    }
    enabled.as_bool()
}

thread_local! {
    /// Location changes of the foreground app's windows (a browser video going fullscreen
    /// keeps its window), re-registered for each new foreground process.
    static LOCATION_HOOK: Cell<Option<HWINEVENTHOOK>> = const { Cell::new(None) };
}

fn press_drag(hwnd: HWND, lparam: LPARAM) -> bool {
    let (x, y) = client_point(lparam);
    let mut pressed = false;
    with_pill(hwnd, |pill| pressed = pill.press_drag(x, y));
    pressed
}

fn client_point(lparam: LPARAM) -> (f32, f32) {
    ((lparam.0 & 0xffff) as i16 as f32, ((lparam.0 >> 16) & 0xffff) as i16 as f32)
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let pt = (*(lparam.0 as *const MSLLHOOKSTRUCT)).pt;
        match wparam.0 as u32 {
            WM_MOUSEMOVE => {
                each_pill(|pill| pill.on_mouse_move(pt));
                // With an island on each display, the one under the pointer leads.
                islands::update_lead(pt);
            }
            WM_LBUTTONDOWN => each_pill(|pill| pill.on_mouse_down(pt, drag_guard::LEFT)),
            WM_RBUTTONDOWN => each_pill(|pill| pill.on_mouse_down(pt, drag_guard::RIGHT)),
            WM_MBUTTONDOWN => each_pill(|pill| pill.on_mouse_down(pt, drag_guard::MIDDLE)),
            WM_LBUTTONUP => each_pill(|pill| pill.on_mouse_up(pt, drag_guard::LEFT)),
            WM_RBUTTONUP => each_pill(|pill| pill.on_mouse_up(pt, drag_guard::RIGHT)),
            WM_MBUTTONUP => each_pill(|pill| pill.on_mouse_up(pt, drag_guard::MIDDLE)),
            _ => {}
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

unsafe extern "system" fn foreground_hook(_: HWINEVENTHOOK, _: u32, hwnd: HWND, _: i32, _: i32, _: u32, _: u32) {
    each_pill(|pill| pill.on_foreground(hwnd));
    watch_location(hwnd);
    host::recheck_fullscreen_soon();
}

/// Follows the foreground app's window moves and resizes (only that process's, so it stays
/// cheap), to notice it going fullscreen without a foreground change.
fn watch_location(fg: HWND) {
    unsafe {
        if let Some(old) = LOCATION_HOOK.take() {
            _ = UnhookWinEvent(old);
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(fg, Some(&mut pid));
        if pid == 0 || pid == std::process::id() {
            return;
        }
        let hook = SetWinEventHook(EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_LOCATIONCHANGE, None, Some(location_hook), pid, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS);
        if !hook.is_invalid() {
            LOCATION_HOOK.set(Some(hook));
        }
    }
}

unsafe extern "system" fn location_hook(_: HWINEVENTHOOK, _: u32, hwnd: HWND, object: i32, child: i32, _: u32, _: u32) {
    // The window itself (not its caret or a control), and only the one in front.
    if object == OBJID_WINDOW.0 && child == CHILDID_SELF as i32 && hwnd == GetForegroundWindow() {
        each_pill(|pill| pill.recheck_fullscreen());
        host::recheck_fullscreen_soon();
    }
}

/// An island window's own messages.
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        // The `monitor` setting changed (posted by the island that read it).
        place::WM_PLACE => {
            islands::sync(false);
            LRESULT(0)
        }
        pill::WM_LOOK => {
            islands::sync(true);
            LRESULT(0)
        }
        // This window's display changed scale.
        WM_DPICHANGED => {
            host::schedule_place(false);
            LRESULT(0)
        }
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        // Debug builds only: a pointer position (screen pixels, lparam as for WM_MOUSEMOVE)
        // posted by a test script, so hover and peek can be checked without moving the
        // user's real mouse. Release builds never read it.
        #[cfg(debug_assertions)]
        WM_TEST_POINTER => {
            let (x, y) = client_point(lparam);
            with_pill(hwnd, |pill| pill.on_mouse_move(POINT { x: x as i32, y: y as i32 }));
            LRESULT(0)
        }
        #[cfg(debug_assertions)]
        WM_TEST_DROP => {
            with_pill(hwnd, |pill| pill.test_drag(wparam.0));
            LRESULT(0)
        }
        #[cfg(debug_assertions)]
        WM_TEST_FOREGROUND => {
            with_pill(hwnd, |pill| pill.test_front_app(wparam.0 as u32));
            LRESULT(0)
        }
        #[cfg(debug_assertions)]
        WM_TEST_QUIET => {
            let (x, y) = client_point(lparam);
            with_pill(hwnd, |pill| pill.test_quiet(x, y, wparam.0));
            LRESULT(0)
        }
        clipboard_keys::WM_KEYS_TAKEN => {
            with_pill(hwnd, |pill| pill.keys_taken());
            LRESULT(0)
        }
        clipboard_keys::WM_KEYS => {
            with_pill(hwnd, |pill| pill.on_keys());
            LRESULT(0)
        }
        // A press on a bar in the open card drags it, with the mouse captured so the drag
        // follows the pointer off the island; anything else acts on release.
        // A press on something that can be dragged out (a held file's card, the shelf's
        // pill) is watched with the mouse captured; moving past the drag distance starts
        // Windows' drag, outside the pill's borrow (the island's own drop target and
        // timers run during it).
        WM_LBUTTONDOWN if press_drag(hwnd, lparam) => {
            SetCapture(hwnd);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let (x, y) = client_point(lparam);
            let mut out = None;
            with_pill(hwnd, |pill| out = pill.drag_out_due(x, y));
            if let Some(drag) = out {
                _ = ReleaseCapture();
                dragdrop::drag_out(hwnd, &drag);
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let (x, y) = client_point(lparam);
            let mut dragging = false;
            with_pill(hwnd, |pill| dragging = pill.on_press(x, y));
            if dragging {
                SetCapture(hwnd);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = client_point(lparam);
            let mut pressed = false;
            with_pill(hwnd, |pill| pressed = pill.release_press());
            if pressed {
                _ = ReleaseCapture();
            }
            let shift = wparam.0 & 0x0004 != 0; // MK_SHIFT
            // The island never has focus, so Alt is read from the hardware state.
            let alt = GetAsyncKeyState(VK_MENU.0 as i32) as u16 & 0x8000 != 0;
            let (mut dragged, mut menu) = (false, None);
            with_pill(hwnd, |pill| {
                dragged = pill.on_release(x, y);
                if !dragged {
                    menu = pill.click_menu(x, y);
                }
            });
            if dragged {
                _ = ReleaseCapture();
            } else if let Some(entries) = menu {
                menus::card_menu(hwnd, entries);
            } else {
                with_pill(hwnd, |pill| pill.on_click(x, y, shift, alt));
            }
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            with_pill(hwnd, |pill| pill.on_capture_lost());
            LRESULT(0)
        }
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            let delta = ((wparam.0 >> 16) & 0xffff) as i16;
            // Wheel messages carry screen coordinates.
            let (x, y) = client_point(lparam);
            with_pill(hwnd, |pill| pill.on_wheel(delta, msg == WM_MOUSEHWHEEL, POINT { x: x as i32, y: y as i32 }));
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            let (x, y) = client_point(lparam);
            menus::right_click(hwnd, x, y);
            LRESULT(0)
        }
        // The pointer settled on another display ("cursor" mode).
        WM_TIMER if wparam.0 == place::TIMER_FOLLOW => {
            _ = KillTimer(Some(hwnd), wparam.0);
            islands::sync(false);
            LRESULT(0)
        }
        WM_TIMER => {
            with_pill(hwnd, |pill| pill.on_timer(wparam.0));
            LRESULT(0)
        }
        // An island's window goes with its display, or with the process.
        WM_DESTROY => {
            dragdrop::unregister(hwnd);
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

    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: islands::CLASS,
            ..Default::default()
        };
        if RegisterClassExW(&class) == 0 {
            return Err(windows::core::Error::from_thread());
        }
    }

    let host = host::create()?;
    let link = ipc::spawn(host);
    islands::set_link(link.to_hub, link.events);
    // An island on the chosen display (primary by default), or on each display.
    islands::sync(false);
    settings::watch(host);
    watch_location(unsafe { GetForegroundWindow() });

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
        if let Some(hook) = LOCATION_HOOK.take() {
            _ = UnhookWinEvent(hook);
        }
        _ = DestroyWindow(host);
    }
    islands::clear();
    Ok(())
}
