//! The only UI: message boxes.

use windows::core::HSTRING;
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, IDYES, MB_DEFBUTTON2, MB_ICONERROR, MB_ICONINFORMATION, MB_ICONQUESTION, MB_SETFOREGROUND, MB_YESNO, MESSAGEBOX_STYLE};

use crate::layout::APP;

fn show(text: &str, style: MESSAGEBOX_STYLE) -> bool {
    unsafe { MessageBoxW(None, &HSTRING::from(text), &HSTRING::from(APP), style | MB_SETFOREGROUND) == IDYES }
}

pub fn info(text: &str) {
    show(text, MB_ICONINFORMATION);
}

pub fn error(text: &str) {
    show(text, MB_ICONERROR);
}

/// Yes/No, No by default.
pub fn ask(text: &str) -> bool {
    show(text, MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2)
}
