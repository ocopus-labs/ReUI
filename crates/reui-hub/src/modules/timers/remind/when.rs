//! When a reminder's words ([`Spec`]) fire, on the local wall clock: which hour a bare `6`
//! means, which day, and for a repeat the next time after a given moment. Pure over
//! `NaiveDateTime` (the caller passes "now" and the locale's 12/24-hour habit), so every
//! rule is tested at any hour without a clock.
//!
//! A bare hour is read the way people say it: on the same day, the next one to come round
//! (`at 6` at 14:00 is 18:00), unless the user's clock is 24-hour and minutes were written
//! (`9:30` is 09:30); on another day or a repeat, 7–11 is morning and 12–6 afternoon
//! (`tomorrow 9` is 9:00, `tomorrow 3` is 15:00).

use chrono::{Datelike, Days, Duration, NaiveDate, NaiveDateTime, NaiveTime, Weekday};

use super::grammar::{Day, Meridiem, Rule, Spec, Time, When};

/// How a reminder repeats, with the day a weekly one falls on settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Every {
    Day,
    Weekday,
    Week(Weekday),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Repeat {
    pub every: Every,
    pub hour: u32,
    pub minute: u32,
}

/// A reminder ready to keep: when it first fires, and how it repeats.
#[derive(Debug, Clone, PartialEq)]
pub struct Due {
    pub at: NaiveDateTime,
    pub text: Option<String>,
    pub repeat: Option<Repeat>,
}

/// Nothing further ahead than this.
const MAX_AHEAD_DAYS: i64 = 366;

pub fn resolve(spec: &Spec, now: NaiveDateTime, clock24: bool) -> Result<Due, &'static str> {
    let (at, repeat) = match &spec.when {
        When::In { ms } => {
            let wait = i64::try_from(*ms).ok().and_then(Duration::try_milliseconds).ok_or("Up to a year ahead")?;
            (now.checked_add_signed(wait).ok_or("Up to a year ahead")?, None)
        }
        When::At { day, time, repeat: Some(rule) } => {
            let (hour, minute) = settled(time, clock24);
            let every = match rule {
                Rule::Daily => Every::Day,
                Rule::Weekdays => Every::Weekday,
                Rule::Weekly(Some(d)) => Every::Week(*d),
                // "weekly": on the day the first one falls (a day typed beside it, else the
                // next time this comes round).
                Rule::Weekly(None) => match day {
                    Some(Day::Weekday(d)) => Every::Week(*d),
                    _ => Every::Week(next_after(Every::Day, hour, minute, now).weekday()),
                },
            };
            let repeat = Repeat { every, hour, minute };
            (next_after(every, hour, minute, now), Some(repeat))
        }
        When::At { day, time, repeat: None } => (once(*day, time, now, clock24)?, None),
    };
    if at <= now {
        return Err("That time has passed");
    }
    if at - now > Duration::days(MAX_AHEAD_DAYS) {
        return Err("Up to a year ahead");
    }
    Ok(Due { at, text: spec.text.clone(), repeat })
}

/// A one-off at a time of day.
fn once(day: Option<Day>, time: &Time, now: NaiveDateTime, clock24: bool) -> Result<NaiveDateTime, &'static str> {
    let today = now.date();
    match day {
        None => {
            // The soonest of the readings still ahead, today or tomorrow.
            let hours = candidates(time, clock24);
            let mut times: Vec<NaiveDateTime> = [today, tomorrow(today)].iter().flat_map(|d| hours.iter().filter_map(move |&h| at(*d, h, time.minute))).filter(|t| *t > now).collect();
            times.sort();
            times.first().copied().ok_or("That time has passed")
        }
        Some(Day::Today) => {
            let hours = candidates(time, clock24);
            hours.iter().filter_map(|&h| at(today, h, time.minute)).filter(|t| *t > now).min().ok_or("That time has passed")
        }
        Some(Day::Tonight) => {
            let (hour, minute) = match (time.meridiem, time.exact) {
                (None, false) if time.hour < 12 => (time.hour + 12, time.minute),
                _ => settled(time, clock24),
            };
            // "tonight 12" is the midnight that ends today.
            let date = if hour == 0 { tomorrow(today) } else { today };
            at(date, hour, minute).filter(|t| *t > now).ok_or("That time has passed")
        }
        Some(Day::Tomorrow) => {
            let (hour, minute) = settled(time, clock24);
            at(tomorrow(today), hour, minute).ok_or("That time has passed")
        }
        Some(Day::Weekday(d)) => {
            let (hour, minute) = settled(time, clock24);
            Ok(next_after(Every::Week(d), hour, minute, now))
        }
    }
}

