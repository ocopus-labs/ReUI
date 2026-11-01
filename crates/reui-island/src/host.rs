//! The host: a hidden window for what there's one of, however many islands there are
//! (docs/PRD-all-displays.md §5.3). It gets the hub link's notifications, the tray icon's
//! clicks, the settings folder watcher's, and Windows' broadcasts (displays, the work
//! area, colours, power, Explorer restarting), and keeps the islands in step. Island
//! windows come and go with displays; this one stays for the life of the process.

use std::cell::Cell;

use windows::core::{w, Result};
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::{ipc, islands, menus, settings, theme, tray};

/// Re-reads settings.json this long after the folder last changed.
const TIMER_SETTINGS: usize = 100;
const SETTINGS_SETTLE_MS: u32 = 150;
/// Re-place the islands (displays, DPI, work area) once things settle after a resume, a
/// display or DPI change, or an Explorer restart; and once more a little later after a
/// resume, when displays are often still waking.
const TIMER_PLACE: usize = 101;
const TIMER_PLACE_LATE: usize = 102;
const PLACE_SETTLE_MS: u32 = 400;
const PLACE_LATE_MS: u32 = 3000;
/// Fullscreen is checked again this long after the foreground app moved or changed: the
/// shell's own fullscreen state can lag the window by a moment.
const TIMER_FULLSCREEN: usize = 103;
const FULLSCREEN_SETTLE_MS: u32 = 300;

thread_local! {
    static HOST: Cell<isize> = const { Cell::new(0) };
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
}

pub fn hwnd() -> HWND {
    HWND(HOST.get() as *mut _)
}

/// Creates the host window (never shown) and puts the tray icon on it.
pub fn create() -> Result<HWND> {
    let hwnd = unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSEXW { cbSize: size_of::<WNDCLASSEXW>() as u32, lpfnWndProc: Some(hostproc), hInstance: instance.into(), lpszClassName: w!("ReUI.Host"), ..Default::default() };
        if RegisterClassExW(&class) == 0 {
            return Err(windows::core::Error::from_thread());
        }
        // A hidden top-level window: it gets broadcasts (WM_DISPLAYCHANGE, TaskbarCreated)
        // that a message-only window wouldn't.
        CreateWindowExW(WS_EX_TOOLWINDOW, w!("ReUI.Host"), w!("ReUI host"), WS_POPUP, 0, 0, 0, 0, None, None, Some(instance.into()), None)?
    };
    HOST.set(hwnd.0 as isize);
    TASKBAR_CREATED.set(tray::taskbar_created());
    tray::add(hwnd);
    Ok(hwnd)
}

/// Re-place the islands once things settle (and again a little later after a resume).
pub fn schedule_place(late: bool) {
    let host = hwnd();
    unsafe {
        SetTimer(Some(host), TIMER_PLACE, PLACE_SETTLE_MS, None);
        if late {
            SetTimer(Some(host), TIMER_PLACE_LATE, PLACE_LATE_MS, None);
        }
    }
}

/// Checks fullscreen again once the shell has caught up.
pub fn recheck_fullscreen_soon() {
    let host = hwnd();
    if !host.is_invalid() {
        unsafe { SetTimer(Some(host), TIMER_FULLSCREEN, FULLSCREEN_SETTLE_MS, None) };
    }
}

unsafe extern "system" fn hostproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        ipc::WM_HUB => {
            islands::follow_cursor();
            islands::drain_hub();
            LRESULT(0)
        }
        // Displays added, removed or re-arranged; resolution or scale changed.
        WM_DISPLAYCHANGE => {
            schedule_place(false);
            LRESULT(0)
        }
        // Light or dark app mode, or the accent: Auto and the Windows accent follow.
        WM_SETTINGCHANGE if theme::is_color_change(lparam) => {
            islands::colors_changed();
            LRESULT(0)
        }
        WM_DWMCOLORIZATIONCOLORCHANGED => {
            islands::colors_changed();
            LRESULT(0)
        }
        // The taskbar moved or resized (work area).
        WM_SETTINGCHANGE if wparam.0 as u32 == SPI_SETWORKAREA.0 => {
            schedule_place(false);
            LRESULT(0)
        }
        WM_POWERBROADCAST => {
            if matches!(wparam.0 as u32, PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND) {
                schedule_place(true);
            }
            LRESULT(1)
        }
        tray::WM_TRAY => {
            match tray::click(lparam) {
                Some(tray::Click::Open) => tray::open_settings(),
                // The island's menu, from the island the pointer is on.
                Some(tray::Click::Menu) => {
                    if let Some(lead) = islands::lead() {
                        menus::open_menu(lead, Vec::new());
                    }
                }
                None => {}
            }
            LRESULT(0)
        }
        // Coalesced: one folder change often arrives as several notifications.
        settings::WM_SETTINGS_CHANGED => {
            SetTimer(Some(hwnd), TIMER_SETTINGS, SETTINGS_SETTLE_MS, None);
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_SETTINGS => {
            _ = KillTimer(Some(hwnd), TIMER_SETTINGS);
            islands::sync_settings();
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_FULLSCREEN => {
            _ = KillTimer(Some(hwnd), TIMER_FULLSCREEN);
            islands::each_pill(|pill| pill.recheck_fullscreen());
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_PLACE || wparam.0 == TIMER_PLACE_LATE => {
            _ = KillTimer(Some(hwnd), wparam.0);
            islands::sync(false);
            LRESULT(0)
        }
        WM_DESTROY => {
            tray::remove(hwnd);
            LRESULT(0)
        }
        // Explorer restarted: the tray icon is gone and the taskbar may sit elsewhere.
        _ if msg != 0 && msg == TASKBAR_CREATED.get() => {
            tray::add(hwnd);
            schedule_place(false);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
