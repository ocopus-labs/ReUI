//! An iCalendar feed (RFC 5545) read into occurrences: the events of a secret ICS link,
//! with recurrences expanded (`rrule.rs`), exceptions applied (EXDATE, RDATE, a moved or
//! cancelled occurrence's own VEVENT with RECURRENCE-ID), and what the user shouldn't see
//! left out (cancelled, and declined by the calendar's owner when the feed says who that
//! is). Parsing is the expensive part and happens once per download, off the hub thread;
//! expanding a few days is cheap and happens whenever the schedule is looked at.

pub mod lines;
pub mod rrule;
pub mod time;
pub mod zones;

use std::collections::{HashMap, HashSet};

use chrono::NaiveDateTime;

use self::lines::Component;
use self::rrule::Rule;
use self::time::{Resolve, When, Zone};
use super::event::{Event, Source};
use super::links::{self, Link};

/// One VEVENT, as written.
#[derive(Debug, Clone)]
struct Master {
    uid: String,
    title: String,
    start: When,
    /// From DTEND, or DTSTART + DURATION; `None`: neither.
    end: Option<When>,
    duration_ms: Option<i64>,
    rule: Option<Rule>,
    rdates: Vec<When>,
    exdates: Vec<When>,
    /// This VEVENT replaces the occurrence that would have started then.
    recurrence_id: Option<When>,
    cancelled: bool,
    declined: bool,
    place: Option<String>,
    link: Option<Link>,
}

/// A parsed feed.
#[derive(Debug, Clone, Default)]
pub struct Feed {
    /// The source's id, the start of every occurrence's key.
    pub id: String,
    masters: Vec<Master>,
    zones: HashMap<String, Zone>,
    /// Masters with a rule finer than we expand (E-C5), for one log line.
    pub unsupported: usize,
}

/// The zones a feed defines, then the PC's (ICU, else local time).
struct FeedZones<'a> {
    zones: &'a HashMap<String, Zone>,
    os: &'a dyn Resolve,
}

impl Resolve for FeedZones<'_> {
    fn utc_ms(&self, local: NaiveDateTime, tzid: Option<&str>) -> i64 {
        match tzid.and_then(|id| self.zones.get(id)) {
            Some(zone) => zone.utc_ms(local),
            None => self.os.utc_ms(local, tzid),
        }
    }
}

const DAY_MS: i64 = 86_400_000;

impl Feed {
    pub fn parse(id: &str, text: &str) -> Feed {
        let top = lines::parse(text);
        let mut feed = Feed { id: id.to_string(), ..Feed::default() };
        for cal in top.iter().filter(|c| c.kind == "VCALENDAR") {
            // Google names a primary calendar after its owner's address: their own
            // declines are in the feed as their attendee line (E-C10).
            let owner = cal.text("X-WR-CALNAME").filter(|n| n.contains('@')).map(|n| n.trim().to_ascii_lowercase());
            for child in &cal.children {
                match child.kind.as_str() {
                    "VTIMEZONE" => {
                        if let Some((id, zone)) = Zone::from_component(child) {
                            feed.zones.insert(id, zone);
                        }
                    }
                    "VEVENT" => {
                        if let Some(m) = master(child, owner.as_deref()) {
                            feed.unsupported += usize::from(m.rule.as_ref().is_some_and(|r| r.unsupported));
                            feed.masters.push(m);
                        }
                    }
                    _ => {}
                }
            }
        }
        feed
    }

    pub fn len(&self) -> usize {
        self.masters.len()
    }