/// The hours a written time could mean on the same day, as the PRD reads them.
fn candidates(time: &Time, clock24: bool) -> Vec<u32> {
    match time.meridiem {
        Some(m) => vec![with_meridiem(time.hour, m)],
        None if time.exact || (clock24 && !time.bare) => vec![time.hour],
        // 1–12 either way round: 6 is 6:00 or 18:00, 12 is noon or midnight.
        None => vec![time.hour % 12, time.hour % 12 + 12],
    }
}

/// One reading for a time on another day: 7–11 morning, 12 noon, 1–6 afternoon.
fn settled(time: &Time, clock24: bool) -> (u32, u32) {
    let hour = match time.meridiem {
        Some(m) => with_meridiem(time.hour, m),
        None if time.exact || (clock24 && !time.bare) => time.hour,
        None if (1..=6).contains(&time.hour) => time.hour + 12,
        None => time.hour,
    };
    (hour, time.minute)
}

fn with_meridiem(hour: u32, m: Meridiem) -> u32 {
    match m {
        Meridiem::Am => hour % 12,
        Meridiem::Pm => hour % 12 + 12,
    }
}

/// The first time after `after` that `every` falls on, at `hour:minute`.
pub fn next_after(every: Every, hour: u32, minute: u32, after: NaiveDateTime) -> NaiveDateTime {
    let mut date = after.date();
    for _ in 0..8 {
        let falls = match every {
            Every::Day => true,
            Every::Weekday => !matches!(date.weekday(), Weekday::Sat | Weekday::Sun),
            Every::Week(d) => date.weekday() == d,
        };
        if let Some(t) = at(date, hour, minute).filter(|t| falls && *t > after) {
            return t;
        }
        date = tomorrow(date);
    }
    // Every rule falls within 8 days; only a bad hour gets here.
    after + Duration::days(1)
}

fn at(date: NaiveDate, hour: u32, minute: u32) -> Option<NaiveDateTime> {
    NaiveTime::from_hms_opt(hour, minute, 0).map(|t| date.and_time(t))
}

fn tomorrow(date: NaiveDate) -> NaiveDate {
    date.checked_add_days(Days::new(1)).unwrap_or(date)
}

#[cfg(test)]
mod tests {
    use super::super::grammar::{read, Reading};
    use super::*;

