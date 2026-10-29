//! Windows' own calendar store (PRD §6.1), read-only: `AppointmentManager` works without
//! package identity (tried on Windows 11 26200 from an unpackaged process), but since Mail
//! & Calendar was retired little fills it, so it's an opt-in source. Read on a blocking
//! thread when the module refreshes; the result is a plain list of events.

use windows::ApplicationModel::Appointments::{AppointmentManager, AppointmentParticipantResponse, AppointmentProperties, AppointmentStoreAccessType, FindAppointmentsOptions};
use windows::Foundation::{DateTime, TimeSpan};

use super::event::{Event, Source};
use super::links;

/// 100 ns ticks between 1601 (WinRT's epoch) and 1970.
const EPOCH_TICKS: i64 = 116_444_736_000_000_000;

/// The store's events from `from` (Unix ms) for `length_ms`. Blocking (WinRT).
pub fn read(from: u64, length_ms: u64) -> Result<Vec<Event>, String> {
    let fail = |e: windows::core::Error| format!("{:#010x}", e.code().0);
    let store = AppointmentManager::RequestStoreAsync(AppointmentStoreAccessType::AllCalendarsReadOnly).and_then(|op| op.join()).map_err(fail)?;
    let options = FindAppointmentsOptions::new().map_err(fail)?;
    _ = options.SetMaxCount(500);
    if let Ok(fetch) = options.FetchProperties() {
        for p in [
            AppointmentProperties::Subject(),
            AppointmentProperties::Location(),
            AppointmentProperties::StartTime(),
            AppointmentProperties::Duration(),
            AppointmentProperties::AllDay(),
            AppointmentProperties::OnlineMeetingLink(),
            AppointmentProperties::Details(),
            AppointmentProperties::IsCanceledMeeting(),
            AppointmentProperties::UserResponse(),
        ]
        .into_iter()
        .flatten()
        {
            _ = fetch.Append(&p);
        }
    }
    let start = DateTime { UniversalTime: from as i64 * 10_000 + EPOCH_TICKS };
    let length = TimeSpan { Duration: length_ms as i64 * 10_000 };
    let list = store.FindAppointmentsAsyncWithOptions(start, length, &options).and_then(|op| op.join()).map_err(fail)?;
    let mut events = Vec::new();
    for a in list {
        if a.IsCanceledMeeting().unwrap_or(false) || a.UserResponse().is_ok_and(|r| r == AppointmentParticipantResponse::Declined) {
            continue;
        }
        let Ok(at) = a.StartTime() else { continue };
        let start = (at.UniversalTime - EPOCH_TICKS) / 10_000;
        let length = a.Duration().map_or(0, |d| d.Duration / 10_000).max(0);
        let text = |r: windows::core::Result<windows::core::HSTRING>| r.map(|s| s.to_string()).unwrap_or_default();
        let (location, meeting, details) = (text(a.Location()), text(a.OnlineMeetingLink()), text(a.Details()));
        let id = a.RoamingId().or_else(|_| a.LocalId()).map(|s| s.to_string()).unwrap_or_default();
        events.push(Event {
            key: format!("windows|{id}|{start}"),
            title: text(a.Subject()).trim().to_string(),
            start: start.max(0) as u64,
            end: (start + length).max(0) as u64,
            all_day: a.AllDay().unwrap_or(false),
            place: links::place(&location),
            link: links::find([meeting.as_str(), location.as_str(), details.as_str()]),
        });
    }
    events.sort_by_key(|e| e.start);
    Ok(events)
}

/// The last read.
#[derive(Debug, Default)]
pub struct Snapshot(pub Vec<Event>);

impl Source for Snapshot {
    fn events(&self, from: u64, to: u64) -> Vec<Event> {
        self.0.iter().filter(|e| e.start < to && (e.end > from || e.start >= from)).cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Read-only, and whatever this PC's store holds (often nothing): it must open. By hand
    /// (`cargo test -p reui-hub store_opens -- --ignored`): it depends on the PC's
    /// privacy settings, which CI's machines don't share.
    #[test]
    #[ignore = "reads this PC's calendar store"]
    fn the_store_opens_without_package_identity() {
        let now = reui_protocol::unix_ms();
        match read(now, 86_400_000) {
            Ok(events) => println!("windows store: {} events", events.len()),
            Err(e) => panic!("the store didn't open: {e}"),
        }
    }
}
