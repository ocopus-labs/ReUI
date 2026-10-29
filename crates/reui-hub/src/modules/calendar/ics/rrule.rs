//! RRULE (RFC 5545 §3.3.10) expanded in the event's own wall time, so a weekly 9:00
//! meeting stays at 9:00 across DST; the caller turns each occurrence into UTC through
//! its zone. What calendars really send is covered: DAILY / WEEKLY / MONTHLY / YEARLY with
//! INTERVAL, COUNT, UNTIL, BYDAY (ordinals too: "2nd Tuesday", "last Friday"), BYMONTHDAY
//! (negative from the month's end), BYMONTH, BYSETPOS ("last weekday of the month") and
//! WKST. Anything finer (BYHOUR, BYWEEKNO, sub-daily) marks the rule `unsupported`, and
//! the event then shows only its first occurrence (PRD E-C5).

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, Weekday};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

/// UNTIL as written: a date, a floating time, or a UTC time (`Z`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Until {
    Date(NaiveDate),
    Local(NaiveDateTime),
    Utc(NaiveDateTime),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub freq: Freq,
    pub interval: u32,
    pub count: Option<u32>,
    pub until: Option<Until>,
    /// (ordinal, weekday): `None` is every such weekday in the period.
    pub by_day: Vec<(Option<i32>, Weekday)>,
    pub by_month_day: Vec<i32>,
    pub by_month: Vec<u32>,
    pub by_set_pos: Vec<i32>,
    pub wkst: Weekday,
    pub unsupported: bool,
}

/// Periods looked at, at most: a rule with COUNT has to be walked from its start.
const MAX_PERIODS: u32 = 20_000;

pub fn parse(value: &str) -> Option<Rule> {
    let mut rule = Rule { freq: Freq::Daily, interval: 1, count: None, until: None, by_day: Vec::new(), by_month_day: Vec::new(), by_month: Vec::new(), by_set_pos: Vec::new(), wkst: Weekday::Mon, unsupported: false };
    let mut freq = None;
    for part in value.trim().split(';').filter(|p| !p.is_empty()) {
        let (k, v) = part.split_once('=')?;
        let v = v.trim();
        match k.trim().to_ascii_uppercase().as_str() {
            "FREQ" => {
                freq = Some(match v.to_ascii_uppercase().as_str() {
                    "DAILY" => Freq::Daily,
                    "WEEKLY" => Freq::Weekly,
                    "MONTHLY" => Freq::Monthly,
                    "YEARLY" => Freq::Yearly,
                    _ => {
                        rule.unsupported = true;
                        Freq::Daily
                    }
                })
            }
            "INTERVAL" => rule.interval = v.parse().ok().filter(|n| *n > 0).unwrap_or(1),
            "COUNT" => rule.count = v.parse().ok(),
            "UNTIL" => rule.until = until(v),
            "BYDAY" => rule.by_day = v.split(',').filter_map(by_day).collect(),
            "BYMONTHDAY" => rule.by_month_day = numbers(v).into_iter().filter(|d| (1..=31).contains(&d.abs())).collect(),
            "BYMONTH" => rule.by_month = numbers(v).into_iter().filter(|m| (1..=12).contains(m)).map(|m| m as u32).collect(),
            "BYSETPOS" => rule.by_set_pos = numbers(v).into_iter().filter(|p| *p != 0).collect(),
            "WKST" => rule.wkst = weekday(v).unwrap_or(Weekday::Mon),
            "BYHOUR" | "BYMINUTE" | "BYSECOND" | "BYWEEKNO" | "BYYEARDAY" => rule.unsupported = true,
            _ => {}
        }
    }
    rule.freq = freq?;
    Some(rule)
}

fn numbers(v: &str) -> Vec<i32> {
    v.split(',').filter_map(|n| n.trim().parse().ok()).collect()
}

fn weekday(v: &str) -> Option<Weekday> {
    Some(match v.trim().to_ascii_uppercase().as_str() {
        "MO" => Weekday::Mon,
        "TU" => Weekday::Tue,
        "WE" => Weekday::Wed,
        "TH" => Weekday::Thu,
        "FR" => Weekday::Fri,
        "SA" => Weekday::Sat,
        "SU" => Weekday::Sun,
        _ => return None,
    })
}

/// "MO", "2TU", "-1FR", "+3WE".
fn by_day(v: &str) -> Option<(Option<i32>, Weekday)> {
    let v = v.trim();
    let split = v.len().checked_sub(2)?;
    let day = weekday(v.get(split..)?)?;
    let n = v[..split].trim_start_matches('+');
    if n.is_empty() {
        return Some((None, day));
    }
    let n: i32 = n.parse().ok()?;
    (n != 0 && n.abs() <= 53).then_some((Some(n), day))
}