    /// Friday 30 October 2026, at `h:m`.
    fn fri(h: u32, m: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, 30).unwrap().and_hms_opt(h, m, 0).unwrap()
    }

    fn day(d: u32, h: u32, m: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, d).or_else(|| NaiveDate::from_ymd_opt(2026, 11, d - 31)).unwrap().and_hms_opt(h, m, 0).unwrap()
    }

    fn when(input: &str, now: NaiveDateTime, clock24: bool) -> Result<Due, &'static str> {
        match read(input) {
            Reading::Reminder(spec) => resolve(&spec, now, clock24),
            other => panic!("{input:?} read as {other:?}"),
        }
    }

    fn fires(input: &str, now: NaiveDateTime, clock24: bool) -> NaiveDateTime {
        when(input, now, clock24).unwrap_or_else(|e| panic!("{input:?}: {e}")).at
    }

    #[test]
    fn a_bare_hour_is_the_next_one_round() {
        assert_eq!(fires("call mom at 6", fri(14, 0), false), fri(18, 0));
        assert_eq!(fires("call mom at 6", fri(5, 0), false), fri(6, 0));
        assert_eq!(fires("call mom at 6", fri(19, 0), false), day(31, 6, 0));
        // A bare hour reads the same in a 24-hour locale: people say "at 6".
        assert_eq!(fires("call mom at 6", fri(14, 0), true), fri(18, 0));
        assert_eq!(fires("lunch at 12", fri(9, 0), false), fri(12, 0));
        assert_eq!(fires("lunch at 12", fri(13, 0), false), day(31, 0, 0));
    }

    #[test]
    fn minutes_follow_the_locale() {
        // 24-hour: as written; 12-hour: the next one round.
        assert_eq!(fires("standup 9:30", fri(8, 0), true), fri(9, 30));
        assert_eq!(fires("standup 9:30", fri(10, 0), true), day(31, 9, 30));
        assert_eq!(fires("standup 9:30", fri(10, 0), false), fri(21, 30));
        assert_eq!(fires("at 18:00 take out trash", fri(10, 0), false), fri(18, 0));
        assert_eq!(fires("at 18:00 take out trash", fri(19, 0), true), day(31, 18, 0));
        assert_eq!(fires("at 06:30 run", fri(10, 0), false), day(31, 6, 30));
        assert_eq!(fires("6pm call", fri(10, 0), true), fri(18, 0));
        assert_eq!(fires("at 12am backup", fri(10, 0), false), day(31, 0, 0));
        assert_eq!(fires("at 12pm lunch", fri(10, 0), false), fri(12, 0));
        assert_eq!(fires("midnight backup", fri(10, 0), true), day(31, 0, 0));
    }

    #[test]
    fn other_days_read_morning_and_afternoon() {
        assert_eq!(fires("tomorrow 9 dentist", fri(22, 0), false), day(31, 9, 0));
        assert_eq!(fires("tomorrow 3 bank", fri(22, 0), false), day(31, 15, 0));
        assert_eq!(fires("tomorrow 12 lunch", fri(22, 0), false), day(31, 12, 0));
        assert_eq!(fires("tomorrow 9pm film", fri(8, 0), false), day(31, 21, 0));
        // Saturday is tomorrow; Friday is today while it's still ahead, else next week.
        assert_eq!(fires("sat 10 review", fri(8, 0), false), day(31, 10, 0));
        assert_eq!(fires("friday 10 review", fri(8, 0), false), fri(10, 0));
        assert_eq!(fires("friday 10 review", fri(11, 0), false), day(37, 10, 0));
        assert_eq!(fires("tonight 9 call", fri(8, 0), false), fri(21, 0));
        assert_eq!(fires("today 5pm ship", fri(8, 0), false), fri(17, 0));
        assert_eq!(fires("today 9 ship", fri(10, 0), false), fri(21, 0));
    }

    #[test]
    fn what_cant_be_says_why() {
        assert_eq!(when("today 8:00 ship", fri(10, 0), true), Err("That time has passed"));
        assert_eq!(when("tonight 9 call", fri(22, 0), false), Err("That time has passed"));
        assert_eq!(when("in 400 days renew", fri(10, 0), false), Err("Up to a year ahead"));
    }

    #[test]
    fn relative_waits_add_to_now() {
        assert_eq!(fires("in 2h check build", fri(23, 0), false), day(31, 1, 0));
        assert_eq!(fires("in 3 days renew", fri(10, 0), false), day(33, 10, 0));
    }

    #[test]
    fn repeats_find_their_first_day() {
        let daily = when("every day 21:00 meds", fri(22, 0), false).unwrap();
        assert_eq!((daily.at, daily.repeat), (day(31, 21, 0), Some(Repeat { every: Every::Day, hour: 21, minute: 0 })));
        assert_eq!(fires("every day 21:00 meds", fri(20, 0), false), fri(21, 0));
        // Weekdays skip the weekend: Monday 2 November, then Tuesday.
        let standup = when("weekdays 9:30 standup", fri(10, 0), true).unwrap();
        assert_eq!(standup.at, day(33, 9, 30));
        assert_eq!(next_after(Every::Weekday, 9, 30, standup.at), day(34, 9, 30));
        let review = when("every monday 10am review", fri(10, 0), false).unwrap();
        assert_eq!((review.at, review.repeat.unwrap().every), (day(33, 10, 0), Every::Week(Weekday::Mon)));
        // "weekly" falls on the first one's day.
        let weekly = when("weekly 9 review", fri(10, 0), false).unwrap();
        assert_eq!((weekly.at, weekly.repeat.unwrap().every), (day(31, 9, 0), Every::Week(Weekday::Sat)));
        assert_eq!(when("every day 8 vitamins", fri(10, 0), false).unwrap().repeat.unwrap().hour, 8);
        assert_eq!(when("every day 3 walk", fri(10, 0), false).unwrap().repeat.unwrap().hour, 15);
    }
}
