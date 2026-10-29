//! The Windows side of the settings: the startup entry, opening folders and links, the
//! Windows build, running apps, folder sizes.

use std::path::Path;

use windows::core::{w, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_FILE_NOT_FOUND, HWND, LPARAM, WIN32_ERROR};
use windows::Win32::System::Registry::*;
use windows::Win32::System::Threading::{GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::BOOL;

use crate::paths::Paths;

const VALUE: &str = "ReUI";
const APPROVED_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
const SANDBOX_APPROVED_KEY: &str = r"Software\ReUI\SettingsSandbox\StartupApproved";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Startup {
    Off,
    On,
    /// The entry starts a different `reui-hub.exe` (another copy of ReUI).
    Elsewhere(String),
    /// The entry is there but switched off in Task Manager (P-2).
    DisabledInTaskManager,
}

fn approved_key(paths: &Paths) -> &'static str {
    if paths.sandbox {
        SANDBOX_APPROVED_KEY
    } else {
        APPROVED_KEY
    }
}

/// The real state of the startup entry.
pub fn startup(paths: &Paths) -> Startup {
    let Some(command) = read_string(HKEY_CURRENT_USER, paths.run_key(), VALUE) else { return Startup::Off };
    // Task Manager keeps its switch separately: an odd first byte means disabled.
    if read_binary(HKEY_CURRENT_USER, approved_key(paths), VALUE).and_then(|b| b.first().copied()).is_some_and(|b| b & 1 == 1) {
        return Startup::DisabledInTaskManager;
    }
    let program = command_program(&command);
    if same_path(&program, &paths.hub_exe().to_string_lossy()) {
        Startup::On
    } else {
        Startup::Elsewhere(program)
    }
}

/// Adds or removes the entry for the hub next to this exe.
pub fn set_startup(paths: &Paths, on: bool) -> Result<(), String> {
    let key = HSTRING::from(paths.run_key());
    let error = unsafe {
        if on {
            let command: Vec<u16> = format!("\"{}\"", paths.hub_exe().display()).encode_utf16().chain([0]).collect();
            RegSetKeyValueW(HKEY_CURRENT_USER, &key, &HSTRING::from(VALUE), REG_SZ.0, Some(command.as_ptr().cast()), (command.len() * 2) as u32)
        } else {
            match RegDeleteKeyValueW(HKEY_CURRENT_USER, &key, &HSTRING::from(VALUE)) {
                ERROR_FILE_NOT_FOUND => WIN32_ERROR(0), // already gone
                e => e,
            }
        }
    };
    if error.is_ok() {
        Ok(())
    } else {
        Err(format!("can't change the startup entry ({})", windows::core::Error::from(error.to_hresult()).message()))
    }
}

/// `"C:\x\reui-hub.exe" --flag` → `C:\x\reui-hub.exe`.
pub fn command_program(command: &str) -> String {
    let c = command.trim();
    match c.strip_prefix('"') {
        Some(rest) => rest.split('"').next().unwrap_or(rest).to_string(),
        None => c.split(" --").next().unwrap_or(c).trim().to_string(),
    }
}

fn same_path(a: &str, b: &str) -> bool {
    let norm = |p: &str| p.replace('/', "\\").trim_end_matches('\\').to_lowercase();
    norm(a) == norm(b)
}

fn read_string(root: HKEY, key: &str, value: &str) -> Option<String> {
    let mut buffer = [0u16; 1024];
    let mut len = (buffer.len() * 2) as u32;
    let error = unsafe { RegGetValueW(root, &HSTRING::from(key), &HSTRING::from(value), RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ, None, Some(buffer.as_mut_ptr().cast()), Some(&mut len)) };
    error.is_ok().then(|| String::from_utf16_lossy(&buffer[..(len as usize / 2).saturating_sub(1)]))
}

fn read_binary(root: HKEY, key: &str, value: &str) -> Option<Vec<u8>> {
    let mut buffer = [0u8; 64];
    let mut len = buffer.len() as u32;
    let error = unsafe { RegGetValueW(root, &HSTRING::from(key), &HSTRING::from(value), RRF_RT_REG_BINARY, None, Some(buffer.as_mut_ptr().cast()), Some(&mut len)) };
    error.is_ok().then(|| buffer[..len as usize].to_vec())
}