fn until(v: &str) -> Option<Until> {
    let v = v.trim();
    if v.len() == 8 {
        return NaiveDate::parse_from_str(v, "%Y%m%d").ok().map(Until::Date);
    }
    let (body, utc) = match v.strip_suffix(['Z', 'z']) {
        Some(b) => (b, true),
        None => (v, false),
    };
    let t = NaiveDateTime::parse_from_str(body, "%Y%m%dT%H%M%S").ok()?;
    Some(if utc { Until::Utc(t) } else { Until::Local(t) })
}

impl Rule {
    /// Occurrences (wall time) from `start` (the first, DTSTART itself, always counts)
    /// falling in `from..to`. `utc` turns a wall time into UTC, to compare with a UTC
    /// UNTIL. With `unsupported`, only `start`.
    pub fn expand(&self, start: NaiveDateTime, from: NaiveDateTime, to: NaiveDateTime, utc: &dyn Fn(NaiveDateTime) -> NaiveDateTime) -> Vec<NaiveDateTime> {
        let mut out = Vec::new();
        let within = |t: NaiveDateTime| t >= from && t < to;
        if self.unsupported {
            if within(start) {
                out.push(start);
            }
            return out;
        }
        let past_until = |t: NaiveDateTime| match self.until {
            Some(Until::Date(d)) => t.date() > d,
            Some(Until::Local(u)) => t > u,
            Some(Until::Utc(u)) => utc(t) > u,
            None => false,
        };
        let mut seen = 0u32;
        // Without a COUNT nothing before the window matters, so start a period before it.
        let first = if self.count.is_none() { self.skip_to(start, from) } else { 0 };
        // DTSTART is the first occurrence even when the rule wouldn't produce it, and it
        // counts towards COUNT (§3.8.5.3).
        if first == 0 && !self.on_rule(start) {
            seen = 1;
            if within(start) && !past_until(start) {
                out.push(start);
            }
        }
        for p in first..first + MAX_PERIODS {
            let Some(mut dates) = self.period(start, p) else { break };
            dates.sort();
            dates.dedup();
            let dates = self.set_pos(dates);
            let mut beyond = false;
            for date in dates {
                let t = date.and_time(start.time());
                if t < start {
                    continue;
                }
                if past_until(t) || t >= to || self.count.is_some_and(|c| seen >= c) {
                    beyond = true;
                    break;
                }
                seen += 1;
                if within(t) {
                    out.push(t);
                }
            }
            if beyond {
                break;
            }
        }
        out
    }

    /// Whether the rule's own first period produces `start`.
    fn on_rule(&self, start: NaiveDateTime) -> bool {
        self.period(start, 0).is_some_and(|mut d| {
            d.sort();
            d.dedup();
            self.set_pos(d).contains(&start.date())
        })
    }

    /// The period a little before `from`, so walking starts there.
    fn skip_to(&self, start: NaiveDateTime, from: NaiveDateTime) -> u32 {
        if from <= start {
            return 0;
        }
        let i = i64::from(self.interval);
        let periods = match self.freq {
            Freq::Daily => (from.date() - start.date()).num_days() / i,
            Freq::Weekly => (from.date() - start.date()).num_days() / 7 / i,
            Freq::Monthly => months_between(start.date(), from.date()) / i,
            Freq::Yearly => i64::from(from.year() - start.year()) / i,
        };
        (periods - 1).clamp(0, i64::from(u32::MAX / 2)) as u32
    }

    /// The candidate dates of period `p` (before BYSETPOS), or `None` past the calendar.
    fn period(&self, start: NaiveDateTime, p: u32) -> Option<Vec<NaiveDate>> {
        let step = i64::from(p) * i64::from(self.interval);
        let d0 = start.date();
        let in_month = |d: &NaiveDate| self.by_month.is_empty() || self.by_month.contains(&d.month());
        let dates = match self.freq {
            Freq::Daily => {
                let d = d0.checked_add_signed(Duration::days(step))?;
                let ok = in_month(&d) && (self.by_month_day.is_empty() || self.by_month_day.iter().any(|&md| month_day(d.year(), d.month(), md) == Some(d))) && (self.by_day.is_empty() || self.by_day.iter().any(|(_, w)| *w == d.weekday()));
                if ok { vec![d] } else { Vec::new() }
            }
            Freq::Weekly => {
                let back = (7 + d0.weekday().num_days_from_monday() as i64 - self.wkst.num_days_from_monday() as i64) % 7;
                let week = d0.checked_sub_signed(Duration::days(back))?.checked_add_signed(Duration::weeks(step))?;
                let days: Vec<Weekday> = if self.by_day.is_empty() { vec![d0.weekday()] } else { self.by_day.iter().map(|(_, w)| *w).collect() };
                (0..7).filter_map(|i| week.checked_add_signed(Duration::days(i))).filter(|d| days.contains(&d.weekday()) && in_month(d)).collect()
            }
            Freq::Monthly => {
                let (y, m) = add_months(d0.year(), d0.month(), step)?;
                if !self.by_month.is_empty() && !self.by_month.contains(&m) {
                    return Some(Vec::new());
                }
                self.days_in_month(y, m, d0)
            }
            Freq::Yearly => {
                let y = d0.year().checked_add(i32::try_from(step).ok()?)?;
                if y > 9000 {
                    return None;
                }
                let months: Vec<u32> = if self.by_month.is_empty() { vec![d0.month()] } else { self.by_month.clone() };
                if self.by_month.is_empty() && !self.by_day.is_empty() && self.by_month_day.is_empty() {
                    // BYDAY alone in a yearly rule counts within the year.
                    year_weekdays(y, &self.by_day)
                } else {
                    months.into_iter().flat_map(|m| self.days_in_month(y, m, d0)).collect()
                }
            }
        };
        Some(dates)
    }

