//! ReUI island: the dynamic-island surface. Renders whatever the hub says is in front and
//! sends the user's actions back. Normally launched by `reui-hub`; it reconnects on its
//! own if the hub restarts. Rest the pointer on it to peek, click to expand; right-click it
//! (or the tray icon) for the short menu; a left-click on the tray icon opens Settings.

#![windows_subsystem = "windows"]

mod desktop;
mod app_rules;
mod drag_guard;
mod dragdrop;
mod features;
mod fullscreen;
mod images;
mod menus;
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

use std::cell::{Cell, RefCell};

use windows::core::{w, Result, BOOL};
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::WinRT::*;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, ReleaseCapture, SetCapture, VK_MENU};
use windows::Win32::UI::WindowsAndMessaging::*;


use crate::features::clipboard_keys;
use reui_design::look::Look;

use crate::pill::Pill;
use crate::place::{MonitorChoice, Placement};
use crate::settings::Settings;

/// Re-reads settings.json this long after the folder last changed. Outside the pill's
/// timer ids (which start at 1).
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

const TIMER_SETTINGS: usize = 100;
const SETTINGS_SETTLE_MS: u32 = 150;
/// Re-place the island (display, DPI, work area) once things settle after a resume, a
/// display or DPI change, or an Explorer restart; and once more a little later after a
/// resume, when displays are often still waking.
const TIMER_PLACE: usize = 101;
const TIMER_PLACE_LATE: usize = 102;
/// Fullscreen is checked again this long after the foreground app moved or changed: the
/// shell's own fullscreen state can lag the window by a moment.
const TIMER_FULLSCREEN: usize = 103;
const FULLSCREEN_SETTLE_MS: u32 = 300;
const PLACE_SETTLE_MS: u32 = 400;
const PLACE_LATE_MS: u32 = 3000;
/// The pill's own timer ids, killed before it's rebuilt.
const PILL_TIMERS: std::ops::RangeInclusive<usize> = 1..=30;

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
    /// Location changes of the foreground app's windows (a browser video going fullscreen
    /// keeps its window), re-registered for each new foreground process.
    static LOCATION_HOOK: Cell<Option<HWINEVENTHOOK>> = const { Cell::new(None) };
    static ISLAND: Cell<isize> = const { Cell::new(0) };
}

/// Checks fullscreen again once the shell has caught up (see [`TIMER_FULLSCREEN`]).
fn recheck_fullscreen_soon() {
    let island = HWND(ISLAND.get() as *mut _);
    if !island.is_invalid() {
        unsafe { SetTimer(Some(island), TIMER_FULLSCREEN, FULLSCREEN_SETTLE_MS, None) };
    }
}

fn schedule_place(hwnd: HWND, late: bool) {
    unsafe {
        SetTimer(Some(hwnd), TIMER_PLACE, PLACE_SETTLE_MS, None);
        if late {
            SetTimer(Some(hwnd), TIMER_PLACE_LATE, PLACE_LATE_MS, None);
        }
    }
}

/// The look for `settings` as Windows is now (its app mode for Auto, its accent).
fn look_for(settings: &Settings) -> Look {
    Look::new(&settings.appearance, theme::apps_light(), theme::accent())
}

/// Windows' colours changed: rebuild only if the look comes out different (Auto in another
/// app mode, the Windows accent changed and is in use).
fn colors_changed(hwnd: HWND) {
    let mut current = None;
    with_pill(|pill| current = Some(*pill.look().0));
    if current.is_some_and(|look| look != look_for(&Settings::load())) {
        place(hwnd, true);
    }
}

/// Puts the island where the `monitor` setting says, at that display's DPI, and back on
/// top. A display with another scale needs the pill rebuilt for it, and so does a new
/// look (`relook`).
fn place(hwnd: HWND, relook: bool) {
    let mut choice = MonitorChoice::default();
    let mut look = Look::default();
    with_pill(|pill| {
        choice = pill.monitor_choice().clone();
        look = *pill.look().0;
    });
    if relook {
        look = look_for(&Settings::load());
    }
    let Some(placement) = place::current(&choice, &look) else { return };
    let mut moved = !relook;
    if moved {
        with_pill(|pill| moved = pill.move_to(&placement));
    }
    if !moved {
        rebuild(hwnd, &placement);
    }
}

/// In "cursor" mode, an idle island moves to the pointer's display before an activity
/// shows up on it.
fn follow_cursor(hwnd: HWND) {
    let mut follow = false;
    with_pill(|pill| follow = *pill.monitor_choice() == MonitorChoice::Cursor && pill.idle());
    if follow {
        place(hwnd, false);
    }
}

