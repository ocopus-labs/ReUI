//! Times in a feed (RFC 5545 §3.3.4–5, §3.6.5): DATE (all-day), DATE-TIME in UTC (`Z`),
//! floating (the reader's own zone) or in a named zone (`TZID`), DURATION, and the feed's
//! own zone definitions (`VTIMEZONE`). A `VTIMEZONE` is what the organiser's calendar says
//! the zone is, so it's preferred over any outside database: Outlook names zones in its
//! own words ("(UTC+01:00) Amsterdam, Berlin, …") that no database knows.

use chrono::{Datelike, NaiveDate, NaiveDateTime};

use super::lines::{Component, Property};
use super::rrule::{self, Rule};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum When {
    Date(NaiveDate),
    Utc(NaiveDateTime),
    Floating(NaiveDateTime),
    Zoned(NaiveDateTime, String),
}

impl When {
    /// A DTSTART-like property: `VALUE=DATE`, or an 8-digit value, is a date.
    pub fn from_prop(p: &Property) -> Option<When> {
        let v = p.value.trim();
        if p.param("VALUE").is_some_and(|t| t.eq_ignore_ascii_case("DATE")) || v.len() == 8 {
            return NaiveDate::parse_from_str(&v[..v.len().min(8)], "%Y%m%d").ok().map(When::Date);
        }
        let (body, utc) = match v.strip_suffix(['Z', 'z']) {
            Some(b) => (b, true),
            None => (v, false),
        };
        // Seconds are sometimes missing in the wild.
        let t = NaiveDateTime::parse_from_str(body, "%Y%m%dT%H%M%S").or_else(|_| NaiveDateTime::parse_from_str(body, "%Y%m%dT%H%M")).ok()?;
        Some(match (utc, p.param("TZID")) {
            (true, _) => When::Utc(t),
            (false, Some(tz)) if !tz.trim().is_empty() => When::Zoned(t, tz.trim().trim_start_matches('/').to_string()),
            (false, _) => When::Floating(t),
        })
    }

    /// Every time in an EXDATE / RDATE line (comma-separated, sharing its parameters).
    pub fn list(p: &Property) -> Vec<When> {
        p.value
            .split(',')
            .filter_map(|v| When::from_prop(&Property { name: p.name.clone(), params: p.params.clone(), value: v.to_string() }))
            .collect()
    }

    pub fn is_date(&self) -> bool {
        matches!(self, When::Date(_))
    }

    /// The wall time it names (midnight for a date).
    pub fn naive(&self) -> NaiveDateTime {
        match self {
            When::Date(d) => d.and_hms_opt(0, 0, 0).unwrap_or_default(),
            When::Utc(t) | When::Floating(t) | When::Zoned(t, _) => *t,
        }
    }
}

/// Turns a wall time into Unix ms: in the named zone, or the PC's own for `None`.
pub trait Resolve {
    fn utc_ms(&self, local: NaiveDateTime, tzid: Option<&str>) -> i64;
}

impl When {
    pub fn unix_ms(&self, zones: &dyn Resolve) -> i64 {
        match self {
            When::Utc(t) => t.and_utc().timestamp_millis(),
            When::Zoned(t, tz) => zones.utc_ms(*t, Some(tz)),
            When::Date(_) | When::Floating(_) => zones.utc_ms(self.naive(), None),
        }
    }
}

/// `P1D`, `PT1H30M`, `-PT15M`, `P2W` in ms (§3.3.6).
pub fn duration_ms(v: &str) -> Option<i64> {
    let v = v.trim();
    let (sign, v) = match v.strip_prefix('-') {
        Some(rest) => (-1, rest),
        None => (1, v.strip_prefix('+').unwrap_or(v)),
    };
    let v = v.strip_prefix(['P', 'p'])?;
    let mut total: i64 = 0;
    let mut number = String::new();
    let mut in_time = false;
    for c in v.chars() {
        match c.to_ascii_uppercase() {
            'T' => in_time = true,
            d if d.is_ascii_digit() => number.push(d),
            unit => {
                let n: i64 = number.parse().ok()?;
                number.clear();
                let secs = match (unit, in_time) {
                    ('W', false) => 7 * 86_400,
                    ('D', false) => 86_400,
                    ('H', true) => 3_600,
                    ('M', true) => 60,
                    ('S', true) => 1,
                    _ => return None,
                };
                total += n * secs;
            }
        }
    }
    number.is_empty().then_some(sign * total * 1000)
}