    /// The days of month `y`-`m` the BYMONTHDAY / BYDAY parts pick (both: those in both),
    /// else DTSTART's day of the month (none, in a month too short for it).
    fn days_in_month(&self, y: i32, m: u32, d0: NaiveDate) -> Vec<NaiveDate> {
        let by_md: Vec<NaiveDate> = self.by_month_day.iter().filter_map(|&md| month_day(y, m, md)).collect();
        let by_wd: Vec<NaiveDate> = month_weekdays(y, m, &self.by_day);
        match (self.by_month_day.is_empty(), self.by_day.is_empty()) {
            (true, true) => NaiveDate::from_ymd_opt(y, m, d0.day()).into_iter().collect(),
            (false, true) => by_md,
            (true, false) => by_wd,
            (false, false) => by_md.into_iter().filter(|d| by_wd.contains(d)).collect(),
        }
    }

    /// BYSETPOS over a period's sorted dates: 1 is the first, -1 the last.
    fn set_pos(&self, dates: Vec<NaiveDate>) -> Vec<NaiveDate> {
        if self.by_set_pos.is_empty() {
            return dates;
        }
        let n = dates.len() as i32;
        let mut picked: Vec<NaiveDate> = self
            .by_set_pos
            .iter()
            .filter_map(|&p| {
                let i = if p > 0 { p - 1 } else { n + p };
                (0..n).contains(&i).then(|| dates[i as usize])
            })
            .collect();
        picked.sort();
        picked.dedup();
        picked
    }
}

/// Day `md` of a month; negative counts from its end (-1 is the last day).
fn month_day(y: i32, m: u32, md: i32) -> Option<NaiveDate> {
    let len = days_in(y, m) as i32;
    let day = if md > 0 { md } else { len + 1 + md };
    (1..=len).contains(&day).then(|| NaiveDate::from_ymd_opt(y, m, day as u32)).flatten()
}

fn days_in(y: i32, m: u32) -> u32 {
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    NaiveDate::from_ymd_opt(ny, nm, 1).and_then(|d| d.pred_opt()).map_or(28, |d| d.day())
}

/// The weekdays of a month BYDAY picks: every one, or the nth (negative from the end).
fn month_weekdays(y: i32, m: u32, by_day: &[(Option<i32>, Weekday)]) -> Vec<NaiveDate> {
    let all: Vec<NaiveDate> = (1..=days_in(y, m)).filter_map(|d| NaiveDate::from_ymd_opt(y, m, d)).collect();
    nth_of(&all, by_day)
}

fn year_weekdays(y: i32, by_day: &[(Option<i32>, Weekday)]) -> Vec<NaiveDate> {
    let Some(first) = NaiveDate::from_ymd_opt(y, 1, 1) else { return Vec::new() };
    let all: Vec<NaiveDate> = first.iter_days().take_while(|d| d.year() == y).collect();
    nth_of(&all, by_day)
}

fn nth_of(all: &[NaiveDate], by_day: &[(Option<i32>, Weekday)]) -> Vec<NaiveDate> {
    let mut out = Vec::new();
    for (n, w) in by_day {
        let days: Vec<NaiveDate> = all.iter().copied().filter(|d| d.weekday() == *w).collect();
        match n {
            None => out.extend(days),
            Some(n) => {
                let i = if *n > 0 { n - 1 } else { days.len() as i32 + n };
                if let Some(d) = usize::try_from(i).ok().and_then(|i| days.get(i)) {
                    out.push(*d);
                }
            }
        }
    }
    out
}

