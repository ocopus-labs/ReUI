//! Feeds shaped like Google's and Outlook's secret links, read into occurrences.

use super::*;
use crate::modules::calendar::links::Service;

fn ms(s: &str) -> u64 {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap().and_utc().timestamp_millis() as u64
}

fn at(ms: u64) -> String {
    chrono::DateTime::from_timestamp_millis(ms as i64).unwrap().format("%Y-%m-%d %H:%M").to_string()
}

/// Google: IANA zones with their VTIMEZONE, the owner's address as the calendar's name,
/// the Meet link in X-GOOGLE-CONFERENCE, a moved and a cancelled occurrence.
const GOOGLE: &str = "BEGIN:VCALENDAR\r
PRODID:-//Google Inc//Google Calendar 70.9054//EN\r
VERSION:2.0\r
X-WR-CALNAME:sam@example.com\r
X-WR-TIMEZONE:Europe/London\r
BEGIN:VTIMEZONE\r
TZID:Europe/London\r
BEGIN:DAYLIGHT\r
TZOFFSETFROM:+0000\r
TZOFFSETTO:+0100\r
DTSTART:19700329T010000\r
RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU\r
END:DAYLIGHT\r
BEGIN:STANDARD\r
TZOFFSETFROM:+0100\r
TZOFFSETTO:+0000\r
DTSTART:19701025T020000\r
RRULE:FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU\r
END:STANDARD\r
END:VTIMEZONE\r
BEGIN:VEVENT\r
DTSTART;TZID=Europe/London:20260105T093000\r
DTEND;TZID=Europe/London:20260105T094500\r
RRULE:FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR\r
EXDATE;TZID=Europe/London:20261029T093000\r
UID:standup@google.com\r
SUMMARY:Standup\r
X-GOOGLE-CONFERENCE:https://meet.google.com/abc-defg-hij\r
DESCRIPTION:Join with Google Meet: https://meet.google.com/abc-defg-hij\\n\\nLearn more\r
 about Meet at: https://support.google.com/a/users/answer/9282720\r
END:VEVENT\r
BEGIN:VEVENT\r
DTSTART;TZID=Europe/London:20261030T110000\r
DTEND;TZID=Europe/London:20261030T111500\r
RECURRENCE-ID;TZID=Europe/London:20261030T093000\r
UID:standup@google.com\r
SUMMARY:Standup (moved)\r
END:VEVENT\r
BEGIN:VEVENT\r
DTSTART:20261030T140000Z\r
DTEND:20261030T150000Z\r
UID:review@google.com\r
SUMMARY:Design review\r
LOCATION:Room 4\r
ATTENDEE;CN=Sam;PARTSTAT=ACCEPTED:mailto:sam@example.com\r
END:VEVENT\r
BEGIN:VEVENT\r
DTSTART:20261030T160000Z\r
DTEND:20261030T170000Z\r
UID:declined@google.com\r
SUMMARY:Optional sync\r
ATTENDEE;CN=Sam;PARTSTAT=DECLINED:mailto:SAM@example.com\r
END:VEVENT\r
BEGIN:VEVENT\r
DTSTART:20261030T170000Z\r
DTEND:20261030T180000Z\r
UID:cancelled@google.com\r
STATUS:CANCELLED\r
SUMMARY:Cancelled thing\r
END:VEVENT\r
BEGIN:VEVENT\r
DTSTART;VALUE=DATE:20261030\r
DTEND;VALUE=DATE:20261031\r
UID:allday@google.com\r
SUMMARY:Offsite\r
END:VEVENT\r
END:VCALENDAR\r
";

/// Outlook: a Windows-worded zone defined in the feed, the Teams link only inside a Safe
/// Link in the description.
const OUTLOOK: &str = "BEGIN:VCALENDAR\r
METHOD:PUBLISH\r
PRODID:Microsoft Exchange Server 2010\r
VERSION:2.0\r
X-WR-CALNAME:Calendar\r
BEGIN:VTIMEZONE\r
TZID:(UTC-05:00) Eastern Time (US & Canada)\r
BEGIN:STANDARD\r
DTSTART:16010101T020000\r
TZOFFSETFROM:-0400\r
TZOFFSETTO:-0500\r
RRULE:FREQ=YEARLY;INTERVAL=1;BYDAY=1SU;BYMONTH=11\r
END:STANDARD\r
BEGIN:DAYLIGHT\r
DTSTART:16010101T020000\r
TZOFFSETFROM:-0500\r
TZOFFSETTO:-0400\r
RRULE:FREQ=YEARLY;INTERVAL=1;BYDAY=2SU;BYMONTH=3\r
END:DAYLIGHT\r
END:VTIMEZONE\r
BEGIN:VEVENT\r
DESCRIPTION:________________\\nMicrosoft Teams meeting\\nJoin: https://nam1\r
 1.safelinks.protection.outlook.com/?url=https%3A%2F%2Fteams.microsoft.com%2F\r
 l%2Fmeetup-join%2F19%253ameeting_X%2540thread.v2%2F0&data=05&reserved=0\\n\r
