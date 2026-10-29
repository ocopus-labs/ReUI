//! What the calendar shows when (PRD §5.1), as a pure function of the events, the time
//! and what's been dismissed: the countdown in the last minutes before a meeting, the
//! start (until joined, seen, or a few minutes in), the day's card, and when anything of
//! that next changes, so the module sleeps until exactly then and not a moment before.

use std::collections::HashSet;

use super::event::Event;

/// How long a start stays an alert when nobody answers it.
pub const ALERT_MS: u64 = 5 * 60_000;
/// Lines the day's card lists: the hero and the two after it.
pub const DAY_LINES: usize = 3;

#[derive(Debug, PartialEq, Eq)]
pub struct Plan<'a> {
    /// Counting down to its start (`lead` before it).
    pub soon: Option<&'a Event>,
    /// Just started, not dismissed.
    pub starting: Option<&'a Event>,
    /// The day's card: what's left of today (timed first, then all-day), or tomorrow's
    /// when today is done.
    pub day: Vec<&'a Event>,
    pub tomorrow: bool,
    /// When any of the above next changes (Unix ms).
    pub wake: Option<u64>,
}

/// The plan at `now`. `day_end` is the next local midnight, `tomorrow_end` the one after;
/// `seen` holds the keys of starts already dismissed. `events` sorted by start.
pub fn plan<'a>(events: &'a [Event], now: u64, lead: u64, seen: &HashSet<String>, day_end: u64, tomorrow_end: u64) -> Plan<'a> {
    let timed = || events.iter().filter(|e| !e.all_day);
    let soon = timed().find(|e| e.start.saturating_sub(lead) <= now && now < e.start);
    let starting = timed().find(|e| e.start <= now && now < alert_end(e) && !seen.contains(&e.key));

    let today = |e: &&Event| e.end > now && e.start < day_end;
    let mut day: Vec<&Event> = timed().filter(today).chain(events.iter().filter(|e| e.all_day).filter(today)).take(DAY_LINES).collect();
    let mut tomorrow = false;
    if day.is_empty() {
        let next = |e: &&Event| e.start >= day_end && e.start < tomorrow_end;
        day = timed().filter(next).chain(events.iter().filter(|e| e.all_day).filter(next)).take(DAY_LINES).collect();
        tomorrow = !day.is_empty();
    }

    let wake = timed()
        .flat_map(|e| [e.start.saturating_sub(lead), e.start, alert_end(e), e.end])
        .chain(events.iter().filter(|e| e.all_day).flat_map(|e| [e.start, e.end]))
        .chain([day_end])
        .filter(|t| *t > now)
        .min();
    Plan { soon, starting, day, tomorrow, wake }
}

/// A start lapses `ALERT_MS` in, or at the end of a shorter meeting.
fn alert_end(e: &Event) -> u64 {
    (e.start + ALERT_MS).min(e.end.max(e.start + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60_000;

    fn ev(key: &str, start_min: u64, len_min: u64) -> Event {
        Event { key: key.into(), title: key.into(), start: start_min * MIN, end: (start_min + len_min) * MIN, all_day: false, place: None, link: None }
    }

    fn all_day(key: &str, start_min: u64) -> Event {
        Event { all_day: true, ..ev(key, start_min, 24 * 60) }
    }

    fn keys(list: &[&Event]) -> Vec<String> {
        list.iter().map(|e| e.key.clone()).collect()
    }

    const DAY: u64 = 24 * 60 * MIN;

    #[test]
    fn nothing_until_the_lead_then_a_countdown_then_the_start() {
        let events = [ev("standup", 600, 15)];
        let seen = HashSet::new();
        let p = plan(&events, 580 * MIN, 10 * MIN, &seen, DAY, 2 * DAY);
        assert_eq!((p.soon, p.starting), (None, None));
        assert_eq!(p.wake, Some(590 * MIN), "wakes exactly when the countdown starts");
        let p = plan(&events, 590 * MIN, 10 * MIN, &seen, DAY, 2 * DAY);
        assert_eq!(p.soon.map(|e| e.key.as_str()), Some("standup"));
        assert_eq!(p.wake, Some(600 * MIN));
        let p = plan(&events, 600 * MIN, 10 * MIN, &seen, DAY, 2 * DAY);
        assert_eq!((p.soon, p.starting.map(|e| e.key.as_str())), (None, Some("standup")));
        assert_eq!(p.wake, Some(605 * MIN), "the alert lapses five minutes in");
        let p = plan(&events, 605 * MIN, 10 * MIN, &seen, DAY, 2 * DAY);
        assert_eq!(p.starting, None);
        assert_eq!(keys(&p.day), ["standup"], "still in the card while it's on");
        assert_eq!(p.wake, Some(615 * MIN));
    }

    #[test]
    fn a_dismissed_start_stays_dismissed_and_short_meetings_end_their_alert() {
        let events = [ev("standup", 600, 15), ev("chat", 700, 2)];
        let seen: HashSet<String> = ["standup".to_string()].into();
        assert_eq!(plan(&events, 601 * MIN, 10 * MIN, &seen, DAY, 2 * DAY).starting, None);
        let p = plan(&events, 701 * MIN, 10 * MIN, &seen, DAY, 2 * DAY);
        assert_eq!(p.starting.map(|e| e.key.as_str()), Some("chat"));
        assert_eq!(p.wake, Some(702 * MIN), "a two-minute chat's alert ends with it");
    }

    #[test]
    fn back_to_back_counts_down_the_next_while_one_is_on() {
        let events = [ev("first", 600, 60), ev("second", 660, 30)];
        let p = plan(&events, 650 * MIN, 10 * MIN, &HashSet::new(), DAY, 2 * DAY);
        assert_eq!(p.soon.map(|e| e.key.as_str()), Some("second"));
        assert_eq!(keys(&p.day), ["first", "second"]);
    }

    #[test]
    fn all_day_events_never_count_down_and_list_last() {
        let events = [all_day("offsite", 0), ev("lunch", 720, 60)];
        let p = plan(&events, 0, 10 * MIN, &HashSet::new(), DAY, 2 * DAY);
        assert_eq!((p.soon, p.starting), (None, None), "an all-day event starting now is no alert");
        assert_eq!(keys(&p.day), ["lunch", "offsite"]);
    }

    #[test]
    fn a_finished_day_shows_tomorrow_and_wakes_at_midnight() {
        let events = [ev("done", 600, 30), ev("tomorrow", DAY / MIN + 540, 30)];
        let p = plan(&events, 1000 * MIN, 10 * MIN, &HashSet::new(), DAY, 2 * DAY);
        assert!(p.tomorrow);
        assert_eq!(keys(&p.day), ["tomorrow"]);
        assert_eq!(p.wake, Some(DAY));
        let empty: [Event; 0] = [];
        let p = plan(&empty, 0, 10 * MIN, &HashSet::new(), DAY, 2 * DAY);
        assert_eq!((p.day.len(), p.tomorrow, p.wake), (0, false, Some(DAY)), "nothing at all: only midnight");
    }

    #[test]
    fn the_card_holds_three() {
        let events: Vec<Event> = (0..5).map(|i| ev(&format!("e{i}"), 600 + i * 60, 30)).collect();
        assert_eq!(plan(&events, 0, 10 * MIN, &HashSet::new(), DAY, 2 * DAY).day.len(), DAY_LINES);
    }
}
