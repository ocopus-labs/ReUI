//! What a reminder typed into the timer field says (docs/PRD-reminders-activities.md §5.1):
//! a time (`at 6`, `6pm`, `18:00`, `noon`), a day (`tomorrow`, `fri`), a repeat (`every
//! day`, `weekdays`, `every monday`) or a relative wait (`in 2h`), and the rest as the text.
//! Only words: which hour a bare `6` means, and when that is, is `when.rs`'s job, so this
//! needs no clock and no locale.
//!
//! The grammar is kept small on purpose. A line is a reminder only when it has a marker a
//! timer never has (`at`, `in`, a day, a repeat, am/pm, `noon`, or `9:30` beside words), so
//! `10 tea` and a lone `5:00` stay timers.

use chrono::Weekday;

use super::super::parse;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Day {
    Today,
    /// Today, and a bare hour is in the evening.
    Tonight,
    Tomorrow,
    /// The next one (today, if the time is still ahead).
    Weekday(Weekday),
}

/// How it repeats, as typed. `Weekly(None)` is "weekly": on the day of the first one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    Daily,
    Weekdays,
    Weekly(Option<Weekday>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Meridiem {
    Am,
    Pm,
}

/// A time of day as written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Time {
    pub hour: u32,
    pub minute: u32,
    pub meridiem: Option<Meridiem>,
    /// Only an hour was written (`at 6`, not `6:00`).
    pub bare: bool,
    /// It can only be read one way: `0`, above 12, or with a leading zero (`06:30`).
    pub exact: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum When {
    At { day: Option<Day>, time: Time, repeat: Option<Rule> },
    /// `in 2h`: this long from now.
    In { ms: u64 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Spec {
    pub when: When,
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Reading {
    /// No reminder marker: maybe a timer.
    Not,
    Reminder(Spec),
    /// It reads as a reminder but can't be one; says why.
    Refused(&'static str),
}

/// Longest reminder text kept.
pub const MAX_TEXT: usize = 80;
const DAY_MS: u64 = 24 * 3_600_000;

pub fn read(input: &str) -> Reading {
    let words: Vec<&str> = input.split_whitespace().collect();
    if words.is_empty() {
        return Reading::Not;
    }
    let low: Vec<String> = words.iter().map(|w| w.to_lowercase()).collect();
    let mut used = vec![false; words.len()];

    if let Some((ms, from, to)) = relative(&words, &low) {
        used[from..to].iter_mut().for_each(|u| *u = true);
        return Reading::Reminder(Spec { when: When::In { ms }, text: text(&words, &used) });
    }

    let repeat = repeat(&low, &mut used);
    let day = day(&low, &mut used);
    let time = time_after_at(&low, &mut used).or_else(|| strong_time(&low, &mut used, repeat.is_some() || day.is_some())).or_else(|| {
        // With a day or a repeat, a bare hour beside it is the time ("tomorrow 9 dentist").
        if repeat.is_none() && day.is_none() {
            return None;
        }
        bare_hour(&low, &mut used)
    });
    match (time, day, repeat) {
        (None, None, None) => Reading::Not,
        (None, _, _) => Reading::Refused("Add a time, like tomorrow 9"),
        (Some(time), day, repeat) => Reading::Reminder(Spec { when: When::At { day, time, repeat }, text: text(&words, &used) }),
    }
}

/// `in <length>` at the start, or at the end: the wait and the words it took.
fn relative(words: &[&str], low: &[String]) -> Option<(u64, usize, usize)> {
    let n = words.len();
    if low[0] == "in" {
        if let Some((ms, k)) = (2..=n).rev().find_map(|k| wait(&words[1..k]).map(|ms| (ms, k))) {
            return Some((ms, 0, k));
        }
    }
    let at = low.iter().rposition(|w| w == "in").filter(|&i| i > 0 && i + 1 < n)?;
    wait(&words[at + 1..]).map(|ms| (ms, at, n))
}

/// A wait in days or weeks (`3 days`, `2w`), else the timer field's lengths (`2h`, `1h30`).
fn wait(words: &[&str]) -> Option<u64> {
    let joined = words.concat().to_lowercase();
    let digits = joined.find(|c: char| !c.is_ascii_digit()).unwrap_or(joined.len());
    let count = joined[..digits].parse::<u64>().ok();
    let per = match &joined[digits..] {
        "d" | "day" | "days" => Some(DAY_MS),
        "w" | "wk" | "week" | "weeks" => Some(7 * DAY_MS),
        _ => None,
    };
    let ms = match (count, per) {
        (Some(c), Some(per)) => c.checked_mul(per)?,
        _ => parse::length(words)?,
    };
    (ms > 0).then_some(ms)
}

fn repeat(low: &[String], used: &mut [bool]) -> Option<Rule> {
    for i in 0..low.len() {
        let next = low.get(i + 1).map(String::as_str);
        let (rule, width) = match (low[i].as_str(), next) {
            ("every", Some("day")) => (Rule::Daily, 2),
            ("every", Some("weekday" | "weekdays")) => (Rule::Weekdays, 2),
            ("every", Some("week")) => (Rule::Weekly(None), 2),
            ("every", Some(w)) if weekday(w).is_some() => (Rule::Weekly(weekday(w)), 2),
            ("daily" | "everyday", _) => (Rule::Daily, 1),
            ("weekdays", _) => (Rule::Weekdays, 1),
            ("weekly", _) => (Rule::Weekly(None), 1),
            _ => continue,
        };
        used[i..i + width].iter_mut().for_each(|u| *u = true);
        return Some(rule);
    }
    None
}

fn day(low: &[String], used: &mut [bool]) -> Option<Day> {
    for i in 0..low.len() {
        if used[i] {
            continue;
        }
        let day = match low[i].as_str() {
            "today" => Day::Today,
            "tonight" => Day::Tonight,
            "tomorrow" | "tmrw" | "tmr" => Day::Tomorrow,
            w => match weekday(w) {
                Some(d) => Day::Weekday(d),
                None => continue,
            },
        };
        used[i] = true;
        // "on friday"
        if i > 0 && !used[i - 1] && low[i - 1] == "on" {
            used[i - 1] = true;
        }
        return Some(day);
    }
    None
}

/// `at <time>`, the time in one word or two (`at 6 pm`).
fn time_after_at(low: &[String], used: &mut [bool]) -> Option<Time> {
    for i in 0..low.len().saturating_sub(1) {
        if used[i] || low[i] != "at" || used[i + 1] {
            continue;
        }
        if let Some((time, width)) = time_at(low, i + 1) {
            used[i..i + 1 + width].iter_mut().for_each(|u| *u = true);
            return Some(time);
        }
    }
    None
}

/// A time that can't be anything else: with am/pm, `noon`, `midnight`, or `h:mm` with other
/// words (alone, `5:00` is a five-minute timer).
fn strong_time(low: &[String], used: &mut [bool], dated: bool) -> Option<Time> {
    let others = |i: usize, width: usize| dated || (0..low.len()).any(|j| !used[j] && (j < i || j >= i + width));
    for i in 0..low.len() {
        if used[i] {
            continue;
        }
        let Some((time, width)) = time_at(low, i) else { continue };
        if (i..i + width).any(|j| used[j]) {
            continue;
        }
        let strong = time.meridiem.is_some() || matches!(low[i].as_str(), "noon" | "midnight") || (!time.bare && others(i, width));
        if strong {
            used[i..i + width].iter_mut().for_each(|u| *u = true);
            return Some(time);
        }
    }
    None
}

/// A bare hour (`9`): the word right after the day or repeat, else the last word.
fn bare_hour(low: &[String], used: &mut [bool]) -> Option<Time> {
    let after_marker = (1..low.len()).find(|&i| used[i - 1] && !used[i]);
    let last = low.len() - 1;
    for i in [after_marker, (!used[last]).then_some(last)].into_iter().flatten() {
        if let Some((time, 1)) = time_at(low, i).filter(|(t, _)| t.meridiem.is_none()) {
            used[i] = true;
            return Some(time);
        }
    }
    None
}

/// A time at `low[i]`: `6`, `6pm`, `6:30`, `6:30pm`, `18:00`, `noon`, `midnight`, or a
/// number followed by `am`/`pm` as its own word. Returns how many words it took.
fn time_at(low: &[String], i: usize) -> Option<(Time, usize)> {
    let word = low[i].as_str();
    match word {
        "noon" | "midday" => return Some((Time { hour: 12, minute: 0, meridiem: None, bare: false, exact: true }, 1)),
        "midnight" => return Some((Time { hour: 0, minute: 0, meridiem: None, bare: false, exact: true }, 1)),
        _ => {}
    }
    let (body, meridiem) = split_meridiem(word);
    let (body, meridiem, width) = match (meridiem, low.get(i + 1).map(|w| split_meridiem(w))) {
        (None, Some(("", Some(m)))) => (body, Some(m), 2),
        (m, _) => (body, m, 1),
    };
    let (h, m) = match body.split_once(':') {
        Some((h, m)) if m.len() == 2 => (h, Some(m)),
        Some(_) => return None,
        None => (body, None),
    };
    if h.is_empty() || h.len() > 2 || !h.bytes().all(|b| b.is_ascii_digit()) || !m.is_none_or(|m| m.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    let hour: u32 = h.parse().ok()?;
    let minute: u32 = m.map_or(Some(0), |m| m.parse().ok())?;
    if minute > 59 || hour > 23 || (meridiem.is_some() && !(1..=12).contains(&hour)) {
        return None;
    }
    let exact = meridiem.is_none() && (hour == 0 || hour > 12 || h.starts_with('0'));
    Some((Time { hour, minute, meridiem, bare: m.is_none(), exact }, width))
}

fn split_meridiem(word: &str) -> (&str, Option<Meridiem>) {
    for (suffix, m) in [("a.m.", Meridiem::Am), ("p.m.", Meridiem::Pm), ("am", Meridiem::Am), ("pm", Meridiem::Pm)] {
        if let Some(body) = word.strip_suffix(suffix) {
            return (body, Some(m));
        }
    }
    (word, None)
}

fn weekday(word: &str) -> Option<Weekday> {
    Some(match word {
        "mon" | "monday" => Weekday::Mon,
        "tue" | "tues" | "tuesday" => Weekday::Tue,
        "wed" | "wednesday" => Weekday::Wed,
        "thu" | "thur" | "thurs" | "thursday" => Weekday::Thu,
        "fri" | "friday" => Weekday::Fri,
        "sat" | "saturday" => Weekday::Sat,
        "sun" | "sunday" => Weekday::Sun,
        _ => return None,
    })
}

/// The words nothing else took, first letter capitalised, cut to [`MAX_TEXT`].
fn text(words: &[&str], used: &[bool]) -> Option<String> {
    let rest: Vec<&str> = words.iter().zip(used).filter(|(_, u)| !**u).map(|(w, _)| *w).collect();
    let joined: String = rest.join(" ").chars().filter(|c| !c.is_control()).take(MAX_TEXT).collect();
    let mut chars = joined.chars();
    let first = chars.next()?;
    Some(first.to_uppercase().chain(chars).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(input: &str) -> (Option<Day>, Time, Option<Rule>, Option<String>) {
        match read(input) {
            Reading::Reminder(Spec { when: When::At { day, time, repeat }, text }) => (day, time, repeat, text),
            other => panic!("{input:?} read as {other:?}"),
        }
    }

    fn hm(input: &str) -> (u32, u32, Option<Meridiem>) {
        let (_, t, _, _) = at(input);
        (t.hour, t.minute, t.meridiem)
    }

    fn text_of(input: &str) -> Option<String> {
        match read(input) {
            Reading::Reminder(spec) => spec.text,
            other => panic!("{input:?} read as {other:?}"),
        }
    }

    #[test]
    fn the_examples_in_the_prd() {
        assert_eq!(at("call mom at 6"), (None, Time { hour: 6, minute: 0, meridiem: None, bare: true, exact: false }, None, Some("Call mom".into())));
        assert_eq!(hm("standup 9:30"), (9, 30, None));
        assert_eq!(text_of("standup 9:30").as_deref(), Some("Standup"));
        assert_eq!(hm("at 18:00 take out trash"), (18, 0, None));
        assert_eq!(text_of("at 18:00 take out trash").as_deref(), Some("Take out trash"));
        assert_eq!(read("in 2h check build"), Reading::Reminder(Spec { when: When::In { ms: 2 * 3_600_000 }, text: Some("Check build".into()) }));
        let (day, time, _, text) = at("tomorrow 9 dentist");
        assert_eq!((day, time.hour, time.bare, text.as_deref()), (Some(Day::Tomorrow), 9, true, Some("Dentist")));
        let (day, time, repeat, text) = at("every day 21:00 meds");
        assert_eq!((day, time.hour, time.exact, repeat, text.as_deref()), (None, 21, true, Some(Rule::Daily), Some("Meds")));
    }

    #[test]
    fn times_in_every_form() {
        assert_eq!(hm("6pm call"), (6, 0, Some(Meridiem::Pm)));
        assert_eq!(hm("call 6 pm"), (6, 0, Some(Meridiem::Pm)));
        assert_eq!(hm("call at 6:30am"), (6, 30, Some(Meridiem::Am)));
        assert_eq!(hm("call at 6:30 p.m."), (6, 30, Some(Meridiem::Pm)));
        assert_eq!(hm("lunch noon"), (12, 0, None));
        assert_eq!(hm("midnight backup"), (0, 0, None));
        assert_eq!(hm("6PM Call"), (6, 0, Some(Meridiem::Pm)));
        assert!(at("at 06:30 run").1.exact);
        assert!(at("at 0:15 run").1.exact);
        assert!(!at("at 6:30 run").1.exact);
        assert_eq!(text_of("at 5pm").as_deref(), None, "a time alone is a reminder with no text");
    }

    #[test]
    fn days_and_weekdays() {
        assert_eq!(at("dentist friday 3pm").0, Some(Day::Weekday(Weekday::Fri)));
        assert_eq!(at("on fri at 3 dentist").0, Some(Day::Weekday(Weekday::Fri)));
        assert_eq!(text_of("on fri at 3 dentist").as_deref(), Some("Dentist"));
        assert_eq!(at("today 17:00 ship it").0, Some(Day::Today));
        assert_eq!(at("tonight 9 call").0, Some(Day::Tonight));
        assert_eq!(at("dentist tomorrow 9").1.hour, 9);
        assert_eq!(text_of("dentist tomorrow 9").as_deref(), Some("Dentist"));
        assert_eq!(read("tomorrow dentist"), Reading::Refused("Add a time, like tomorrow 9"));
    }

    #[test]
    fn repeats() {
        assert_eq!(at("daily 8 vitamins").2, Some(Rule::Daily));
        assert_eq!(at("every weekday 9:30 standup").2, Some(Rule::Weekdays));
        assert_eq!(at("weekdays 9:30 standup").2, Some(Rule::Weekdays));
        assert_eq!(at("every monday 10am review").2, Some(Rule::Weekly(Some(Weekday::Mon))));
        assert_eq!(at("weekly 10am review").2, Some(Rule::Weekly(None)));
        assert_eq!(text_of("meds every day at 21:00").as_deref(), Some("Meds"));
        assert_eq!(read("every day meds"), Reading::Refused("Add a time, like tomorrow 9"));
    }

    #[test]
    fn relative_waits() {
        let ms = |input: &str| match read(input) {
            Reading::Reminder(Spec { when: When::In { ms }, .. }) => ms,
            other => panic!("{input:?} read as {other:?}"),
        };
        assert_eq!(ms("in 90 min stretch"), 90 * 60_000);
        assert_eq!(ms("in 1h30 call back"), 90 * 60_000);
        assert_eq!(ms("in 3 days renew"), 3 * DAY_MS);
        assert_eq!(ms("in 2w invoice"), 14 * DAY_MS);
        assert_eq!(ms("check build in 2h"), 2 * 3_600_000);
        assert_eq!(text_of("check build in 2h").as_deref(), Some("Check build"));
        assert_eq!(ms("in 10"), 10 * 60_000);
    }

    #[test]
    fn timers_stay_timers() {
        for timer in ["10", "10m tea", "tea 4m", "5:00", "1:30:00", "focus", "sw", "2 hours", "½h", "tea", "plug in the kettle"] {
            assert_eq!(read(timer), Reading::Not, "{timer:?}");
        }
        assert_eq!(read(""), Reading::Not);
    }

    #[test]
    fn nonsense_is_not_a_time() {
        assert_eq!(read("look at 25:00 now"), Reading::Not);
        assert_eq!(read("at 13pm"), Reading::Not);
        assert_eq!(read("at 6:7"), Reading::Not);
        assert_eq!(read("at 6:75"), Reading::Not);
        assert_eq!(read("look at that"), Reading::Not);
        assert_eq!(text_of("meet at the cafe 5pm").as_deref(), Some("Meet at the cafe"));
    }

    #[test]
    fn text_is_capped_and_clean() {
        let long = format!("at 6 {}", "a".repeat(200));
        assert_eq!(text_of(&long).unwrap().chars().count(), MAX_TEXT);
        assert_eq!(text_of("at 6 éclair").as_deref(), Some("Éclair"));
    }
}
