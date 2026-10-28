//! The finish sound: a Windows alarm file played in a loop until stopped. Files rather than
//! the sound scheme's aliases, which the user's scheme may have made silent; a missing file
//! falls back to Alarm01, then to the default beep (E-T10).

use std::path::PathBuf;

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_FILENAME, SND_LOOP, SND_NODEFAULT};
use windows::Win32::System::Diagnostics::Debug::MessageBeep;
use windows::Win32::UI::WindowsAndMessaging::MB_ICONASTERISK;

pub const DEFAULT: &str = "Alarm01";

/// `name` is a file in `C:\Windows\Media` without `.wav` (Alarm01–10, Ring01–10); `None`
/// plays nothing. `looped` repeats it until [`stop`].
pub fn play(name: Option<&str>, looped: bool) {
    let Some(name) = name else { return };
    let candidates = [file(name), file(DEFAULT)];
    let Some(path) = candidates.iter().find(|p| p.is_file()) else {
        unsafe { _ = MessageBeep(MB_ICONASTERISK) };
        return;
    };
    let flags = SND_FILENAME | SND_ASYNC | SND_NODEFAULT | if looped { SND_LOOP } else { Default::default() };
    let wide = HSTRING::from(path.as_os_str());
    unsafe {
        if !PlaySoundW(&wide, None, flags).as_bool() {
            _ = MessageBeep(MB_ICONASTERISK);
        }
    }
}

pub fn stop() {
    unsafe { _ = PlaySoundW(PCWSTR::null(), None, Default::default()) };
}

fn file(name: &str) -> PathBuf {
    let windows = std::env::var_os("WINDIR").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    windows.join("Media").join(format!("{name}.wav"))
}