/// A new pill for another scale (per-monitor DPI) or look: the old one hands over its hub
/// link, snapshot and GPU objects (so a style switch takes milliseconds, not the ~140 ms a
/// new device and fonts cost), the window is resized, and the new one picks up where it
/// left off.
fn rebuild(hwnd: HWND, placement: &Placement) {
    let Some(mut old) = PILL.with(|cell| cell.try_borrow_mut().ok().and_then(|mut slot| slot.take())) else { return };
    eprintln!("island: rebuilding for scale {}", placement.scale);
    let (to_hub, events, snapshot) = old.take_link();
    let places = old.places();
    let gpu = old.into_gpu();
    unsafe {
        for id in PILL_TIMERS {
            _ = KillTimer(Some(hwnd), id);
        }
        let w = placement.window;
        _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), w.left, w.top, w.right - w.left, w.bottom - w.top, SWP_NOACTIVATE | SWP_NOOWNERZORDER);
    }
    let settings = Settings::load();
    let look = look_for(&settings);
    match Pill::new(hwnd, placement.scale, placement.window, placement.monitor, !animations_enabled(), settings.visibility, settings.appearance, look, to_hub, events, Some(gpu)) {
        Ok(mut pill) => {
            pill.apply_settings(settings);
            pill.restore(snapshot, places);
            pill.recheck_fullscreen();
            PILL.with(|cell| *cell.borrow_mut() = Some(pill));
        }
        Err(e) => {
            // Without a pill there's nothing to show; the hub starts a fresh island.
            eprintln!("island: can't rebuild the pill: {e}");
            unsafe { PostQuitMessage(1) };
        }
    }
}

