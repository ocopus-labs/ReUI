//! Zones a feed names without defining (`TZID=America/New_York` and no `VTIMEZONE`), from
//! the ICU Windows ships (`icuin.dll`, Windows 10 1703+), which knows the IANA names and
//! maps Windows' own ("Pacific Standard Time"). That way ReUI carries no time-zone
//! database of its own and stays as current as Windows Update keeps it. A name ICU doesn't
//! know, and floating or all-day times, are the PC's own zone (PRD E-C2).

use chrono::{Datelike, NaiveDateTime, TimeZone, Timelike};
use windows::core::PCSTR;
use windows::Win32::Globalization::{ucal_clear, ucal_close, ucal_getCanonicalTimeZoneID, ucal_getMillis, ucal_getTimeZoneIDForWindowsID, ucal_open, ucal_setDateTime, UCAL_GREGORIAN, U_ZERO_ERROR};

use super::time::Resolve;

/// The PC's zone and ICU's.
pub struct System;

impl Resolve for System {
    fn utc_ms(&self, local: NaiveDateTime, tzid: Option<&str>) -> i64 {
        tzid.and_then(|id| icu(local, id)).unwrap_or_else(|| local_ms(local))
    }
}

/// In the PC's own zone; a skipped hour reads with the offset from before the change.
pub fn local_ms(local: NaiveDateTime) -> i64 {
    match chrono::Local.from_local_datetime(&local) {
        chrono::LocalResult::Single(t) | chrono::LocalResult::Ambiguous(t, _) => t.timestamp_millis(),
        chrono::LocalResult::None => chrono::Local.from_local_datetime(&(local - chrono::Duration::hours(1))).earliest().map_or(0, |t| t.timestamp_millis() + 3_600_000),
    }
}

/// `local` in zone `id` (IANA, or a Windows name), or `None` if ICU doesn't know it.
fn icu(local: NaiveDateTime, id: &str) -> Option<i64> {
    let zone = known(id).or_else(|| from_windows(id))?;
    let wide: Vec<u16> = zone.encode_utf16().collect();
    let mut status = U_ZERO_ERROR;
    unsafe {
        let cal = ucal_open(wide.as_ptr(), wide.len() as i32, PCSTR::null(), UCAL_GREGORIAN, &mut status);
        if cal.is_null() || status.0 > 0 {
            return None;
        }
        ucal_clear(cal);
        ucal_setDateTime(cal, local.year(), local.month0() as i32, local.day() as i32, local.hour() as i32, local.minute() as i32, local.second() as i32, &mut status);
        let ms = ucal_getMillis(cal as *const _, &mut status);
        ucal_close(cal);
        (status.0 <= 0).then_some(ms as i64)
    }
}

/// The canonical IANA id when ICU knows `id` as a system zone.
fn known(id: &str) -> Option<String> {
    let wide: Vec<u16> = id.encode_utf16().collect();
    let mut out = [0u16; 128];
    let mut system = 0i8;
    let mut status = U_ZERO_ERROR;
    let n = unsafe { ucal_getCanonicalTimeZoneID(wide.as_ptr(), wide.len() as i32, out.as_mut_ptr(), out.len() as i32, &mut system, &mut status) };
    (status.0 <= 0 && system != 0 && n > 0).then(|| String::from_utf16_lossy(&out[..n as usize]))
}

/// A Windows zone name ("W. Europe Standard Time") as its IANA id.
fn from_windows(id: &str) -> Option<String> {
    let wide: Vec<u16> = id.encode_utf16().collect();
    let mut out = [0u16; 128];
    let mut status = U_ZERO_ERROR;
    let n = unsafe { ucal_getTimeZoneIDForWindowsID(wide.as_ptr(), wide.len() as i32, PCSTR::null(), out.as_mut_ptr(), out.len() as i32, &mut status) };
    (status.0 <= 0 && n > 0).then(|| String::from_utf16_lossy(&out[..n as usize]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    fn utc(ms: i64) -> String {
        chrono::DateTime::from_timestamp_millis(ms).unwrap().format("%Y-%m-%d %H:%M").to_string()
    }

    #[test]
    fn windows_icu_knows_iana_and_windows_names() {
        assert_eq!(icu(t("2026-07-01 09:00"), "America/New_York").map(utc).as_deref(), Some("2026-07-01 13:00"));
        assert_eq!(icu(t("2026-12-01 09:00"), "America/New_York").map(utc).as_deref(), Some("2026-12-01 14:00"));
        assert_eq!(icu(t("2026-07-01 09:00"), "W. Europe Standard Time").map(utc).as_deref(), Some("2026-07-01 07:00"));
        assert_eq!(icu(t("2026-07-01 09:00"), "Asia/Kolkata").map(utc).as_deref(), Some("2026-07-01 03:30"));
        assert_eq!(icu(t("2026-07-01 09:00"), "Not/AZone"), None);
        assert_eq!(icu(t("2026-07-01 09:00"), "(UTC+01:00) Amsterdam, Berlin"), None, "Outlook's words need the feed's VTIMEZONE");
    }
}
