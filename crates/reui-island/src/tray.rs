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
const CMD_UPDATE: usize = 108;
const CMD_STATS: usize = 109;
const CMD_HIDE: usize = 110;
const CMD_CALENDAR: usize = 120;
const CMD_SHELF: usize = 111;
const CMD_SHELF_CLEAR: usize = 112;
/// Timer presets: this plus the preset's index.
const CMD_PRESET: usize = 200;
const CMD_ITEM: usize = 400;
/// A moment's own entries (Don't show these, Snooze): this plus the entry's index.
const CMD_MOMENT: usize = 500;

pub enum Command {
    Settings,
    /// Pause ReUI for an hour, or resume it when paused.
    TogglePause,
    /// Hide the island until asked again (as its shortcut does), or show it.
    ToggleHidden,
    /// (module, action)
    Module(&'static str, String),
    /// The `n`th of the moment's own entries.
    Moment(usize),
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
/// the pause entry resumes. `hidden`: the island is hidden (its shortcut), so the entry
/// shows it again: the tray is the one place a hidden island can be found. `moment`: the
/// right-clicked moment's own entries, first, since they're what the click was about.
///
/// Like [`item_menu`], the island takes the foreground only for the menu's life and hands
/// it back to the app that had it; returns that app too.
pub fn menu(hwnd: HWND, paused: bool, hidden: bool, moment: &[&str]) -> (Option<Command>, HWND) {
    let previous = unsafe { GetForegroundWindow() };
    let picked = show_menu(hwnd, paused, hidden, moment);
    unsafe {
        if !previous.is_invalid() && GetForegroundWindow() == hwnd {
            _ = SetForegroundWindow(previous);
        }
    }
    (picked, previous)
}

fn show_menu(hwnd: HWND, paused: bool, hidden: bool, moment: &[&str]) -> Option<Command> {
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        for (i, label) in moment.iter().enumerate() {
            let label: Vec<u16> = label.encode_utf16().chain([0]).collect();
            _ = AppendMenuW(menu, MF_STRING, CMD_MOMENT + i, PCWSTR(label.as_ptr()));
        }
        if !moment.is_empty() {
            _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        }
        _ = AppendMenuW(menu, MF_STRING, CMD_SETTINGS, w!("Settings"));
        let pause = if paused { w!("Resume ReUI") } else { w!("Pause ReUI for 1 hour") };
        _ = AppendMenuW(menu, MF_STRING, CMD_PAUSE, pause);
        let hide = if hidden { w!("Show the island") } else { w!("Hide the island") };
        _ = AppendMenuW(menu, MF_STRING, CMD_HIDE, hide);
        if let Some(version) = known_update() {
            let label: Vec<u16> = format!("Install update ({version})").encode_utf16().chain([0]).collect();
            _ = AppendMenuW(menu, MF_STRING, CMD_UPDATE, PCWSTR(label.as_ptr()));
        }
        _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        _ = AppendMenuW(menu, MF_STRING, CMD_CLIPBOARD, w!("Clipboard history"));
        _ = AppendMenuW(menu, MF_STRING, CMD_DEVICES, w!("Devices"));
        _ = AppendMenuW(menu, MF_STRING, CMD_STATS, w!("System stats"));
        _ = AppendMenuW(menu, MF_STRING, CMD_CALENDAR, w!("Calendar"));
        if let Ok(shelf) = CreatePopupMenu() {
            _ = AppendMenuW(shelf, MF_STRING, CMD_SHELF, w!("Open shelf"));
            _ = AppendMenuW(shelf, MF_STRING, CMD_SHELF_CLEAR, w!("Clear shelf"));
            _ = AppendMenuW(menu, MF_POPUP, shelf.0 as usize, w!("Shelf"));
        }
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
            CMD_HIDE => Some(Command::ToggleHidden),
            CMD_CLIPBOARD => Some(Command::Module("clipboard", "open".into())),
            CMD_DEVICES => Some(Command::Module("devices", "overview".into())),
            CMD_STATS => Some(Command::Module("stats", "open".into())),
            CMD_CALENDAR => Some(Command::Module("calendar", "open".into())),
            CMD_SHELF => Some(Command::Module("shelf", "open".into())),
            CMD_SHELF_CLEAR => Some(Command::Module("shelf", "clear".into())),
            CMD_NEW_TIMER => Some(Command::Module("timers", "launcher".into())),
            CMD_FOCUS => Some(Command::Module("timers", "start:focus".into())),
            CMD_STOPWATCH => Some(Command::Module("timers", "start:stopwatch".into())),
            CMD_UPDATE => Some(Command::Module("updates", "install".into())),
            n if (CMD_MOMENT..CMD_MOMENT + moment.len()).contains(&n) => Some(Command::Moment(n - CMD_MOMENT)),
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

/// The newer version the last update check found, if it can be installed from here: the
/// hub's updates module installs it (the same flow as the card's Install). Read when the
/// menu opens, so it costs nothing in between.
fn known_update() -> Option<String> {
    let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(std::path::PathBuf::from);
    let local = match var("REUI_CONFIG_DIR") {
        Some(root) => root.join("local"),
        None => var("LOCALAPPDATA")?.join("ReUI"),
    };
    let update = std::fs::read_to_string(local.join("update.json")).unwrap_or_default();
    let settings = crate::settings::Settings::dir().and_then(|dir| std::fs::read_to_string(dir.join("settings.json")).ok()).unwrap_or_default();
    update_entry(&update, &settings, env!("CARGO_PKG_VERSION"))
}

/// `update.json`'s `available` when it's newer than `current`, has an installer, and
/// "Check automatically" (`check_updates`, default on) hasn't been turned off.
fn update_entry(update_json: &str, settings_json: &str, current: &str) -> Option<String> {
    let settings: serde_json::Value = serde_json::from_str(settings_json).unwrap_or_default();
    if settings.get("check_updates").and_then(serde_json::Value::as_bool) == Some(false) {
        return None;
    }
    let update: serde_json::Value = serde_json::from_str(update_json).ok()?;
    let version = update.get("available")?.as_str()?;
    update.get("setup_url")?.as_str()?;
    reui_protocol::version::newer(version, current).then(|| version.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_update_entry_shows_for_a_newer_installable_version_while_checks_are_on() {
        let found = r#"{ "available": "0.2.0", "setup_url": "https://example.com/ReUI-Setup-0.2.0.exe" }"#;
        assert_eq!(update_entry(found, "", "0.1.0").as_deref(), Some("0.2.0"));
        assert_eq!(update_entry(found, r#"{ "check_updates": true }"#, "0.1.0").as_deref(), Some("0.2.0"));
        assert_eq!(update_entry(found, r#"{ "check_updates": false }"#, "0.1.0"), None);
        assert_eq!(update_entry(found, "", "0.2.0"), None);
        assert_eq!(update_entry(r#"{ "available": "0.2.0" }"#, "", "0.1.0"), None, "no installer");
        assert_eq!(update_entry(r#"{ "available": null }"#, "", "0.1.0"), None);
        assert_eq!(update_entry("", "", "0.1.0"), None);
    }
}
