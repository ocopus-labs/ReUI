//! The player behind a media session as a desktop app: which executable it is (for names
//! Windows doesn't give us) and bringing its window forward when the card's art is clicked.
//! Blocking Win32/COM: call from a blocking thread.

use windows::core::{BOOL, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_LOCAL_SERVER, COINIT_MULTITHREADED};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::Shell::{ApplicationActivationManager, IApplicationActivationManager, ShellExecuteW, AO_NONE};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetWindow, GetWindowThreadProcessId, IsIconic, IsWindowVisible, SetForegroundWindow, ShowWindow, GW_OWNER, SW_RESTORE, SW_SHOWNORMAL,
};

/// Browsers that name their media session with an install hash instead of an app id.
pub const HASH_BROWSERS: [&str; 5] = ["firefox.exe", "librewolf.exe", "waterfox.exe", "zen.exe", "floorp.exe"];

/// Lower-case file name of a process's executable ("spotify.exe").
pub fn exe_name(pid: u32) -> Option<String> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buffer = [0u16; 1024];
        let mut len = buffer.len() as u32;
        let named = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &mut len);
        _ = CloseHandle(process);
        named.ok()?;
        let path = String::from_utf16_lossy(&buffer[..len as usize]);
        Some(path.rsplit(['\\', '/']).next().unwrap_or(&path).to_lowercase())
    }
}

/// Visible top-level app windows (not tool or owned windows) with their process ids, in
/// z-order (front first). Also how the agent module finds a session's terminal.
pub fn app_windows() -> Vec<(HWND, u32)> {
    unsafe extern "system" fn each(hwnd: HWND, found: LPARAM) -> BOOL {
        let found = unsafe { &mut *(found.0 as *mut Vec<(HWND, u32)>) };
        unsafe {
            if IsWindowVisible(hwnd).as_bool() && GetWindow(hwnd, GW_OWNER).is_err() {
                let mut pid = 0;
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                found.push((hwnd, pid));
            }
        }
        BOOL(1)
    }
    let mut found: Vec<(HWND, u32)> = Vec::new();
    unsafe {
        _ = EnumWindows(Some(each), LPARAM(&mut found as *mut _ as isize));
    }
    found
}

/// The first of `exes` that has a window open.
pub fn running(exes: &[&str]) -> Option<String> {
    let mut seen = std::collections::HashSet::new();
    app_windows().into_iter().filter(|(_, pid)| seen.insert(*pid)).filter_map(|(_, pid)| exe_name(pid)).find(|exe| exes.contains(&exe.as_str()))
}

/// The executable a desktop player's AUMID names: "Spotify.exe", "Chrome" → "chrome.exe",
/// "C:\…\foobar2000.exe". `None` for packaged apps.
fn exe_for(aumid: &str) -> Option<String> {
    if aumid.contains('!') || aumid.is_empty() {
        return None;
    }
    let file = aumid.rsplit(['\\', '/']).next().unwrap_or(aumid).to_lowercase();
    // PWAs ("Chrome._crx_…") live in their browser's process.
    let file = file.split("._crx_").next().unwrap_or(&file).to_string();
    Some(if file.ends_with(".exe") { file } else { format!("{file}.exe") })
}

/// Brings the player forward: packaged apps through the activation manager (which also
/// starts them), desktop apps by their main window. Best effort; failures are logged.
pub fn open(aumid: &str, pid: Option<u32>) {
    if aumid.contains('!') {
        let activated = unsafe {
            _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            CoCreateInstance::<_, IApplicationActivationManager>(&ApplicationActivationManager, None, CLSCTX_LOCAL_SERVER).and_then(|m| m.ActivateApplication(&HSTRING::from(aumid), PCWSTR::null(), AO_NONE))
        };
        if let Err(e) = activated {
            eprintln!("media: can't open {aumid}: {e}");
        }
        return;
    }
    let windows = app_windows();
    let by_pid = pid.and_then(|pid| windows.iter().find(|(_, p)| *p == pid));
    let by_exe = || {
        let exe = exe_for(aumid)?;
        windows.iter().find(|(_, p)| exe_name(*p).as_deref() == Some(exe.as_str()))
    };
    match by_pid.or_else(by_exe) {
        Some(&(hwnd, _)) => focus(hwnd),
        None => eprintln!("media: no window to open for {aumid}"),
    }
}

/// Windows lets only the foreground process hand the foreground on; borrowing the
/// foreground thread's input state is the usual way round that for a background helper.
pub fn focus(hwnd: HWND) {
    unsafe {
        if IsIconic(hwnd).as_bool() {
            _ = ShowWindow(hwnd, SW_RESTORE);
        }
        if SetForegroundWindow(hwnd).as_bool() {
            return;
        }
        let ours = GetCurrentThreadId();
        let theirs = GetWindowThreadProcessId(GetForegroundWindow(), None);
        let attached = theirs != 0 && theirs != ours && AttachThreadInput(ours, theirs, true).as_bool();
        let focused = SetForegroundWindow(hwnd).as_bool();
        if attached {
            _ = AttachThreadInput(ours, theirs, false);
        }
        if !focused {
            eprintln!("media: Windows didn't let the player come forward");
        }
    }
}

/// Sound settings, where the output device can be chosen when we can't switch it.
pub fn open_sound_settings() {
    unsafe {
        _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let result = ShellExecuteW(None, &HSTRING::from("open"), &HSTRING::from("ms-settings:sound"), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
        if result.0 as isize <= 32 {
            eprintln!("media: can't open Sound settings ({})", result.0 as isize);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::exe_for;

    #[test]
    fn desktop_aumids_name_their_executable() {
        assert_eq!(exe_for("Spotify.exe").as_deref(), Some("spotify.exe"));
        assert_eq!(exe_for("Chrome").as_deref(), Some("chrome.exe"));
        assert_eq!(exe_for("Chrome._crx_cinhimbnkkaeohfgghhklpknlkffjgod").as_deref(), Some("chrome.exe"));
        assert_eq!(exe_for(r"C:\Apps\foobar2000.exe").as_deref(), Some("foobar2000.exe"));
        assert_eq!(exe_for("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"), None);
    }
}