fn press_drag(lparam: LPARAM) -> bool {
    let (x, y) = client_point(lparam);
    let mut pressed = false;
    with_pill(|pill| pressed = pill.press_drag(x, y));
    pressed
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
        let pt = (*(lparam.0 as *const MSLLHOOKSTRUCT)).pt;
        match wparam.0 as u32 {
            WM_MOUSEMOVE => with_pill(|pill| pill.on_mouse_move(pt)),
            WM_LBUTTONDOWN => with_pill(|pill| pill.on_mouse_down(pt, drag_guard::LEFT)),
            WM_RBUTTONDOWN => with_pill(|pill| pill.on_mouse_down(pt, drag_guard::RIGHT)),
            WM_MBUTTONDOWN => with_pill(|pill| pill.on_mouse_down(pt, drag_guard::MIDDLE)),
            WM_LBUTTONUP => with_pill(|pill| pill.on_mouse_up(pt, drag_guard::LEFT)),
            WM_RBUTTONUP => with_pill(|pill| pill.on_mouse_up(pt, drag_guard::RIGHT)),
            WM_MBUTTONUP => with_pill(|pill| pill.on_mouse_up(pt, drag_guard::MIDDLE)),
            _ => {}
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

unsafe extern "system" fn foreground_hook(_: HWINEVENTHOOK, _: u32, hwnd: HWND, _: i32, _: i32, _: u32, _: u32) {
    with_pill(|pill| pill.on_foreground(hwnd));
    watch_location(hwnd);
    recheck_fullscreen_soon();
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
        with_pill(|pill| pill.recheck_fullscreen());
        recheck_fullscreen_soon();
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        ipc::WM_HUB => {
            follow_cursor(hwnd);
            with_pill(|pill| pill.drain_hub());
            LRESULT(0)
        }
        place::WM_PLACE => {
            place(hwnd, false);
            LRESULT(0)
        }
        pill::WM_LOOK => {
            place(hwnd, true);
            LRESULT(0)
        }
        // Displays added, removed or re-arranged; resolution or scale changed; the taskbar
        // moved (work area).
        WM_DISPLAYCHANGE | WM_DPICHANGED => {
            schedule_place(hwnd, false);
            LRESULT(0)
        }
        // Light or dark app mode, or the accent: Auto and the Windows accent follow.
        WM_SETTINGCHANGE if theme::is_color_change(lparam) => {
            colors_changed(hwnd);
            LRESULT(0)
        }
        WM_DWMCOLORIZATIONCOLORCHANGED => {
            colors_changed(hwnd);
            LRESULT(0)
        }
        WM_SETTINGCHANGE if wparam.0 as u32 == SPI_SETWORKAREA.0 => {
            schedule_place(hwnd, false);
            LRESULT(0)
        }
        WM_POWERBROADCAST => {
            if matches!(wparam.0 as u32, PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND) {
                schedule_place(hwnd, true);
            }
            LRESULT(1)
        }
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        // Debug builds only: a pointer position (screen pixels, lparam as for WM_MOUSEMOVE)
        // posted by a test script, so hover and peek can be checked without moving the
        // user's real mouse. Release builds never read it.
        #[cfg(debug_assertions)]
        WM_TEST_POINTER => {
            let (x, y) = client_point(lparam);
            with_pill(|pill| pill.on_mouse_move(POINT { x: x as i32, y: y as i32 }));
            LRESULT(0)
        }
        #[cfg(debug_assertions)]
        WM_TEST_DROP => {
            with_pill(|pill| pill.test_drag(wparam.0));
            LRESULT(0)
        }
        #[cfg(debug_assertions)]
        WM_TEST_FOREGROUND => {
            with_pill(|pill| pill.test_front_app(wparam.0 as u32));
            LRESULT(0)
        }
        #[cfg(debug_assertions)]
        WM_TEST_QUIET => {
            let (x, y) = client_point(lparam);
            with_pill(|pill| pill.test_quiet(x, y, wparam.0));
            LRESULT(0)
        }
        clipboard_keys::WM_KEYS => {
            with_pill(|pill| pill.on_keys());
            LRESULT(0)
        }
        // A press on a bar in the open card drags it, with the mouse captured so the drag
        // follows the pointer off the island; anything else acts on release.
        // A press on something that can be dragged out (a held file's card, the shelf's
        // pill) is watched with the mouse captured; moving past the drag distance starts
        // Windows' drag, outside the pill's borrow (the island's own drop target and
        // timers run during it).
        WM_LBUTTONDOWN if press_drag(lparam) => {
            SetCapture(hwnd);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let (x, y) = client_point(lparam);
            let mut out = None;
            with_pill(|pill| out = pill.drag_out_due(x, y));
            if let Some(drag) = out {
                _ = ReleaseCapture();
                dragdrop::drag_out(hwnd, &drag);
            }
            LRESULT(0)
        }
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
            let mut pressed = false;
            with_pill(|pill| pressed = pill.release_press());
            if pressed {
                _ = ReleaseCapture();
            }
            let shift = wparam.0 & 0x0004 != 0; // MK_SHIFT
            // The island never has focus, so Alt is read from the hardware state.
            let alt = GetAsyncKeyState(VK_MENU.0 as i32) as u16 & 0x8000 != 0;
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
                menus::card_menu(hwnd, entries);
            } else {
                with_pill(|pill| pill.on_click(x, y, shift, alt));
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
            menus::right_click(hwnd, x, y);
            LRESULT(0)
        }
        tray::WM_TRAY => {
            match tray::click(lparam) {
                Some(tray::Click::Open) => tray::open_settings(),
                Some(tray::Click::Menu) => menus::open_menu(hwnd, Vec::new()),
                None => {}
            }
            LRESULT(0)
        }
        // Coalesced: one folder change often arrives as several notifications.
        settings::WM_SETTINGS_CHANGED => {
            SetTimer(Some(hwnd), TIMER_SETTINGS, SETTINGS_SETTLE_MS, None);
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_FULLSCREEN => {
            _ = KillTimer(Some(hwnd), TIMER_FULLSCREEN);
            with_pill(|pill| pill.recheck_fullscreen());
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_PLACE || wparam.0 == TIMER_PLACE_LATE => {
            _ = KillTimer(Some(hwnd), wparam.0);
            place(hwnd, false);
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_SETTINGS => {
            _ = KillTimer(Some(hwnd), TIMER_SETTINGS);
            let settings = Settings::load();
            with_pill(|pill| pill.apply_settings(settings));
            LRESULT(0)
        }
        WM_TIMER => {
            with_pill(|pill| pill.on_timer(wparam.0));
            LRESULT(0)
        }
        WM_DESTROY => {
            dragdrop::unregister(hwnd);
            tray::remove(hwnd);
            PostQuitMessage(0);
            LRESULT(0)
        }
        // Explorer restarted: the tray icon is gone and the taskbar may sit elsewhere.
        _ if msg != 0 && msg == TASKBAR_CREATED.get() => {
            tray::add(hwnd);
            schedule_place(hwnd, false);
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

    // The chosen display (primary by default), top centre, at its DPI.
    let settings = Settings::load();
    let look = look_for(&settings);
    let Some(Placement { monitor, scale, window }) = place::current(&settings.monitor, &look) else {
        return Err(windows::core::Error::from_thread());
    };

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
        let (width, height) = (window.right - window.left, window.bottom - window.top);
        let hwnd = CreateWindowExW(ex, w!("ReUI.Island"), w!("ReUI"), WS_POPUP, window.left, window.top, width, height, None, None, Some(instance.into()), None)?;
        SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA)?;
        hwnd
    };

    ISLAND.set(hwnd.0 as isize);
    let link = ipc::spawn(hwnd);
    let mut pill = Pill::new(hwnd, scale, window, monitor, !animations_enabled(), settings.visibility, settings.appearance, look, link.to_hub, link.events, None)?;
    pill.apply_settings(settings);
    settings::watch(hwnd);
    PILL.with(|cell| *cell.borrow_mut() = Some(pill));
    // Things dragged onto the island go to the file shelf (its drop target answers only
    // near the island, `pill/drop.rs`).
    dragdrop::register(hwnd, Box::new(|event| {
        let mut taken = false;
        with_pill(|pill| taken = pill.on_drop_event(event));
        taken
    }));
    unsafe { _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
    watch_location(unsafe { GetForegroundWindow() });
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
        if let Some(hook) = LOCATION_HOOK.take() {
            _ = UnhookWinEvent(hook);
        }
    }
    PILL.with(|cell| cell.borrow_mut().take());
    Ok(())
}
