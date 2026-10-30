//! Reminders (docs/PRD-reminders-activities.md): typed into the timer field (`call mom at
//! 6`, `in 2h check build`, `every day 21:00 meds`), kept with the timers in
//! `timer-state.json`, and fired as an Alert card with Snooze and Done.
//!
//! They live in the timers module because they share its field, its file, its sound and
//! its sleep-until-due loop; this folder holds their part of the module's state so
//! `timers/mod.rs` only calls in. Their truth is the wall clock (an event at a time of
//! day), so the wait is worked out again from it on every wake, a resume, and a clock or
//! time-zone change.
//!
//! Files: `grammar.rs` (the words), `when.rs` (which hour and day), `list.rs` (fire, snooze,
//! done), `card.rs` (how they read).

pub mod card;
pub mod grammar;
pub mod list;
pub mod when;

use chrono::{Datelike, Local, NaiveDate, NaiveDateTime, TimeZone};
use reui_protocol::{unix_ms, Headline, Priority};

use self::card::Shown;
use self::grammar::Reading;
use self::list::{Reminder, Reminders};
use self::when::Due;
use super::parse::{self, Parsed};
use super::{sound, view, State};
use crate::hub::{ModuleContext, View};

/// The fired card's activity key.
pub const KEY: &str = "reminder";

/// The reminders part of the timers module's state.
#[derive(Default)]
pub struct Desk {
    pub list: Reminders,
    /// The fired reminder the card shows (`None`: the oldest).
    page: Option<u32>,
    /// The one added last, for Ctrl+Z in the timer field.
    last_added: Option<u32>,
}

/// What a line in the timer field is, as far as reminders go.
pub enum Typed {
    Reminder(Due),
    Refused(&'static str),
    /// Not a reminder: a timer, focus, the stopwatch, or nothing yet.
    Other,
}

/// Reads a line from the timer field. A reminder that can't be yields to a timer that can
/// (`10m daily standup` is a timer labelled "Daily standup").
pub fn typed(text: &str) -> Typed {
    match grammar::read(text) {
        Reading::Not => Typed::Other,
        Reading::Reminder(spec) => match when::resolve(&spec, local_now(), clock24()) {
            Ok(due) => Typed::Reminder(due),
            Err(why) => Typed::Refused(why),
        },
        Reading::Refused(why) => match parse::parse(text) {
            Parsed::Timer { .. } | Parsed::Focus | Parsed::Stopwatch => Typed::Other,
            _ => Typed::Refused(why),
        },
    }
}

impl Desk {
    pub fn new(list: Reminders) -> Self {
        Self { list, page: None, last_added: None }
    }
}

impl State {
    /// When the next reminder fires, on the module's clock.
    pub(super) fn reminder_due(&self) -> Option<u64> {
        let due = self.remind.list.next_due()?;
        Some(self.now() + due.saturating_sub(unix_ms()))
    }

    /// Fires what's due; the timer sound once if the user wants it and nothing rings
    /// already. Returns whether anything fired.
    pub(super) fn fire_reminders(&mut self) -> bool {
        let (fired, on_time) = self.remind.list.fire(unix_ms());
        if fired > 0 {
            println!("timers: {fired} reminder(s) fired{}", if on_time == 0 { " (late)" } else { "" });
        }
        if on_time > 0 && self.settings.reminder_sound && self.ring_until.is_none() {
            sound::play(self.settings.sound.as_deref(), false);
        }
        fired > 0
    }

    pub(super) fn add_reminder(&mut self, due: Due) -> Result<(), &'static str> {
        let at = to_unix(due.at);
        let id = self.remind.list.add(due.text, at, due.repeat).ok_or("50 reminders is the limit")?;
        println!("timers: reminder set for {}", view::time_of_day(at));
        self.remind.last_added = Some(id);
        Ok(())
    }

    /// Ctrl+Z in the timer field: forget the reminder just added.
    pub(super) fn undo_reminder(&mut self) -> bool {
        let Some(id) = self.remind.last_added.take() else { return false };
        self.remind.list.remove(id).is_some()
    }

    /// The fired card's buttons and pager. Returns whether anything changed.
    pub(super) fn act_reminder(&mut self, action: &str) -> bool {
        let ringing: Vec<u32> = self.remind.list.ringing().iter().map(|r| r.id).collect();
        let at = self.remind.page.and_then(|p| ringing.iter().position(|id| *id == p)).unwrap_or(0);
        let Some(&id) = ringing.get(at) else { return false };
        match action {
            card::ACTION_DONE => {
                let next = self.remind.list.get(id).and_then(|r| r.repeat).map(|r| to_unix(when::next_after(r.every, r.hour, r.minute, local_now())));
                self.remind.list.done(id, next);
            }
            card::ACTION_SNOOZE => self.remind.list.snooze(id, unix_ms()),
            card::ACTION_NEXT | card::ACTION_PREV => {
                let step = if action == card::ACTION_NEXT { 1 } else { ringing.len() - 1 };
                self.remind.page = ringing.get((at + step) % ringing.len()).copied();
            }
            _ => return false,
        }
        if self.remind.page == Some(id) && !matches!(action, card::ACTION_NEXT | card::ACTION_PREV) {
            self.remind.page = None;
        }
        true
    }

