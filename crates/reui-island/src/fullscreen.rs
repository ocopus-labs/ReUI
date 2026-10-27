//! "Something fullscreen is in front, hide": exclusive fullscreen games (Direct3D), borderless
//! fullscreen games and video players, browser video fullscreen (the browser's own window
//! grows to cover the display), and presentation mode. The classification is pure; the
//! inputs come from [`read`].

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, HMONITOR, MONITORINFO, MONITOR_DEFAULTTONULL};
use windows::Win32::UI::Shell::{SHQueryUserNotificationState, QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN};
use windows::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetWindowRect};

/// What `SHQueryUserNotificationState` says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notify {
    Normal,
    /// A fullscreen app is running ("busy").
    Busy,
    /// An exclusive-mode Direct3D app (most fullscreen games).
    D3d,
    /// Presentation mode (PowerPoint's slide show, "presentation settings").
    Presentation,
}

/// The foreground window, as far as fullscreen goes.
#[derive(Clone, Debug)]
pub struct Foreground {
    pub class: String,
    pub rect: RECT,
    /// On the island's display.
    pub on_monitor: bool,
}

/// Windows that cover the display but aren't apps: the desktop and the taskbar.
fn is_shell(class: &str) -> bool {
    matches!(class, "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd")
}

/// Whether the island hides. `enabled` is the `hide_in_fullscreen` setting; `monitor` is the
/// island's display.
pub fn hides(enabled: bool, notify: Notify, fg: Option<&Foreground>, monitor: &RECT) -> bool {
    if !enabled {
        return false;
    }
    // Exclusive fullscreen may have changed the display mode, so geometry can't be trusted;
    // a presentation wants a clean screen wherever the island is.
    if matches!(notify, Notify::D3d | Notify::Presentation) {
        return true;
    }
    let Some(fg) = fg.filter(|fg| fg.on_monitor && !is_shell(&fg.class)) else { return false };
    if notify == Notify::Busy {
        return true;
    }
    let r = &fg.rect;
    r.left <= monitor.left && r.top <= monitor.top && r.right >= monitor.right && r.bottom >= monitor.bottom
}

/// Reads the inputs for [`hides`] now.
pub fn read(fg: HWND, monitor: HMONITOR) -> (Notify, Option<Foreground>, RECT) {
    unsafe {
        let notify = match SHQueryUserNotificationState() {
            Ok(QUNS_RUNNING_D3D_FULL_SCREEN) => Notify::D3d,
            Ok(QUNS_PRESENTATION_MODE) => Notify::Presentation,
            Ok(QUNS_BUSY) => Notify::Busy,
            _ => Notify::Normal,
        };
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        _ = GetMonitorInfoW(monitor, &mut info);
        let foreground = (!fg.is_invalid()).then(|| {
            let mut class = [0u16; 64];
            let len = GetClassNameW(fg, &mut class) as usize;
            let mut rect = RECT::default();
            _ = GetWindowRect(fg, &mut rect);
            Foreground { class: String::from_utf16_lossy(&class[..len]), rect, on_monitor: MonitorFromWindow(fg, MONITOR_DEFAULTTONULL) == monitor }
        });
        (notify, foreground, info.rcMonitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: RECT = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };

    fn window(class: &str, rect: RECT, on_monitor: bool) -> Foreground {
        Foreground { class: class.into(), rect, on_monitor }
    }

    #[test]
    fn games_video_and_presentations_hide_the_island() {
        let normal = window("Chrome_WidgetWin_1", RECT { left: 100, top: 100, right: 1200, bottom: 900 }, true);
        // Browser video / borderless game / video player: the window covers the display
        // (often overhanging it a little).
        let covering = window("Chrome_WidgetWin_1", RECT { left: -8, top: -8, right: 1928, bottom: 1088 }, true);
        assert!(!hides(true, Notify::Normal, Some(&normal), &SCREEN));
        assert!(hides(true, Notify::Normal, Some(&covering), &SCREEN));
        // Exclusive fullscreen and presentations, whatever the window says.
        assert!(hides(true, Notify::D3d, Some(&normal), &SCREEN));
        assert!(hides(true, Notify::Presentation, None, &SCREEN));
        // "Busy" counts when the app in front is on the island's display.
        assert!(hides(true, Notify::Busy, Some(&normal), &SCREEN));
        assert!(!hides(true, Notify::Busy, Some(&window("X", SCREEN, false)), &SCREEN));
    }

    #[test]
    fn the_desktop_other_displays_and_the_setting_keep_it() {
        assert!(!hides(true, Notify::Normal, Some(&window("WorkerW", SCREEN, true)), &SCREEN));
        assert!(!hides(true, Notify::Busy, Some(&window("Progman", SCREEN, true)), &SCREEN));
        assert!(!hides(true, Notify::Normal, Some(&window("Game", SCREEN, false)), &SCREEN), "fullscreen on another display");
        assert!(!hides(true, Notify::Normal, None, &SCREEN));
        assert!(!hides(false, Notify::D3d, Some(&window("Game", SCREEN, true)), &SCREEN), "hide_in_fullscreen off");
    }
}
