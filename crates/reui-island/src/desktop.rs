//! Whether the user is on the desktop: the foreground is the desktop window (they clicked the
//! wallpaper, or Win+D showed it) rather than an app. Used by the "Always show on the
//! desktop" setting. The taskbar, Start, search and the tray's pop-ups are shell surfaces
//! you pass through on the way somewhere, so they don't change the answer.

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::GetClassNameW;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Front {
    Desktop,
    App,
    /// Taskbar, Start, search, Alt+Tab, the tray overflow: keep what was there.
    Shell,
}

/// What's in front, by the foreground window's class name.
pub fn classify(class: &str) -> Front {
    match class {
        "Progman" | "WorkerW" => Front::Desktop,
        "" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" | "NotifyIconOverflowWindow" | "TopLevelWindowForOverflowXamlIsland" | "Windows.UI.Core.CoreWindow" | "XamlExplorerHostIslandWindow" | "MultitaskingViewFrame" | "ForegroundStaging" => Front::Shell,
        _ => Front::App,
    }
}

/// [`classify`] for a window; no window counts as the shell.
pub fn front(hwnd: HWND) -> Front {
    if hwnd.is_invalid() {
        return Front::Shell;
    }
    let mut name = [0u16; 64];
    let len = unsafe { GetClassNameW(hwnd, &mut name) };
    classify(&String::from_utf16_lossy(&name[..len.max(0) as usize]))
}

/// Whether the user is on the desktop after `front` came forward, given what it was.
pub fn on_desktop(front: Front, was: bool) -> bool {
    match front {
        Front::Desktop => true,
        Front::App => false,
        Front::Shell => was,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_desktop_apps_and_the_shell_are_told_apart() {
        assert_eq!(classify("Progman"), Front::Desktop);
        assert_eq!(classify("WorkerW"), Front::Desktop);
        assert_eq!(classify("Chrome_WidgetWin_1"), Front::App);
        assert_eq!(classify("CASCADIA_HOSTING_WINDOW_CLASS"), Front::App);
        assert_eq!(classify("Shell_TrayWnd"), Front::Shell);
        assert_eq!(classify("Windows.UI.Core.CoreWindow"), Front::Shell);
    }

    #[test]
    fn passing_through_the_shell_keeps_the_answer() {
        assert!(on_desktop(Front::Shell, true));
        assert!(!on_desktop(Front::Shell, false));
        assert!(on_desktop(Front::Desktop, false));
        assert!(!on_desktop(Front::App, true));
    }
}
