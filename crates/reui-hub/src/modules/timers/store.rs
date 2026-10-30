//! Running and paused timers, a focus session, the stopwatch and reminders in
//! `%APPDATA%\ReUI\timer-state.json`, so they survive a restart or a reboot (S9). Written on
//! every change, never on a tick. Across a restart only the wall clock connects the two
//! runs, so a running countdown is saved as when it ends; while the hub runs, the monotonic
//! clock is the truth and a clock change moves nothing (E-T6). Reminders are wall-clock
//! events and are saved as such (their time, and a repeat's rule and time of day).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::focus::{Focus, Phase, Plan};
use super::model::{Finished, Timers};
use super::remind::list::{Reminder, Reminders};
use super::remind::when::{Every, Repeat};
use super::stopwatch::Stopwatch;

#[derive(Serialize, Deserialize, Default)]
struct File {
    #[serde(default)]
    timers: Vec<Saved>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    focus: Option<SavedFocus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stopwatch: Option<SavedStopwatch>,
    /// ReUI turned Do Not Disturb on and hasn't put it back yet (E-T14).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    dnd_ours: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    reminders: Vec<SavedReminder>,
}

#[derive(Serialize, Deserialize)]
struct SavedReminder {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    due_at_unix_ms: u64,
    /// `day`, `weekday`, or a weekday (`mon`) for weekly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    every: Option<String>,
    /// A repeat's time of day, `21:00`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    time: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    ringing: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    late: bool,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    length_ms: u64,
    /// Time left, for a paused timer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    left_ms: Option<u64>,
    /// When a running timer ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ends_at_unix_ms: Option<u64>,
}

#[derive(Serialize, Deserialize)]
struct SavedFocus {
    phase: String,
    done_work: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    left_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ends_at_unix_ms: Option<u64>,
}

#[derive(Serialize, Deserialize)]
struct SavedStopwatch {
    /// Elapsed at `saved_at_unix_ms`.
    elapsed_ms: u64,
    running: bool,
    saved_at_unix_ms: u64,
    #[serde(default)]
    laps: Vec<u64>,
}

/// Everything saved, as the module holds it.
pub struct State<'a> {
    pub timers: &'a Timers,
    pub focus: Option<&'a Focus>,
    pub stopwatch: Option<&'a Stopwatch>,
    pub dnd_ours: bool,
    pub reminders: &'a Reminders,
}

/// What comes back at start.
#[derive(Default)]
pub struct Loaded {
    /// Timers that ended while ReUI wasn't running.
    pub finished: Vec<Finished>,
    pub focus: Option<Focus>,
    pub stopwatch: Option<Stopwatch>,
    pub dnd_ours: bool,
    /// As saved; one that came due meanwhile fires (late) on the module's first tick.
    pub reminders: Reminders,
}

pub fn path() -> PathBuf {
    crate::paths::config().join("timer-state.json")
}

pub fn save(path: &Path, state: &State, now: u64, now_unix: u64) {
    let file = File {
        timers: state
            .timers
            .all()
            .iter()
            .map(|t| Saved {
                label: t.label.clone(),
                length_ms: t.length_ms,
                left_ms: (!t.running()).then(|| t.left(now)),
                ends_at_unix_ms: t.running().then(|| now_unix + t.left(now)),
            })
            .collect(),
        // A session waiting on its next phase isn't worth bringing back after a restart.
        focus: state.focus.filter(|f| !f.waiting()).map(|f| SavedFocus {
            phase: match f.phase {
                Phase::Work => "work",
                Phase::ShortBreak => "short-break",
                Phase::LongBreak => "long-break",
            }
            .into(),
            done_work: f.done_work,
            left_ms: (!f.running()).then(|| f.left(now)),
            ends_at_unix_ms: f.running().then(|| now_unix + f.left(now)),
        }),
        stopwatch: state.stopwatch.map(|s| SavedStopwatch { elapsed_ms: s.elapsed(now), running: s.running(), saved_at_unix_ms: now_unix, laps: s.laps.clone() }),
        dnd_ours: state.dnd_ours,
        reminders: state.reminders.all().iter().map(save_reminder).collect(),
    };
    let Ok(text) = serde_json::to_string_pretty(&file) else { return };
    let tmp = path.with_extension("json.tmp");
    if let Err(e) = std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, path)) {
        eprintln!("timers: can't save {}: {e}", path.display());
    }
}