/// A `VTIMEZONE`: its observances (standard and daylight), each an offset that starts at
/// an onset and recurs (usually yearly) from there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Zone {
    observances: Vec<Observance>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Observance {
    start: NaiveDateTime,
    /// Seconds east of UTC before and after the onset.
    from: i32,
    to: i32,
    rule: Option<Rule>,
    rdates: Vec<NaiveDateTime>,
}

impl Zone {
    pub fn from_component(c: &Component) -> Option<(String, Zone)> {
        let id = c.prop("TZID")?.value.trim().trim_start_matches('/').to_string();
        let observances: Vec<Observance> = c
            .children
            .iter()
            .filter(|o| o.kind == "STANDARD" || o.kind == "DAYLIGHT")
            .filter_map(|o| {
                Some(Observance {
                    start: When::from_prop(o.prop("DTSTART")?)?.naive(),
                    from: offset(&o.prop("TZOFFSETFROM")?.value)?,
                    to: offset(&o.prop("TZOFFSETTO")?.value)?,
                    rule: o.prop("RRULE").and_then(|r| rrule::parse(&r.value)),
                    rdates: o.props("RDATE").flat_map(When::list).map(|w| w.naive()).collect(),
                })
            })
            .collect();
        (!observances.is_empty()).then_some((id, Zone { observances }))
    }

    /// Onsets around `year` as (UTC seconds, offset before, offset after), in order.
    fn transitions(&self, year: i32) -> Vec<(i64, i32, i32)> {
        let from = NaiveDate::from_ymd_opt(year - 1, 1, 1).and_then(|d| d.and_hms_opt(0, 0, 0));
        let to = NaiveDate::from_ymd_opt(year + 2, 1, 1).and_then(|d| d.and_hms_opt(0, 0, 0));
        let (Some(from), Some(to)) = (from, to) else { return Vec::new() };
        let mut out = Vec::new();
        for o in &self.observances {
            let mut onsets = match &o.rule {
                Some(rule) => rule.expand(o.start, from, to, &|t| t - chrono::Duration::seconds(i64::from(o.from))),
                None => vec![o.start],
            };
            onsets.extend(o.rdates.iter().copied());
            out.extend(onsets.into_iter().map(|t| (t.and_utc().timestamp() - i64::from(o.from), o.from, o.to)));
        }
        out.sort_unstable();
        out
    }

    /// The wall time `local` in this zone as Unix ms. An hour that happens twice (the
    /// autumn change) reads as the first; one skipped (spring) with the offset from before
    /// the change, an hour later on the clock (PRD E-C3).
    pub fn utc_ms(&self, local: NaiveDateTime) -> i64 {
        let l = local.and_utc().timestamp();
        let transitions = self.transitions(local.year());
        let base = self.observances.iter().min_by_key(|o| o.start).map_or(0, |o| o.from);
        let offset_at = |u: i64| transitions.iter().take_while(|t| t.0 <= u).last().map_or(base, |t| t.2);
        let mut candidates: Vec<i32> = transitions.iter().flat_map(|t| [t.1, t.2]).collect();
        candidates.push(base);
        candidates.sort_unstable_by(|a, b| b.cmp(a));
        candidates.dedup();
        let secs = candidates.iter().map(|&o| l - i64::from(o)).find(|&u| i64::from(offset_at(u)) == l - u).unwrap_or_else(|| l - i64::from(offset_at(l - 86_400)));
        secs * 1000 + i64::from(local.and_utc().timestamp_subsec_millis())
    }
}

/// `+0100`, `-0530`, `+013000` in seconds east of UTC.
fn offset(v: &str) -> Option<i32> {
    let v = v.trim();
    let (sign, digits) = match v.as_bytes().first()? {
        b'+' => (1, &v[1..]),
        b'-' => (-1, &v[1..]),
        _ => (1, v),
    };
    if digits.len() < 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let h: i32 = digits[0..2].parse().ok()?;
    let m: i32 = digits[2..4].parse().ok()?;
    let s: i32 = digits.get(4..6).and_then(|s| s.parse().ok()).unwrap_or(0);
    Some(sign * (h * 3600 + m * 60 + s))
}

