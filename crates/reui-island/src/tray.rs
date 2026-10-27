//! Tray icon and the island's menu (the same menu opens from the tray and from a
//! right-click on the pill). The tray matters because a hidden island can't be right-clicked.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::UI::Shell::{Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::settings::Visibility;

/// Tray icon callback message.
pub const WM_TRAY: u32 = WM_APP + 2;
const ICON_ID: u32 = 1;
const CMD_QUIT: usize = 100;
const CMD_VISIBILITY: usize = 200;
const CMD_CLIPBOARD: usize = 300;
const CMD_ITEM: usize = 400;
const CMD_MUSIC: usize = 500;

/// Module commands in the menu: (module, action, label). Sent whether or not the module
/// has anything showing; clearing keeps pins, so it asks nothing.
const CLIPBOARD: [(&str, &str, &str); 3] = [
    ("clipboard", "pause", "Pause clipboard for 1 hour"),
    ("clipboard", "resume", "Resume clipboard"),
    ("clipboard", "clear", "Clear clipboard history"),
];

/// Music settings, kept by the media module (lyrics go to lrclib.net, so they're opt-in).
const MUSIC: [(&str, &str, &str); 5] = [
    ("media", "lyrics:on", "Lyrics on"),
    ("media", "lyrics:off", "Lyrics off"),
    ("media", "visualizer:real", "Visualizer: real"),
    ("media", "visualizer:simple", "Visualizer: simple"),
    ("media", "visualizer:off", "Visualizer: off"),
];

pub enum Command {
    Visibility(Visibility),
    /// (module, action)
    Module(&'static str, &'static str),
    Quit,
}

fn icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: ICON_ID,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        hIcon: unsafe { LoadIconW(None, IDI_APPLICATION).unwrap_or_default() },
        ..Default::default()
    };
    for (slot, unit) in data.szTip.iter_mut().zip("ReUI".encode_utf16()) {
        *slot = unit;
    }
    data
}

/// Adds the icon; call again when Explorer restarts (the `TaskbarCreated` message).
pub fn add(hwnd: HWND) {
    unsafe { _ = Shell_NotifyIconW(NIM_ADD, &icon_data(hwnd)) };
}

pub fn remove(hwnd: HWND) {
    unsafe { _ = Shell_NotifyIconW(NIM_DELETE, &icon_data(hwnd)) };
}

/// Explorer broadcasts this after it (re)starts; the icon has to be re-added then.
pub fn taskbar_created() -> u32 {
    unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) }
}

/// Does the tray callback mean "open the menu"?
pub fn wants_menu(lparam: LPARAM) -> bool {
    matches!(lparam.0 as u32, WM_LBUTTONUP | WM_RBUTTONUP)
}

/// Shows the menu at the pointer and returns what was picked.
pub fn menu(hwnd: HWND, current: Visibility) -> Option<Command> {
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        _ = AppendMenuW(menu, MF_STRING | MF_DISABLED | MF_GRAYED, 0, w!("Show the island"));
        for (i, (visibility, label)) in Visibility::ALL.iter().enumerate() {
            let checked = if *visibility == current { MF_CHECKED } else { MF_UNCHECKED };
            let label: Vec<u16> = format!("    {label}").encode_utf16().chain([0]).collect();
            _ = AppendMenuW(menu, MF_STRING | checked, CMD_VISIBILITY + i, PCWSTR(label.as_ptr()));
        }
        _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        for (i, (_, _, label)) in CLIPBOARD.iter().enumerate() {
            let label: Vec<u16> = label.encode_utf16().chain([0]).collect();
            _ = AppendMenuW(menu, MF_STRING, CMD_CLIPBOARD + i, PCWSTR(label.as_ptr()));
        }
        _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        _ = AppendMenuW(menu, MF_STRING | MF_DISABLED | MF_GRAYED, 0, w!("Music"));
        for (i, (_, _, label)) in MUSIC.iter().enumerate() {
            let label: Vec<u16> = format!("    {label}").encode_utf16().chain([0]).collect();
            _ = AppendMenuW(menu, MF_STRING, CMD_MUSIC + i, PCWSTR(label.as_ptr()));
        }
        _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        _ = AppendMenuW(menu, MF_STRING, CMD_QUIT, w!("Quit ReUI"));

        let mut pt = POINT::default();
        _ = GetCursorPos(&mut pt);
        // Required for the menu to close when the user clicks elsewhere.
        _ = SetForegroundWindow(hwnd);
        let picked = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, pt.x, pt.y, None, hwnd, None).0 as usize;
        _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        _ = DestroyMenu(menu);
        match picked {
            CMD_QUIT => Some(Command::Quit),
            n if (CMD_VISIBILITY..CMD_VISIBILITY + Visibility::ALL.len()).contains(&n) => Some(Command::Visibility(Visibility::ALL[n - CMD_VISIBILITY].0)),
            n if (CMD_CLIPBOARD..CMD_CLIPBOARD + CLIPBOARD.len()).contains(&n) => {
                let (module, action, _) = CLIPBOARD[n - CMD_CLIPBOARD];
                Some(Command::Module(module, action))
            }
            n if (CMD_MUSIC..CMD_MUSIC + MUSIC.len()).contains(&n) => {
                let (module, action, _) = MUSIC[n - CMD_MUSIC];
                Some(Command::Module(module, action))
            }
            _ => None,
        }
    }
}

/// A card's context menu at the pointer, entries (label, checked); returns the index of
/// the picked one.
///
/// A popup menu only closes on an outside click when its owner is the foreground window,
/// so the island takes the foreground for the menu's life and hands it straight back to
/// the app that had it (the paste target). Returns that app too, so the pill can ignore
/// the foreground change this causes.
pub fn item_menu(hwnd: HWND, labels: &[(&str, bool)]) -> (Option<usize>, HWND) {
    unsafe {
        let previous = GetForegroundWindow();
        let Ok(menu) = CreatePopupMenu() else { return (None, previous) };
        for (i, (label, checked)) in labels.iter().enumerate() {
            let label: Vec<u16> = label.encode_utf16().chain([0]).collect();
            let check = if *checked { MF_CHECKED } else { MF_UNCHECKED };
            _ = AppendMenuW(menu, MF_STRING | check, CMD_ITEM + i, PCWSTR(label.as_ptr()));
        }
        let mut pt = POINT::default();
        _ = GetCursorPos(&mut pt);
        _ = SetForegroundWindow(hwnd);
        let picked = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, pt.x, pt.y, None, hwnd, None).0 as usize;
        _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        _ = DestroyMenu(menu);
        if !previous.is_invalid() && GetForegroundWindow() == hwnd {
            _ = SetForegroundWindow(previous);
        }
        let picked = picked.checked_sub(CMD_ITEM).filter(|&i| i < labels.len());
        (picked, previous)
    }
}