fn read_dword(root: HKEY, key: &str, value: &str) -> Option<u32> {
    let mut data = 0u32;
    let mut len = 4u32;
    let error = unsafe { RegGetValueW(root, &HSTRING::from(key), &HSTRING::from(value), RRF_RT_REG_DWORD, None, Some((&mut data as *mut u32).cast()), Some(&mut len)) };
    error.is_ok().then_some(data)
}

/// "26200.6899", for bug reports.
/// Windows' app mode is light (Settings > Personalization > Colors): what the island's
/// Auto theme follows. Light when the value is missing, as Windows treats it.
pub fn apps_light() -> bool {
    read_dword(HKEY_CURRENT_USER, r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize", "AppsUseLightTheme").is_none_or(|v| v != 0)
}

/// The Windows accent colour (`AccentColor` is 0xAABBGGRR), for the island's Windows accent.
pub fn accent_color() -> Option<reui_design::Color> {
    let abgr = read_dword(HKEY_CURRENT_USER, r"Software\Microsoft\Windows\DWM", "AccentColor")?;
    Some(reui_design::Color::from_rgb8((abgr & 0xFF) as u8, (abgr >> 8 & 0xFF) as u8, (abgr >> 16 & 0xFF) as u8))
}

pub fn windows_build() -> String {
    let key = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
    let build = read_string(HKEY_LOCAL_MACHINE, key, "CurrentBuild").unwrap_or_else(|| "?".into());
    match read_dword(HKEY_LOCAL_MACHINE, key, "UBR") {
        Some(ubr) => format!("{build}.{ubr}"),
        None => build,
    }
}

/// Opens a folder, file or link with its default handler.
pub fn open(target: &str) {
    unsafe { ShellExecuteW(None, w!("open"), &HSTRING::from(target), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
}

/// Opens a folder, creating it first so the click never does nothing.
pub fn open_folder(path: &Path) {
    _ = std::fs::create_dir_all(path);
    open(&path.to_string_lossy());
}

/// A settings file in Notepad (an invalid one has to be fixed by hand).
pub fn edit_file(path: &Path) {
    unsafe { ShellExecuteW(None, w!("open"), w!("notepad.exe"), &HSTRING::from(format!("\"{}\"", path.display())), PCWSTR::null(), SW_SHOWNORMAL) };
}

/// Task Manager on its Startup apps page.
pub fn open_task_manager() {
    unsafe { ShellExecuteW(None, w!("open"), w!("taskmgr.exe"), w!("/0 /startup"), PCWSTR::null(), SW_SHOWNORMAL) };
}

/// Total size of the files under `path` (0 if it doesn't exist).
pub fn folder_size(path: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else { return 0 };
    entries
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => folder_size(&e.path()),
            Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
            _ => 0,
        })
        .sum()
}

/// Executable names of apps with a window on screen, lower case, sorted, without ReUI's.
pub fn running_apps() -> Vec<String> {
    unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let pids = &mut *(lparam.0 as *mut Vec<u32>);
        if IsWindowVisible(hwnd).as_bool() && GetWindow(hwnd, GW_OWNER).is_err() && GetWindowTextLengthW(hwnd) > 0 {
            let mut pid = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if !pids.contains(&pid) {
                pids.push(pid);
            }
        }
        BOOL(1)
    }
    let mut pids: Vec<u32> = Vec::new();
    unsafe { _ = EnumWindows(Some(visit), LPARAM(&mut pids as *mut _ as isize)) };
    let me = unsafe { GetCurrentProcessId() };
    let mut names: Vec<String> = pids.into_iter().filter(|&pid| pid != me).filter_map(exe_name).filter(|n| !n.starts_with("reui-")).collect();
    names.sort();
    names.dedup();
    names
}

pub fn exe_name(pid: u32) -> Option<String> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buffer = [0u16; 1024];
        let mut len = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &mut len).is_ok();
        _ = CloseHandle(process);
        let path = String::from_utf16_lossy(&buffer[..len as usize]);
        ok.then(|| path.rsplit('\\').next().unwrap_or(&path).to_lowercase())
    }
}

/// 1_234_567 → "1.2 MB".
pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if n < 1000 {
        return format!("{n} bytes");
    }
    let mut value = n as f64 / 1000.0;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    if value < 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

/// Minutes left of a pause, rounded up: "42 min", "1 h".
pub fn time_left(ms: u64) -> String {
    let minutes = ms.div_ceil(60_000);
    if minutes >= 60 {
        format!("{} h", minutes / 60)
    } else {
        format!("{minutes} min")
    }
}