#[cfg(test)]
mod tests {
    use super::super::lines;
    use super::*;

    fn t(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    fn utc(ms: i64) -> String {
        chrono::DateTime::from_timestamp_millis(ms).unwrap().format("%Y-%m-%d %H:%M").to_string()
    }

    /// Outlook's way: a Windows-worded TZID with its rules spelled out.
    const BERLIN: &str = "BEGIN:VTIMEZONE\r\nTZID:W. Europe Standard Time\r\nBEGIN:STANDARD\r\nDTSTART:16010101T030000\r\nTZOFFSETFROM:+0200\r\nTZOFFSETTO:+0100\r\nRRULE:FREQ=YEARLY;INTERVAL=1;BYDAY=-1SU;BYMONTH=10\r\nEND:STANDARD\r\nBEGIN:DAYLIGHT\r\nDTSTART:16010101T020000\r\nTZOFFSETFROM:+0100\r\nTZOFFSETTO:+0200\r\nRRULE:FREQ=YEARLY;INTERVAL=1;BYDAY=-1SU;BYMONTH=3\r\nEND:DAYLIGHT\r\nEND:VTIMEZONE\r\n";

    fn zone(text: &str) -> Zone {
        Zone::from_component(&lines::parse(text)[0]).unwrap().1
    }

    #[test]
    fn a_vtimezone_reads_summer_winter_and_the_changes() {
        let z = zone(BERLIN);
        assert_eq!(utc(z.utc_ms(t("2026-07-01 09:00"))), "2026-07-01 07:00");
        assert_eq!(utc(z.utc_ms(t("2026-11-02 09:00"))), "2026-11-02 08:00");
        // Oct 25, 2026: 03:00 summer time goes back to 02:00; 02:30 happens twice.
        assert_eq!(utc(z.utc_ms(t("2026-10-25 02:30"))), "2026-10-25 00:30", "the first of the two");
        // Mar 29, 2026: 02:00 jumps to 03:00; 02:30 doesn't exist.
        assert_eq!(utc(z.utc_ms(t("2026-03-29 02:30"))), "2026-03-29 01:30", "with the winter offset");
        assert_eq!(utc(z.utc_ms(t("2026-03-29 03:30"))), "2026-03-29 01:30");
    }

    #[test]
    fn a_zone_with_one_observance_is_a_fixed_offset() {
        let z = zone("BEGIN:VTIMEZONE\nTZID:Asia/Kolkata\nBEGIN:STANDARD\nDTSTART:19700101T000000\nTZOFFSETFROM:+0530\nTZOFFSETTO:+0530\nEND:STANDARD\nEND:VTIMEZONE\n");
        assert_eq!(utc(z.utc_ms(t("2026-10-30 15:30"))), "2026-10-30 10:00");
    }

    #[test]
    fn values_and_durations_parse() {
        let p = |line: &str| lines::parse(&format!("BEGIN:X\n{line}\nEND:X"))[0].props[0].clone();
        assert_eq!(When::from_prop(&p("DTSTART;VALUE=DATE:20261030")), Some(When::Date(NaiveDate::from_ymd_opt(2026, 10, 30).unwrap())));
        assert_eq!(When::from_prop(&p("DTSTART:20261030T090000Z")), Some(When::Utc(t("2026-10-30 09:00"))));
        assert_eq!(When::from_prop(&p("DTSTART;TZID=\"/America/New_York\":20261030T090000")), Some(When::Zoned(t("2026-10-30 09:00"), "America/New_York".into())));
        assert_eq!(When::list(&p("EXDATE:20261030T090000Z,20261031T090000Z")).len(), 2);
        assert_eq!(duration_ms("PT1H30M"), Some(90 * 60_000));
        assert_eq!(duration_ms("P1D"), Some(86_400_000));
        assert_eq!(duration_ms("-PT15M"), Some(-15 * 60_000));
        assert_eq!(duration_ms("P1W"), Some(7 * 86_400_000));
        assert_eq!(duration_ms("1H"), None);
        assert_eq!(offset("-0530"), Some(-(5 * 3600 + 1800)));
    }
}