    /// Occurrences overlapping `from..to` (Unix ms), resolving zones the feed doesn't
    /// define with `os`.
    pub fn occurrences(&self, from: u64, to: u64, os: &dyn Resolve) -> Vec<Event> {
        let zones = FeedZones { zones: &self.zones, os };
        let (from, to) = (from as i64, to as i64);
        // Replacements by (uid, the start they replace).
        let mut moved: HashSet<(&str, i64)> = HashSet::new();
        for m in &self.masters {
            if let Some(rid) = &m.recurrence_id {
                moved.insert((m.uid.as_str(), rid.unix_ms(&zones)));
            }
        }
        let mut out = Vec::new();
        for m in &self.masters {
            if m.cancelled || m.declined {
                continue;
            }
            let length = self.length(m, &zones);
            let starts: Vec<i64> = match (&m.rule, &m.recurrence_id) {
                (Some(rule), None) => {
                    // Wall-time bounds wide enough for any zone (±14 h) and the event's length.
                    let wall = |ms: i64| chrono::DateTime::from_timestamp_millis(ms).map(|t| t.naive_utc()).unwrap_or_default();
                    let lo = wall(from - length - DAY_MS);
                    let hi = wall(to + DAY_MS);
                    let tz = match &m.start {
                        When::Zoned(_, tz) => Some(tz.as_str()),
                        _ => None,
                    };
                    let to_utc = |t: NaiveDateTime| wall(zones.utc_ms(t, tz));
                    let mut wall_starts = rule.expand(m.start.naive(), lo, hi, &to_utc);
                    wall_starts.extend(m.rdates.iter().map(When::naive).filter(|t| *t >= lo && *t < hi));
                    let ex: HashSet<i64> = m.exdates.iter().map(|w| w.unix_ms(&zones)).collect();
                    let ex_dates: HashSet<chrono::NaiveDate> = m.exdates.iter().filter(|w| w.is_date()).map(|w| w.naive().date()).collect();
                    let mut starts: Vec<i64> = wall_starts
                        .into_iter()
                        .filter(|t| !ex_dates.contains(&t.date()))
                        .map(|t| if m.start.is_date() { zones.utc_ms(t, None) } else { zones.utc_ms(t, tz) })
                        .filter(|s| !ex.contains(s) && !moved.contains(&(m.uid.as_str(), *s)))
                        .collect();
                    starts.sort_unstable();
                    starts.dedup();
                    starts
                }
                _ => vec![m.start.unix_ms(&zones)],
            };
            for s in starts {
                let e = s + length;
                // Overlapping the window; a zero-length event counts at its moment.
                let overlaps = s < to && if length > 0 { e > from } else { s >= from };
                if !overlaps {
                    continue;
                }
                let original = m.recurrence_id.as_ref().map_or(s, |r| r.unix_ms(&zones));
                out.push(Event {
                    key: format!("{}|{}|{}", self.id, m.uid, original),
                    title: m.title.clone(),
                    start: s.max(0) as u64,
                    end: e.max(s).max(0) as u64,
                    all_day: m.start.is_date(),
                    place: m.place.clone(),
                    link: m.link.clone(),
                });
            }
        }
        out.sort_by(|a, b| (a.start, a.end, &a.title).cmp(&(b.start, b.end, &b.title)));
        out
    }

    /// How long each occurrence lasts (E-C1): DTEND − DTSTART, else DURATION, else none
    /// (timed) or a day (all-day).
    fn length(&self, m: &Master, zones: &dyn Resolve) -> i64 {
        if let Some(end) = &m.end {
            let l = end.unix_ms(zones) - m.start.unix_ms(zones);
            if l >= 0 {
                return l;
            }
        }
        m.duration_ms.filter(|d| *d >= 0).unwrap_or(if m.start.is_date() { DAY_MS } else { 0 })
    }
}

/// What the PC itself resolves (ICU, local time).
impl Source for Feed {
    fn events(&self, from: u64, to: u64) -> Vec<Event> {
        self.occurrences(from, to, &zones::System)
    }
}

fn master(c: &Component, owner: Option<&str>) -> Option<Master> {
    let start = When::from_prop(c.prop("DTSTART")?)?;
    let uid = c.text("UID").unwrap_or_else(|| format!("{:?}", start.naive()));
    let declined = owner.is_some_and(|me| {
        c.props("ATTENDEE").any(|a| a.value.trim().to_ascii_lowercase().trim_start_matches("mailto:") == me && a.param("PARTSTAT").is_some_and(|s| s.eq_ignore_ascii_case("DECLINED")))
    });
    let location = c.text("LOCATION");
    // The dedicated properties first, then where people paste links.
    let alt = c.prop("DESCRIPTION").and_then(|d| d.param("ALTREP")).map(str::to_string);
    let fields: Vec<String> = ["X-MICROSOFT-SKYPETEAMSMEETINGURL", "X-GOOGLE-CONFERENCE", "CONFERENCE", "URL", "LOCATION", "DESCRIPTION", "X-ALT-DESC"].iter().filter_map(|n| c.prop(n).map(|p| p.text())).chain(alt).collect();
    Some(Master {
        title: c.text("SUMMARY").map(|s| s.trim().to_string()).unwrap_or_default(),
        end: c.prop("DTEND").and_then(When::from_prop),
        duration_ms: c.prop("DURATION").and_then(|d| time::duration_ms(&d.value)),
        rule: c.prop("RRULE").and_then(|r| rrule::parse(&r.value)),
        rdates: c.props("RDATE").flat_map(When::list).collect(),
        exdates: c.props("EXDATE").flat_map(When::list).collect(),
        recurrence_id: c.prop("RECURRENCE-ID").and_then(When::from_prop),
        cancelled: c.prop("STATUS").is_some_and(|s| s.value.trim().eq_ignore_ascii_case("CANCELLED")),
        declined,
        place: location.as_deref().and_then(links::place),
        link: links::find(fields.iter().map(String::as_str)),
        uid,
        start,
    })
}

/// Wall times in the PC's zone for tests: a fixed offset, so they don't depend on where
/// they run.
#[cfg(test)]
pub struct Fixed(pub i64);

#[cfg(test)]
impl Resolve for Fixed {
    fn utc_ms(&self, local: NaiveDateTime, _tzid: Option<&str>) -> i64 {
        (local - chrono::Duration::hours(self.0)).and_utc().timestamp_millis()
    }
}

#[cfg(test)]
mod tests;
