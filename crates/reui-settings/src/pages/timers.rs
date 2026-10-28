//! Timers (`timers.json`): the shortcut that opens the timer field, the finish sound and how
//! long it rings, and focus sessions (lengths, auto-start, Do Not Disturb).

use serde_json::Value;

use super::{button, enable, flag, group, head, shortcut, Effect, Page};
use crate::model::files::File;
use crate::state::State;
use crate::ui::icons;
use crate::ui::{Act, Block, Part, Role, Row, Target};

/// The finish sounds: Windows' own, from `C:\Windows\Media`.
const SOUNDS: [&str; 21] = [
    "Alarm01", "Alarm02", "Alarm03", "Alarm04", "Alarm05", "Alarm06", "Alarm07", "Alarm08", "Alarm09", "Alarm10", "Ring01", "Ring02", "Ring03", "Ring04", "Ring05", "Ring06", "Ring07",
    "Ring08", "Ring09", "Ring10", "none",
];
/// Seconds it rings; 0 until stopped.
const RING_FOR: [u64; 4] = [30, 60, 120, 0];
const WORK: [u64; 8] = [15, 20, 25, 30, 45, 50, 60, 90];
const SHORT_BREAK: [u64; 4] = [3, 5, 10, 15];
const LONG_BREAK: [u64; 5] = [10, 15, 20, 30, 45];
const ROUNDS: [u64; 5] = [2, 3, 4, 5, 6];

pub fn build(state: &State) -> Vec<Block> {
    let mut blocks = head(state, File::Timers, Page::Timers, "Countdowns, focus sessions and a stopwatch, running in the island where you can see them. Type how long, like 10m tea, and press Enter.");
    enable(&mut blocks, state, File::Timers, "Timers");
    let ok = state.files.doc(File::Timers).error().is_none();
    let f = &state.files;

    let start = Row::new("start", "Timer field").detail("Type a length, a label, focus or stopwatch").icon(icons::timers()).part(button("Open", Role::Secondary)).enabled(ok);
    blocks.push(group("", vec![shortcut::row(state, File::Timers, "Opens the timer field"), start]));

    let sound = f.timer_sound();
    let sound_row = Row::new("sound", "Sound").part(Part::Dropdown { label: sound_name(&sound) }).part(button("Play", Role::Secondary)).enabled(ok);
    let ring = Row::new("ring", "Ring for").detail("Then the card waits quietly until you stop it").part(Part::Dropdown { label: seconds(f.ring_for()) }).enabled(ok);
    blocks.push(group("When a timer ends", vec![sound_row, ring]));

    let dropdown = |id: &str, title: &str, label: String| Row::new(id, title).part(Part::Dropdown { label }).enabled(ok);
    blocks.push(group(
        "Focus sessions",
        vec![
            dropdown("work", "Focus", minutes(f.minutes(File::Timers, "focus_work_min"))),
            dropdown("short", "Short break", minutes(f.minutes(File::Timers, "focus_short_break_min"))),
            dropdown("long", "Long break", minutes(f.minutes(File::Timers, "focus_long_break_min"))),
            dropdown("rounds", "Long break after", rounds(f.focus_rounds())),
            flag(state, File::Timers, "focus_auto_start", "Start the next phase by itself", "Otherwise the island asks when a phase ends"),
            flag(state, File::Timers, "focus_dnd", "Do Not Disturb while focusing", "Turned on for focus and back off for breaks"),
        ],
    ));
    blocks
}

fn sound_name(name: &str) -> String {
    match name {
        "none" => "None".into(),
        n => n.replace("Alarm", "Alarm ").replace("Ring", "Ring ").replace(" 0", " "),
    }
}

fn seconds(n: u64) -> String {
    match n {
        0 => "Until stopped".into(),
        60 => "1 minute".into(),
        n if n % 60 == 0 => format!("{} minutes", n / 60),
        n => format!("{n} seconds"),
    }
}

fn minutes(n: u64) -> String {
    if n == 1 { "1 minute".into() } else { format!("{n} minutes") }
}

fn rounds(n: u64) -> String {
    if n == 1 { "1 round".into() } else { format!("{n} rounds") }
}

/// The choices, with the file's value added in order if it's one of its own.
fn with(list: &[u64], current: u64) -> Vec<u64> {
    let mut list = list.to_vec();
    if !list.contains(&current) {
        let at = list.iter().position(|&v| (v == 0 && current != 0) || v > current).unwrap_or(list.len());
        list.insert(at, current);
    }
    list
}

/// A number dropdown: its file key, the choices, the current value, and how one reads.
type Numbers = (&'static str, Vec<u64>, u64, fn(u64) -> String);

fn numbers(state: &State, row: &str) -> Option<Numbers> {
    let f = &state.files;
    Some(match row {
        "ring" => ("ring_for_s", with(&RING_FOR, f.ring_for()), f.ring_for(), seconds),
        "work" => ("focus_work_min", with(&WORK, f.minutes(File::Timers, "focus_work_min")), f.minutes(File::Timers, "focus_work_min"), minutes),
        "short" => ("focus_short_break_min", with(&SHORT_BREAK, f.minutes(File::Timers, "focus_short_break_min")), f.minutes(File::Timers, "focus_short_break_min"), minutes),
        "long" => ("focus_long_break_min", with(&LONG_BREAK, f.minutes(File::Timers, "focus_long_break_min")), f.minutes(File::Timers, "focus_long_break_min"), minutes),
        "rounds" => ("focus_rounds", with(&ROUNDS, f.focus_rounds()), f.focus_rounds(), rounds),
        _ => return None,
    })
}

pub fn menu(state: &mut State, target: &Target) -> Option<(Vec<String>, Option<usize>)> {
    if target.row == "sound" {
        let current = state.files.timer_sound();
        return Some((SOUNDS.iter().map(|s| sound_name(s)).collect(), SOUNDS.iter().position(|s| *s == current)));
    }
    let (_, list, current, label) = numbers(state, &target.row)?;
    Some((list.iter().map(|&v| label(v)).collect(), list.iter().position(|&v| v == current)))
}

pub fn act(state: &mut State, target: &Target, act: Act) -> Effect {
    match (target.row.as_str(), target.part, act) {
        ("hotkey", _, Act::Press) => return Effect::Record(File::Timers),
        ("start", _, Act::Press) => state.tell_hub("timers", "launcher"),
        ("sound", 0, Act::Choose(i)) => {
            if let Some(name) = SOUNDS.get(i) {
                state.set(File::Timers, "sound", Value::String((*name).into()));
                crate::system::play_sound(name);
            }
        }
        ("sound", 1, Act::Press) => crate::system::play_sound(&state.files.timer_sound()),
        (row, _, Act::Choose(i)) => {
            if let Some((key, list, _, _)) = numbers(state, row) {
                if let Some(&v) = list.get(i) {
                    state.set(File::Timers, key, Value::from(v));
                }
            }
        }
        _ => {}
    }
    Effect::None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_read_naturally_and_keep_odd_values() {
        assert_eq!((sound_name("Alarm01").as_str(), sound_name("Ring10").as_str(), sound_name("none").as_str()), ("Alarm 1", "Ring 10", "None"));
        assert_eq!((seconds(30).as_str(), seconds(60).as_str(), seconds(120).as_str(), seconds(0).as_str()), ("30 seconds", "1 minute", "2 minutes", "Until stopped"));
        assert_eq!(with(&RING_FOR, 90), [30, 60, 90, 120, 0]);
        assert_eq!(with(&WORK, 25), WORK);
        assert_eq!(with(&WORK, 40), [15, 20, 25, 30, 40, 45, 50, 60, 90]);
    }
}