fn add_months(y: i32, m: u32, n: i64) -> Option<(i32, u32)> {
    let total = i64::from(y) * 12 + i64::from(m) - 1 + n;
    let y = i32::try_from(total.div_euclid(12)).ok()?;
    (y <= 9000).then_some((y, (total.rem_euclid(12) + 1) as u32))
}

fn months_between(a: NaiveDate, b: NaiveDate) -> i64 {
    (i64::from(b.year()) - i64::from(a.year())) * 12 + i64::from(b.month()) - i64::from(a.month())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    fn run(rule: &str, start: &str, from: &str, to: &str) -> Vec<String> {
        let r = parse(rule).unwrap();
        r.expand(t(start), t(from), t(to), &|x| x).iter().map(|x| x.format("%Y-%m-%d %H:%M").to_string()).collect()
    }

    #[test]
    fn weekly_on_weekdays_skips_to_the_window() {
        let got = run("FREQ=WEEKLY;BYDAY=MO,WE,FR", "2020-01-06 09:00", "2026-10-26 00:00", "2026-11-02 00:00");
        assert_eq!(got, ["2026-10-26 09:00", "2026-10-28 09:00", "2026-10-30 09:00"]);
        let every_other = run("FREQ=WEEKLY;INTERVAL=2;BYDAY=TU", "2026-10-06 10:00", "2026-10-01 00:00", "2026-11-01 00:00");
        assert_eq!(every_other, ["2026-10-06 10:00", "2026-10-20 10:00"]);
    }

    #[test]
    fn count_and_until_stop_it() {
        assert_eq!(run("FREQ=DAILY;COUNT=3", "2026-10-28 08:00", "2026-10-01 00:00", "2026-12-01 00:00").len(), 3);
        let until = run("FREQ=DAILY;UNTIL=20261030T080000Z", "2026-10-28 08:00", "2026-10-01 00:00", "2026-12-01 00:00");
        assert_eq!(until, ["2026-10-28 08:00", "2026-10-29 08:00", "2026-10-30 08:00"]);
        let date_until = run("FREQ=DAILY;UNTIL=20261029", "2026-10-28 08:00", "2026-10-01 00:00", "2026-12-01 00:00");
        assert_eq!(date_until.len(), 2);
        // COUNT counts from the start even when the window is later.
        assert!(run("FREQ=WEEKLY;COUNT=2", "2026-10-01 08:00", "2026-10-20 00:00", "2026-12-01 00:00").is_empty());
    }

    #[test]
    fn monthly_ordinals_last_days_and_setpos() {
        assert_eq!(run("FREQ=MONTHLY;BYDAY=2TU", "2026-01-13 15:00", "2026-10-01 00:00", "2026-12-01 00:00"), ["2026-10-13 15:00", "2026-11-10 15:00"]);
        assert_eq!(run("FREQ=MONTHLY;BYDAY=-1FR", "2026-01-30 15:00", "2026-10-01 00:00", "2026-11-01 00:00"), ["2026-10-30 15:00"]);
        assert_eq!(run("FREQ=MONTHLY;BYMONTHDAY=-1", "2026-01-31 09:00", "2026-02-01 00:00", "2026-03-01 00:00"), ["2026-02-28 09:00"]);
        assert_eq!(run("FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1", "2026-01-30 17:00", "2026-10-01 00:00", "2026-11-01 00:00"), ["2026-10-30 17:00"]);
        // The 31st doesn't exist in November: no occurrence that month.
        assert!(run("FREQ=MONTHLY", "2026-01-31 09:00", "2026-11-01 00:00", "2026-12-01 00:00").is_empty());
    }

    #[test]
    fn yearly_and_dtstart_counts_even_off_rule() {
        assert_eq!(run("FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU", "2000-03-26 02:00", "2026-01-01 00:00", "2027-01-01 00:00"), ["2026-03-29 02:00"]);
        assert_eq!(run("FREQ=YEARLY", "2024-02-29 09:00", "2025-01-01 00:00", "2029-01-01 00:00"), ["2028-02-29 09:00"]);
        // A Thursday start of a Mon/Wed rule: the start itself is the first occurrence.
        assert_eq!(run("FREQ=WEEKLY;BYDAY=MO,WE;COUNT=2", "2026-10-29 09:00", "2026-10-01 00:00", "2026-12-01 00:00"), ["2026-10-29 09:00", "2026-11-02 09:00"]);
    }

    #[test]
    fn unsupported_parts_give_the_first_only() {
        let r = parse("FREQ=HOURLY;INTERVAL=2").unwrap();
        assert!(r.unsupported);
        assert_eq!(run("FREQ=WEEKLY;BYWEEKNO=20", "2026-10-29 09:00", "2026-10-01 00:00", "2027-12-01 00:00").len(), 1);
        assert!(parse("INTERVAL=2").is_none(), "no FREQ, no rule");
    }
}
