//! The idle island (no activities): what its pill shows, and the wall clock it reads.
//!
//! At rest it's the quiet empty pill, or the time with `idle_clock` on (for a hidden
//! taskbar). A peek shows the date and time; a click opens the row of places (the view
//! switcher's destinations). The clock is redrawn once a minute only while one shows, so a
//! resting island without it runs nothing.

use windows::core::PCWSTR;
use windows::Win32::Globalization::{GetDateFormatEx, GetTimeFormatEx, TIME_NOSECONDS};
use windows::Win32::System::SystemInformation::GetLocalTime;

/// What the idle pill shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Face {
    /// The quiet empty pill.
    Empty,
    /// The time alone, quiet (resting, with `idle_clock` on).
    Time,
    /// The date and the time (a peek).
    DateTime,
    /// The row of places (clicked open).
    Places,
}

/// The face for the idle pill: `peek` (the pointer rests on it), `open` (clicked open into
/// the places), `clock` (the `idle_clock` setting).
pub fn face(peek: bool, open: bool, clock: bool) -> Face {
    match (open, peek, clock) {
        (true, ..) => Face::Places,
        (_, true, _) => Face::DateTime,
        (_, _, true) => Face::Time,
        _ => Face::Empty,
    }
}

impl Face {
    /// Whether it shows the time (and so needs redrawing each minute).
    pub fn shows_time(&self) -> bool {
        matches!(self, Face::Time | Face::DateTime)
    }
}

/// Milliseconds until the minute turns, from the current second and millisecond.
pub fn until_next_minute(second: u16, millis: u16) -> u32 {
    let into = u32::from(second.min(59)) * 1000 + u32::from(millis.min(999));
    60_000 - into
}

/// The local date ("Thu 30 Oct", the user's language for the names) and time (the user's
/// 12- or 24-hour format, no seconds), and how long until the minute turns.
pub fn now() -> (String, String, u32) {
    let st = unsafe { GetLocalTime() };
    let mut buf = [0u16; 64];
    let read = |n: i32, buf: &[u16]| String::from_utf16_lossy(&buf[..(n.max(1) as usize - 1).min(buf.len())]);
    let picture: Vec<u16> = "ddd d MMM\0".encode_utf16().collect();
    let n = unsafe { GetDateFormatEx(PCWSTR::null(), Default::default(), Some(&st), PCWSTR(picture.as_ptr()), Some(&mut buf), PCWSTR::null()) };
    let date = read(n, &buf);
    let n = unsafe { GetTimeFormatEx(PCWSTR::null(), TIME_NOSECONDS, Some(&st), PCWSTR::null(), Some(&mut buf)) };
    let time = read(n, &buf);
    (date, time, until_next_minute(st.wSecond, st.wMilliseconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rest_peek_and_open() {
        assert_eq!(face(false, false, false), Face::Empty, "the quiet pill by default");
        assert_eq!(face(false, false, true), Face::Time);
        assert_eq!(face(true, false, false), Face::DateTime, "a peek shows the date either way");
        assert_eq!(face(true, true, true), Face::Places);
        assert!(!Face::Empty.shows_time() && !Face::Places.shows_time());
        assert!(Face::Time.shows_time() && Face::DateTime.shows_time());
    }

    #[test]
    fn the_clock_wakes_when_the_minute_turns() {
        assert_eq!(until_next_minute(0, 0), 60_000);
        assert_eq!(until_next_minute(59, 999), 1);
        assert_eq!(until_next_minute(30, 500), 29_500);
    }
}