/// The displays, as (device name, label): `\\.\DISPLAY2` → "Display 2".
pub fn monitors() -> Vec<(String, String)> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW};
    unsafe extern "system" fn visit(monitor: HMONITOR, _: HDC, _: *mut RECT, lparam: LPARAM) -> BOOL {
        let names = &mut *(lparam.0 as *mut Vec<String>);
        let mut info = MONITORINFOEXW { monitorInfo: MONITORINFO { cbSize: size_of::<MONITORINFOEXW>() as u32, ..Default::default() }, ..Default::default() };
        if GetMonitorInfoW(monitor, &mut info.monitorInfo).as_bool() {
            let len = info.szDevice.iter().position(|&c| c == 0).unwrap_or(info.szDevice.len());
            names.push(String::from_utf16_lossy(&info.szDevice[..len]));
        }
        BOOL(1)
    }
    let mut names: Vec<String> = Vec::new();
    unsafe { _ = EnumDisplayMonitors(None, None, Some(visit), LPARAM(&mut names as *mut _ as isize)) };
    names.sort();
    names
        .into_iter()
        .map(|n| {
            let label = monitor_label(&n);
            (n, label)
        })
        .collect()
}

/// "Display 2" for `\\.\DISPLAY2`; the name itself otherwise.
pub fn monitor_label(device: &str) -> String {
    match device.trim_start_matches(['\\', '.']).strip_prefix("DISPLAY") {
        Some(n) if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => format!("Display {n}"),
        _ => device.to_string(),
    }
}

/// Percent-encodes a query value (RFC 3986 unreserved characters stay).
pub fn url_encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_commands_name_their_program() {
        assert_eq!(command_program(r#""C:\Apps\ReUI\reui-hub.exe" --demo"#), r"C:\Apps\ReUI\reui-hub.exe");
        assert_eq!(command_program(r"C:\Apps\ReUI\reui-hub.exe --demo"), r"C:\Apps\ReUI\reui-hub.exe");
        assert!(same_path(r"c:/apps/reui/REUI-HUB.exe", r"C:\Apps\ReUI\reui-hub.exe"));
    }

    #[test]
    fn sizes_and_times_read_naturally() {
        assert_eq!(bytes(512), "512 bytes");
        assert_eq!(monitor_label(r"\\.\DISPLAY12"), "Display 12");
        assert_eq!(monitor_label("Dell U2720Q"), "Dell U2720Q");
        assert_eq!(bytes(1_234_567), "1.2 MB");
        assert_eq!(bytes(56_000_000), "56 MB");
        assert_eq!(time_left(1), "1 min");
        assert_eq!(time_left(42 * 60_000), "42 min");
        assert_eq!(time_left(3_600_000), "1 h");
        assert_eq!(url_encode("ReUI 0.1 / build#1"), "ReUI%200.1%20%2F%20build%231");
    }

    #[test]
    fn folder_sizes_add_up_nested_files() {
        let dir = crate::model::doc::tests::temp_dir("sizes");
        std::fs::create_dir_all(dir.join("a")).unwrap();
        std::fs::write(dir.join("x"), [0u8; 10]).unwrap();
        std::fs::write(dir.join("a").join("y"), [0u8; 5]).unwrap();
        assert_eq!(folder_size(&dir), 15);
        assert_eq!(folder_size(&dir.join("missing")), 0);
        _ = std::fs::remove_dir_all(&dir);
    }
}

/// Plays a finish sound once, as the Timers module would (`Alarm01` from
/// `C:\Windows\Media`); "none" stops any preview.
pub fn play_sound(name: &str) {
    use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_FILENAME, SND_NODEFAULT};
    let valid = !name.is_empty() && name.len() <= 32 && name.chars().all(|c| c.is_ascii_alphanumeric());
    let windows = std::env::var_os("WINDIR").map_or_else(|| std::path::PathBuf::from(r"C:\Windows"), std::path::PathBuf::from);
    let file = windows.join("Media").join(format!("{name}.wav"));
    unsafe {
        if name == "none" || !valid || !file.is_file() {
            _ = PlaySoundW(PCWSTR::null(), None, Default::default());
        } else {
            _ = PlaySoundW(&HSTRING::from(file.as_os_str()), None, SND_FILENAME | SND_ASYNC | SND_NODEFAULT);
        }
    }
}
