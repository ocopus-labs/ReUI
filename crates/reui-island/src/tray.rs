//! Tray icon and the island's menu (the same menu opens from a right-click on the tray icon
//! and on the pill). Short, per PRD 5.3: every setting lives in the Settings window, which a
//! left-click on the icon opens. The tray matters because a hidden island can't be clicked.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::UI::Shell::{Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW};
use windows::Win32::UI::WindowsAndMessaging::*;

/// Tray icon callback message.
pub const WM_TRAY: u32 = WM_APP + 2;
const ICON_ID: u32 = 1;
const CMD_SETTINGS: usize = 100;
const CMD_PAUSE: usize = 101;
const CMD_CLIPBOARD: usize = 102;
const CMD_DEVICES: usize = 103;
const CMD_QUIT: usize = 104;
const CMD_NEW_TIMER: usize = 105;
const CMD_FOCUS: usize = 106;
const CMD_STOPWATCH: usize = 107;
/// Timer presets: this plus the preset's index.
const CMD_PRESET: usize = 200;
const CMD_ITEM: usize = 400;

pub enum Command {
    Settings,
    /// Pause ReUI for an hour, or resume it when paused.
    TogglePause,
    /// (module, action)
    Module(&'static str, String),
    Quit,
}

/// The ReUI logo (linked in by `build.rs`) at the tray's size; shared, so loading it again
/// for each add or delete costs nothing.
fn logo() -> HICON {
    unsafe {
        let module = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default();
        let (w, h) = (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON));
        match LoadImageW(Some(module.into()), PCWSTR(reui_design::icon::ICON_ID as usize as *const u16), IMAGE_ICON, w, h, LR_SHARED) {
            Ok(icon) => HICON(icon.0),
            Err(_) => LoadIconW(None, IDI_APPLICATION).unwrap_or_default(),
        }
    }
}

fn icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: ICON_ID,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        hIcon: logo(),
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

/// What a tray callback asks for.
pub enum Click {
    /// Left click: the Settings window.
    Open,
    Menu,
}

pub fn click(lparam: LPARAM) -> Option<Click> {
    match lparam.0 as u32 {
        WM_LBUTTONUP => Some(Click::Open),
        WM_RBUTTONUP => Some(Click::Menu),
        _ => None,
    }
}

/// Opens `reui-settings.exe` from next to the island. It is single-instance: a second
/// launch brings the open window to the front, which it may do because we let it.
pub fn open_settings() {
    let Some(exe) = std::env::current_exe().ok().map(|exe| exe.with_file_name("reui-settings.exe")) else { return };
    unsafe { _ = AllowSetForegroundWindow(ASFW_ANY) };
    if let Err(e) = std::process::Command::new(&exe).spawn() {
        eprintln!("island: can't start {}: {e}", exe.display());
    }
}

/// Shows the menu at the pointer and returns what was picked. `paused`: ReUI is paused, so
/// the pause entry resumes.
pub fn menu(hwnd: HWND, paused: bool) -> Option<Command> {
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        _ = AppendMenuW(menu, MF_STRING, CMD_SETTINGS, w!("Settings"));
        let pause = if paused { w!("Resume ReUI") } else { w!("Pause ReUI for 1 hour") };
        _ = AppendMenuW(menu, MF_STRING, CMD_PAUSE, pause);
        _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        _ = AppendMenuW(menu, MF_STRING, CMD_CLIPBOARD, w!("Clipboard history"));
        _ = AppendMenuW(menu, MF_STRING, CMD_DEVICES, w!("Devices"));
        let presets = timer_presets();
        if let Ok(timer) = CreatePopupMenu() {
            _ = AppendMenuW(timer, MF_STRING, CMD_NEW_TIMER, w!("New timer…"));
            _ = AppendMenuW(timer, MF_SEPARATOR, 0, None);
            for (i, minutes) in presets.iter().enumerate() {
                let label: Vec<u16> = format!("{minutes} min").encode_utf16().chain([0]).collect();
                _ = AppendMenuW(timer, MF_STRING, CMD_PRESET + i, PCWSTR(label.as_ptr()));
            }
            _ = AppendMenuW(timer, MF_SEPARATOR, 0, None);
            _ = AppendMenuW(timer, MF_STRING, CMD_FOCUS, w!("Focus session"));
            _ = AppendMenuW(timer, MF_STRING, CMD_STOPWATCH, w!("Stopwatch"));
            _ = AppendMenuW(menu, MF_POPUP, timer.0 as usize, w!("Timer"));
        }
        _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        _ = AppendMenuW(menu, MF_STRING, CMD_QUIT, w!("Quit ReUI"));
        _ = SetMenuDefaultItem(menu, CMD_SETTINGS as u32, 0);

        let mut pt = POINT::default();
        _ = GetCursorPos(&mut pt);
        // Required for the menu to close when the user clicks elsewhere.
        _ = SetForegroundWindow(hwnd);
        let picked = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, pt.x, pt.y, None, hwnd, None).0 as usize;
        _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        _ = DestroyMenu(menu);
        match picked {
            CMD_SETTINGS => Some(Command::Settings),
            CMD_PAUSE => Some(Command::TogglePause),
            CMD_CLIPBOARD => Some(Command::Module("clipboard", "open".into())),
            CMD_DEVICES => Some(Command::Module("devices", "overview".into())),
            CMD_NEW_TIMER => Some(Command::Module("timers", "launcher".into())),
            CMD_FOCUS => Some(Command::Module("timers", "start:focus".into())),
            CMD_STOPWATCH => Some(Command::Module("timers", "start:stopwatch".into())),
            n if (CMD_PRESET..CMD_PRESET + presets.len()).contains(&n) => Some(Command::Module("timers", format!("start:{}m", presets[n - CMD_PRESET]))),
            CMD_QUIT => Some(Command::Quit),
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

/// The timer presets in minutes, from the Timers settings (`timers.json`), as Settings
/// writes them; the defaults when there's no file yet.
fn timer_presets() -> Vec<u32> {
    let file = crate::settings::Settings::dir().map(|dir| dir.join("timers.json"));
    let text = file.and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
    let parsed: Option<Vec<u32>> = serde_json::from_str::<serde_json::Value>(&text).ok().and_then(|v| {
        let list: Vec<u32> = v.get("presets")?.as_array()?.iter().filter_map(|m| m.as_u64()).filter(|m| (1..=1440).contains(m)).map(|m| m as u32).take(6).collect();
        (!list.is_empty()).then_some(list)
    });
    parsed.unwrap_or_else(|| vec![1, 5, 10, 25])
}
