//! Now Playing (`media.json`).

use serde_json::Value;

use super::{flag, group, head, Effect};
use crate::model::files::{File, VISUALIZER};
use crate::state::State;
use crate::ui::{Act, Block, Part, Row, Target};

/// "Leave the island after" choices, in minutes; 0 is never.
const RETRACT: [u64; 7] = [1, 5, 10, 15, 30, 60, 0];

pub fn build(state: &State) -> Vec<Block> {
    let mut blocks = head(state, File::Media, "Now Playing", "Whatever plays on this PC, in the island: artwork, controls, a timeline you can scrub, and bars that move with the music.");
    let ok = state.files.doc(File::Media).error().is_none();
    let selected = state.files.visualizer();
    let detail = ["Bars follow the player's own audio", "Bars move without listening to the audio", "Bars stay still"][selected];
    let visualizer = Row::new("visualizer", "Visualizer").detail(detail).part(Part::Segmented { options: vec!["Real".into(), "Simple".into(), "Off".into()], selected }).enabled(ok);
    let retract = Row::new("retract", "Leave the island after a pause").detail("A paused player steps away after this long").part(Part::Dropdown { label: minutes(state.files.retract_min()) }).enabled(ok);
    blocks.push(group(
        "Card",
        vec![
            visualizer,
            flag(state, File::Media, "swipe_to_skip", "Swipe to skip", "Swipe the pill sideways for the next or previous track"),
            flag(state, File::Media, "show_video", "Videos", "Films and videos too, not only music"),
            retract,
        ],
    ));
    blocks.push(group("Lyrics", vec![flag(state, File::Media, "lyrics", "Synced lyrics", "Song titles are sent to lrclib.net to find them")]));
    blocks
}

fn minutes(n: u64) -> String {
    match n {
        0 => "Never".into(),
        60 => "1 hour".into(),
        n if n % 60 == 0 => format!("{} hours", n / 60),
        1 => "1 minute".into(),
        n => format!("{n} minutes"),
    }
}

/// The choices, with the file's value added if it's one of its own.
fn choices(current: u64) -> Vec<u64> {
    let mut list = RETRACT.to_vec();
    if !list.contains(&current) {
        let at = list.iter().position(|&m| m == 0 || m > current).unwrap_or(list.len());
        list.insert(at, current);
    }
    list
}

pub fn menu(state: &State, target: &Target) -> Option<(Vec<String>, Option<usize>)> {
    (target.row == "retract").then(|| {
        let current = state.files.retract_min();
        let list = choices(current);
        (list.iter().map(|&m| minutes(m)).collect(), list.iter().position(|&m| m == current))
    })
}

pub fn act(state: &mut State, target: &Target, act: Act) -> Effect {
    match (target.row.as_str(), act) {
        ("visualizer", Act::Choose(i)) if i < VISUALIZER.len() => state.set(File::Media, "visualizer", Value::String(VISUALIZER[i].into())),
        ("retract", Act::Choose(i)) => {
            if let Some(&m) = choices(state.files.retract_min()).get(i) {
                state.set(File::Media, "retract_after_pause_min", Value::from(m));
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
    fn minutes_read_naturally_and_odd_values_are_kept() {
        assert_eq!((minutes(0).as_str(), minutes(1).as_str(), minutes(10).as_str(), minutes(60).as_str(), minutes(120).as_str()), ("Never", "1 minute", "10 minutes", "1 hour", "2 hours"));
        assert_eq!(choices(10), RETRACT);
        assert_eq!(choices(7), [1, 5, 7, 10, 15, 30, 60, 0]);
        assert_eq!(choices(90), [1, 5, 10, 15, 30, 60, 90, 0]);
    }
}