    pub(super) fn publish_reminders(&self, ctx: &ModuleContext) {
        let ringing = self.remind.list.ringing();
        let at = self.remind.page.and_then(|p| ringing.iter().position(|r| r.id == p)).unwrap_or(0);
        match ringing.get(at) {
            Some(r) => ctx.publish(KEY, Priority::Alert, card::fired(&shown(r), at, ringing.len())),
            None => ctx.retract(KEY),
        }
    }

    /// The timer field reads a reminder being typed (hero: when it fires), and with the
    /// field empty lists the next few. `refused`: Enter was refused, and the detail line
    /// says why.
    pub(super) fn dress_launcher(&self, launcher: &mut View, query: &str, refused: bool) {
        let Some(card) = launcher.expanded.as_mut() else { return };
        if query.trim().is_empty() {
            let upcoming: Vec<Shown> = self.remind.list.upcoming().into_iter().map(shown).collect();
            card.rows.extend(card::upcoming(&upcoming));
            return;
        }
        let refused = match typed(query) {
            Typed::Reminder(due) => {
                let at = to_unix(due.at);
                let every = due.repeat.map(|r| r.every);
                let (eyebrow, hero, detail) = card::preview(&Shown { text: due.text.as_deref(), time: view::time_of_day(at), day: day_of(at), every, late: false });
                card.eyebrow = Some(eyebrow.into());
                card.headline = Headline::Value(hero);
                if !refused {
                    card.detail = detail;
                }
                return;
            }
            Typed::Refused(why) => why,
            Typed::Other => return,
        };
        card.eyebrow = Some("Reminder".into());
        card.headline = Headline::Value("–".into());
        card.detail = Some(refused.into());
    }
}

/// A reminder as the cards read it: its time in the user's format, its day in words.
fn shown(r: &Reminder) -> Shown<'_> {
    Shown { text: r.text.as_deref(), time: view::time_of_day(r.due_unix_ms), day: day_of(r.due_unix_ms), every: r.repeat.map(|r| r.every), late: r.late }
}

fn day_of(unix_ms: u64) -> Option<String> {
    let date = Local.timestamp_millis_opt(unix_ms as i64).single()?.date_naive();
    card_day(date, local_now().date())
}

/// "Tomorrow", "Yesterday", a weekday within the week either side, else "Mon 3 Nov";
/// `None` for today.
pub fn card_day(date: NaiveDate, today: NaiveDate) -> Option<String> {
    let days = (date - today).num_days();
    Some(match days {
        0 => return None,
        1 => "Tomorrow".into(),
        -1 => "Yesterday".into(),
        -6..=6 => date.weekday().to_string(),
        _ => format!("{} {} {}", date.weekday(), date.day(), date.format("%b")),
    })
}

fn local_now() -> NaiveDateTime {
    Local::now().naive_local()
}

/// A local time as Unix ms: an ambiguous one (the hour DST repeats) takes the earlier, one
/// in a DST gap the first valid minute after it.
fn to_unix(at: NaiveDateTime) -> u64 {
    let mut t = at;
    for _ in 0..180 {
        if let Some(local) = Local.from_local_datetime(&t).earliest() {
            return local.timestamp_millis().max(0) as u64;
        }
        t += chrono::Duration::minutes(1);
    }
    unix_ms()
}

/// Whether the user's time format is 24-hour (its pattern has `H`), which decides how a
/// written `9:30` reads.
fn clock24() -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::Globalization::{GetLocaleInfoEx, LOCALE_STIMEFORMAT};
    let mut buf = [0u16; 80];
    let n = unsafe { GetLocaleInfoEx(PCWSTR::null(), LOCALE_STIMEFORMAT, Some(&mut buf)) };
    n > 1 && String::from_utf16_lossy(&buf[..n as usize - 1]).contains('H')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_read_in_words() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 30).unwrap();
        let d = |n: i64| today + chrono::Duration::days(n);
        assert_eq!(card_day(today, today), None);
        assert_eq!(card_day(d(1), today).as_deref(), Some("Tomorrow"));
        assert_eq!(card_day(d(-1), today).as_deref(), Some("Yesterday"));
        assert_eq!(card_day(d(3), today).as_deref(), Some("Mon"));
        assert_eq!(card_day(d(10), today).as_deref(), Some("Mon 9 Nov"));
    }

    #[test]
    fn a_reminder_that_cant_be_yields_to_a_timer() {
        assert!(matches!(typed("10m daily standup"), Typed::Other));
        assert!(matches!(typed("every day meds"), Typed::Refused(_)));
        assert!(matches!(typed("10m tea"), Typed::Other));
        assert!(matches!(typed("in 2h check build"), Typed::Reminder(_)));
    }
}
