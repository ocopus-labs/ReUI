//! Typing the paste: waiting for the target and the modifiers, the chord each app expects,
//! and the integrity check that tells when Windows would drop our input.

use std::ffi::c_void;
use std::time::{Duration, Instant};

use tokio::sync::mpsc::UnboundedSender;
use windows::Win32::Foundation::*;
use windows::Win32::Security::{GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TokenIntegrityLevel, TOKEN_MANDATORY_LABEL, TOKEN_QUERY};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use super::target::{exe_path, file_name_lower};
use super::OsEvent;

/// Integrity levels (the last sub-authority of the token's mandatory label).
const MEDIUM_INTEGRITY: u32 = 0x2000;
const HIGH_INTEGRITY: u32 = 0x3000;

/// Types the paste chord into `target` once it's in front and the user has let go of the
/// modifiers (up to 500 ms; a still-held Shift/Alt/Win from the hotkey would turn Ctrl+V
/// into a different shortcut). Reports `PasteBlocked` instead if focus is elsewhere, or
/// the target runs at a higher integrity level (Windows would drop our input).
pub(super) fn type_paste(events: &UnboundedSender<OsEvent>, target: HWND) {
    std::thread::sleep(Duration::from_millis(40));
    let held = || [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN].iter().any(|vk| unsafe { GetAsyncKeyState(i32::from(vk.0)) } < 0);
    let in_front = || !target.is_invalid() && unsafe { GetForegroundWindow() } == target;
    let deadline = Instant::now() + Duration::from_millis(500);
    while (held() || !in_front()) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    if !in_front() || !unsafe { can_type_into(target) } {
        _ = events.send(OsEvent::PasteBlocked);
        return;
    }
    let exe = unsafe { exe_path(target) }.map(|p| file_name_lower(&p)).unwrap_or_default();
    let mut class = [0u16; 64];
    let len = unsafe { GetClassNameW(target, &mut class) }.max(0) as usize;
    let inputs = match chord_for(&exe, &String::from_utf16_lossy(&class[..len])) {
        Chord::CtrlV => [key(VK_CONTROL, false), key(VK_V, false), key(VK_V, true), key(VK_CONTROL, true)],
        Chord::ShiftInsert => [key(VK_SHIFT, false), key(VK_INSERT, false), key(VK_INSERT, true), key(VK_SHIFT, true)],
    };
    unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Chord {
    CtrlV,
    ShiftInsert,
}

/// Classic consoles and PuTTY-style terminals read Ctrl+V as a control character; they all
/// paste on Shift+Insert. Windows Terminal and everything else get Ctrl+V.
fn chord_for(exe: &str, class: &str) -> Chord {
    const SHIFT_INSERT: [&str; 4] = ["conhost.exe", "mintty.exe", "putty.exe", "kitty.exe"];
    if class == "ConsoleWindowClass" || SHIFT_INSERT.iter().any(|e| exe.eq_ignore_ascii_case(e)) {
        Chord::ShiftInsert
    } else {
        Chord::CtrlV
    }
}

/// One key event, with its scan code too: RDP and VM windows forward scan codes, not VKs.
fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    let scan = unsafe { MapVirtualKeyW(u32::from(vk.0), MAPVK_VK_TO_VSC) } as u16;
    let mut flags = if up { KEYEVENTF_KEYUP.0 } else { 0 };
    // Insert shares its scan code with numpad 0; the extended bit picks the real Insert.
    if vk == VK_INSERT {
        flags |= KEYEVENTF_EXTENDEDKEY.0;
    }
    INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: KEYBD_EVENT_FLAGS(flags), ..Default::default() } } }
}

/// The integrity level of a process's token.
unsafe fn integrity(process: HANDLE) -> Option<u32> {
    let mut token = HANDLE::default();
    OpenProcessToken(process, TOKEN_QUERY, &mut token).ok()?;
    let mut len = 0u32;
    _ = GetTokenInformation(token, TokenIntegrityLevel, None, 0, &mut len);
    // u64s: the buffer holds a pointer-aligned TOKEN_MANDATORY_LABEL.
    let mut buffer = vec![0u64; (len as usize).div_ceil(8).max(1)];
    let got = GetTokenInformation(token, TokenIntegrityLevel, Some(buffer.as_mut_ptr() as *mut c_void), len, &mut len);
    _ = CloseHandle(token);
    got.ok()?;
    let sid = (*(buffer.as_ptr() as *const TOKEN_MANDATORY_LABEL)).Label.Sid;
    let count = *GetSidSubAuthorityCount(sid);
    (count > 0).then(|| *GetSidSubAuthority(sid, u32::from(count) - 1))
}

/// UIPI drops input sent to a window of a higher integrity level (an elevated app). If its
/// token can't even be read, assume it's elevated unless we are too.
unsafe fn can_type_into(target: HWND) -> bool {
    let ours = integrity(GetCurrentProcess()).unwrap_or(MEDIUM_INTEGRITY);
    let mut pid = 0u32;
    GetWindowThreadProcessId(target, Some(&mut pid));
    let theirs = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok().and_then(|process| {
        let level = integrity(process);
        _ = CloseHandle(process);
        level
    });
    match theirs {
        Some(level) => level <= ours,
        None => ours >= HIGH_INTEGRITY,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords_per_target() {
        assert_eq!(chord_for("conhost.exe", ""), Chord::ShiftInsert);
        assert_eq!(chord_for("cmd.exe", "ConsoleWindowClass"), Chord::ShiftInsert);
        assert_eq!(chord_for("PuTTY.exe", "PuTTY"), Chord::ShiftInsert);
        assert_eq!(chord_for("mintty.exe", "mintty"), Chord::ShiftInsert);
        assert_eq!(chord_for("windowsterminal.exe", "CASCADIA_HOSTING_WINDOW_CLASS"), Chord::CtrlV);
        assert_eq!(chord_for("notepad.exe", "Notepad"), Chord::CtrlV);
        assert_eq!(chord_for("", ""), Chord::CtrlV);
    }
}
