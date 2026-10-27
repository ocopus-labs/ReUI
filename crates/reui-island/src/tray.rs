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

pub enum Command {
    Visibility(Visibility),
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
            _ => None,
        }
    }
}