/// The saved state, put back on the monotonic clock at `now`. Timers that ended while ReUI
/// wasn't running come back as late finishes (E-T2); a focus phase that ended then ends the
/// session (nobody was there to start the next).
pub fn load(path: &Path, timers: &mut Timers, plan: Plan, now: u64, now_unix: u64) -> Loaded {
    let file: File = match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
            eprintln!("timers: {} isn't readable ({e}); starting with none", path.display());
            File::default()
        }),
        Err(_) => File::default(),
    };
    let mut loaded = Loaded { dnd_ours: file.dnd_ours, ..Loaded::default() };
    for saved in file.timers {
        match (saved.ends_at_unix_ms, saved.left_ms) {
            (Some(end), _) if end <= now_unix => loaded.finished.push(Finished { label: saved.label, length_ms: saved.length_ms, late_ms: now_unix - end }),
            (Some(end), _) => _ = timers.add(saved.label, saved.length_ms, end - now_unix, Some(now)),
            (None, Some(left)) if left > 0 => _ = timers.add(saved.label, saved.length_ms, left, None),
            _ => {}
        }
    }
    loaded.focus = file.focus.and_then(|f| {
        let phase = match f.phase.as_str() {
            "work" => Phase::Work,
            "short-break" => Phase::ShortBreak,
            "long-break" => Phase::LongBreak,
            _ => return None,
        };
        match (f.ends_at_unix_ms, f.left_ms) {
            (Some(end), _) if end > now_unix => Some(Focus::restore(plan, phase, f.done_work, end - now_unix, Some(now))),
            (None, Some(left)) if left > 0 => Some(Focus::restore(plan, phase, f.done_work, left, None)),
            _ => None,
        }
    });
    for saved in file.reminders {
        if let Some(r) = load_reminder(saved) {
            _ = loaded.reminders.restore(r);
        }
    }
    loaded.stopwatch = file.stopwatch.map(|s| {
        let elapsed = if s.running { s.elapsed_ms + now_unix.saturating_sub(s.saved_at_unix_ms) } else { s.elapsed_ms };
        Stopwatch::restore(elapsed, s.running.then_some(now), s.laps)
    });
    loaded
}

fn save_reminder(r: &Reminder) -> SavedReminder {
    let every = r.repeat.map(|r| match r.every {
        Every::Day => "day".to_string(),
        Every::Weekday => "weekday".to_string(),
        Every::Week(d) => d.to_string().to_lowercase(),
    });
    let time = r.repeat.map(|r| format!("{:02}:{:02}", r.hour, r.minute));
    SavedReminder { text: r.text.clone(), due_at_unix_ms: r.due_unix_ms, every, time, ringing: r.ringing, late: r.late }
}