RRULE:FREQ=MONTHLY;UNTIL=20270101T000000Z;INTERVAL=1;BYDAY=-1FR\r
UID:040000008200E00074C5B7101A82E008\r
SUMMARY:Monthly review\r
DTSTART;TZID=\"(UTC-05:00) Eastern Time (US & Canada)\":20260130T090000\r
DTEND;TZID=\"(UTC-05:00) Eastern Time (US & Canada)\":20260130T100000\r
LOCATION:Microsoft Teams Meeting\r
END:VEVENT\r
END:VCALENDAR\r
";

#[test]
fn a_google_feed_expands_moves_and_hides_what_it_should() {
    let feed = Feed::parse("g", GOOGLE);
    let day = feed.occurrences(ms("2026-10-30 00:00"), ms("2026-10-31 00:00"), &Fixed(0));
    let titles: Vec<&str> = day.iter().map(|e| e.title.as_str()).collect();
    assert_eq!(titles, ["Offsite", "Standup (moved)", "Design review"], "no declined, no cancelled, the 9:30 replaced");
    let moved = &day[1];
    // London is on GMT again after Oct 25: 11:00 local is 11:00 UTC.
    assert_eq!(at(moved.start), "2026-10-30 11:00");
    assert!(moved.key.ends_with(&ms("2026-10-30 09:30").to_string()), "keyed by the start it replaces");
    assert!(day[0].all_day);
    assert_eq!((at(day[0].start), at(day[0].end)), ("2026-10-30 00:00".into(), "2026-10-31 00:00".into()));
    assert_eq!(day[2].place.as_deref(), Some("Room 4"));

    let before = feed.occurrences(ms("2026-10-21 00:00"), ms("2026-10-22 00:00"), &Fixed(0));
    let standup = before.iter().find(|e| e.title == "Standup").unwrap();
    // Still British Summer Time on the 21st (it ends on the 25th).
    assert_eq!(at(standup.start), "2026-10-21 08:30");
    assert_eq!(standup.link.as_ref().map(|l| l.service), Some(Service::Meet));
    assert_eq!(standup.where_().as_deref(), Some("Google Meet"));
    let excluded = feed.occurrences(ms("2026-10-29 00:00"), ms("2026-10-30 00:00"), &Fixed(0));
    assert!(excluded.iter().all(|e| e.title != "Standup"), "EXDATE");
}

#[test]
fn an_outlook_feed_reads_its_own_zone_and_safe_links() {
    let feed = Feed::parse("o", OUTLOOK);
    let oct = feed.occurrences(ms("2026-10-01 00:00"), ms("2026-11-01 00:00"), &Fixed(0));
    assert_eq!(oct.len(), 1);
    // Last Friday of October, 9:00 EDT (UTC-4).
    assert_eq!(at(oct[0].start), "2026-10-30 13:00");
    let nov = feed.occurrences(ms("2026-11-01 00:00"), ms("2026-12-01 00:00"), &Fixed(0));
    // Nov 27, 9:00 EST (UTC-5): the meeting stays at 9:00 across the change.
    assert_eq!(at(nov[0].start), "2026-11-27 14:00");
    assert_eq!(nov[0].end - nov[0].start, 3_600_000);
    let link = nov[0].link.as_ref().unwrap();
    assert_eq!(link.service, Service::Teams);
    assert!(link.url.starts_with("https://teams.microsoft.com/l/meetup-join/"));
    assert_eq!(nov[0].place, None, "\"Microsoft Teams Meeting\" isn't a place");
    assert!(feed.occurrences(ms("2027-01-01 00:00"), ms("2027-03-01 00:00"), &Fixed(0)).is_empty(), "UNTIL");
}

#[test]
fn floating_and_all_day_times_are_the_pcs_zone() {
    let feed = Feed::parse("f", "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:a\nDTSTART:20261030T090000\nDURATION:PT30M\nSUMMARY:Floating\nEND:VEVENT\nBEGIN:VEVENT\nUID:b\nDTSTART;VALUE=DATE:20261030\nSUMMARY:No end\nEND:VEVENT\nEND:VCALENDAR\n");
    let day = feed.occurrences(ms("2026-10-29 00:00"), ms("2026-10-31 00:00"), &Fixed(5));
    let floating = day.iter().find(|e| e.title == "Floating").unwrap();
    assert_eq!(at(floating.start), "2026-10-30 04:00", "09:00 at UTC+5");
    assert_eq!(floating.end - floating.start, 30 * 60_000);
    let all_day = day.iter().find(|e| e.title == "No end").unwrap();
    assert_eq!(all_day.end - all_day.start, 86_400_000, "an all-day event without an end is one day");
}