/// A saved reminder, or `None` when its repeat doesn't read (a newer version's rule).
fn load_reminder(s: SavedReminder) -> Option<Reminder> {
    let repeat = match (s.every.as_deref(), s.time.as_deref()) {
        (None, _) => None,
        (Some(every), Some(time)) => {
            let every = match every {
                "day" => Every::Day,
                "weekday" => Every::Weekday,
                d => Every::Week(d.parse().ok()?),
            };
            let (h, m) = time.split_once(':')?;
            let (hour, minute) = (h.parse().ok().filter(|h| *h < 24)?, m.parse().ok().filter(|m| *m < 60)?);
            Some(Repeat { every, hour, minute })
        }
        (Some(_), None) => return None,
    };
    let text = s.text.map(|t| t.chars().filter(|c| !c.is_control()).take(super::remind::grammar::MAX_TEXT).collect());
    Some(Reminder { id: 0, text, due_unix_ms: s.due_at_unix_ms, repeat, ringing: s.ringing, late: s.late })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60_000;
    const PLAN: Plan = Plan { work_ms: 25 * MIN, short_break_ms: 5 * MIN, long_break_ms: 15 * MIN, rounds: 4 };

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("reui-timers-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("timer-state.json")
    }

    fn save_timers(path: &Path, timers: &Timers, now: u64, now_unix: u64) {
        save(path, &State { timers, focus: None, stopwatch: None, dnd_ours: false, reminders: &Reminders::default() }, now, now_unix);
    }

    #[test]
    fn round_trip_and_late_finishes() {
        let path = temp("timers");
        let mut t = Timers::default();
        t.start(Some("Tea".into()), 4 * MIN, 0);
        let bread = t.start(Some("Bread".into()), 40 * MIN, 0).unwrap();
        t.toggle(bread, MIN);
        save_timers(&path, &t, MIN, 1_000_000);

        // Back 10 minutes later: tea ended 7 minutes ago, bread is still paused at 39 min.
        let mut back = Timers::default();
        let loaded = load(&path, &mut back, PLAN, 5, 1_000_000 + 10 * MIN);
        assert_eq!(loaded.finished, vec![Finished { label: Some("Tea".into()), length_ms: 4 * MIN, late_ms: 7 * MIN }]);
        let left: Vec<(u64, bool)> = back.all().iter().map(|t| (t.left(5), t.running())).collect();
        assert_eq!(left, vec![(39 * MIN, false)]);

        // Back a minute later: tea runs on with 2 minutes left.
        let mut soon = Timers::default();
        assert!(load(&path, &mut soon, PLAN, 0, 1_000_000 + MIN).finished.is_empty());
        assert_eq!(soon.ordered(0)[0].left(0), 2 * MIN);

        std::fs::write(&path, "{ broken").unwrap();
        assert!(load(&path, &mut Timers::default(), PLAN, 0, 0).finished.is_empty());
        _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn focus_and_stopwatch_come_back() {
        let path = temp("focus");
        let timers = Timers::default();
        let focus = Focus::start(PLAN, 0);
        let watch = Stopwatch::start(0);
        save(&path, &State { timers: &timers, focus: Some(&focus), stopwatch: Some(&watch), dnd_ours: true, reminders: &Reminders::default() }, 5 * MIN, 1_000_000);

        let loaded = load(&path, &mut Timers::default(), PLAN, 0, 1_000_000 + MIN);
        let f = loaded.focus.unwrap();
        assert_eq!((f.phase, f.left(0), f.running()), (Phase::Work, 19 * MIN, true));
        assert_eq!(loaded.stopwatch.unwrap().elapsed(0), 6 * MIN);
        assert!(loaded.dnd_ours);

        // The phase ended while ReUI was closed: the session is over.
        let later = load(&path, &mut Timers::default(), PLAN, 0, 1_000_000 + 30 * MIN);
        assert!(later.focus.is_none());
        _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn reminders_come_back_with_their_repeat() {
        let path = temp("reminders");
        let mut list = Reminders::default();
        list.add(Some("Call".into()), 2_000_000, None);
        let meds = list.add(Some("Meds".into()), 3_000_000, Some(Repeat { every: Every::Week(chrono::Weekday::Mon), hour: 21, minute: 5 })).unwrap();
        list.fire(1_500_000);
        list.fire(3_000_000);
        save(&path, &State { timers: &Timers::default(), focus: None, stopwatch: None, dnd_ours: false, reminders: &list }, 0, 1_000_000);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"every\": \"mon\"") && text.contains("\"time\": \"21:05\""), "{text}");

        let back = load(&path, &mut Timers::default(), PLAN, 0, 1_000_000).reminders;
        let all: Vec<(Option<&str>, u64, bool)> = back.all().iter().map(|r| (r.text.as_deref(), r.due_unix_ms, r.ringing)).collect();
        assert_eq!(all, [(Some("Call"), 2_000_000, true), (Some("Meds"), 3_000_000, true)]);
        assert_eq!(back.all()[1].repeat, list.get(meds).unwrap().repeat);
        _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
