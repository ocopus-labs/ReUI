//! General: start with Windows, when the island shows, and pausing ReUI.

use serde_json::Value;

use super::{button, group, head, Effect};
use crate::model::files::{File, VISIBILITY};
use crate::state::State;
use crate::system::{self, Startup};
use crate::ui::{Act, Block, Part, Role, Row, Target, Tone};

const PAUSE_MS: u64 = 3_600_000;

pub fn build(state: &State) -> Vec<Block> {
    let mut blocks = head(state, File::Island, "General", "");

    let hub = state.paths.hub_exe();
    let startup = match &state.startup {
        _ if !hub.exists() => Row::new("startup", "Start with Windows").detail("reui-hub.exe isn't next to Settings").tone(Tone::Warning).part(Part::Toggle { on: false }).enabled(false),
        Startup::Off => Row::new("startup", "Start with Windows").detail("Opens ReUI when you sign in").part(Part::Toggle { on: false }),
        Startup::On => Row::new("startup", "Start with Windows").detail("Opens ReUI when you sign in").part(Part::Toggle { on: true }),
        Startup::Elsewhere(path) => Row::new("startup", "Start with Windows").detail(format!("Starts {path}")).tone(Tone::Warning).part(Part::Toggle { on: true }),
        Startup::DisabledInTaskManager => Row::new("startup", "Start with Windows")
            .detail("Turned off in Task Manager")
            .tone(Tone::Warning)
            .part(super::button("Open Task Manager", Role::Secondary)),
    };
    blocks.push(group("Startup", vec![startup]));

    let doc_ok = state.files.doc(File::Island).error().is_none();
    let selected = state.files.visibility();
    let detail = ["The resting pill stays at the top of the screen", "Shows while something is happening", "Shows for questions, alerts and changes you make"][selected];
    let visibility = Row::new("visibility", "Show the island")
        .detail(detail)
        .part(Part::Segmented { options: vec!["Always".into(), "When active".into(), "When needed".into()], selected })
        .enabled(doc_ok);
    blocks.push(group("Island", vec![visibility]));

    let left = state.files.paused_until().and_then(|until| until.checked_sub(reui_protocol::unix_ms())).filter(|&ms| ms > 0);
    let pause = match left {
        Some(ms) => Row::new("pause", "Paused").detail(format!("Resumes in {}", system::time_left(ms))).tone(Tone::Warning).part(button("Resume", Role::Primary)),
        None => Row::new("pause", "Pause ReUI").detail("Quiets the island and stops recording the clipboard for an hour; permission requests still show").part(button("Pause for 1 hour", Role::Secondary)),
    };
    blocks.push(group("", vec![pause.enabled(doc_ok)]));
    blocks
}

pub fn act(state: &mut State, target: &Target, act: Act) -> Effect {
    match (target.row.as_str(), act) {
        ("startup", Act::Press) => {
            match state.startup {
                Startup::DisabledInTaskManager => system::open_task_manager(),
                Startup::On | Startup::Elsewhere(_) => report(state, system::set_startup(&state.paths, false)),
                Startup::Off => report(state, system::set_startup(&state.paths, true)),
            }
            state.startup = system::startup(&state.paths);
        }
        ("visibility", Act::Choose(i)) if i < VISIBILITY.len() => state.set(File::Island, "visibility", Value::String(VISIBILITY[i].into())),
        // The island sees the file change, hides, and pauses clipboard recording itself.
        ("pause", Act::Press) => {
            let paused = state.files.paused_until().is_some_and(|until| until > reui_protocol::unix_ms());
            let until = if paused { Value::Null } else { Value::from(reui_protocol::unix_ms() + PAUSE_MS) };
            state.set(File::Island, "paused_until", until);
        }
        _ => {}
    }
    Effect::None
}

fn report(state: &mut State, result: Result<(), String>) {
    state.write_error = result.err();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::tests::state;

    #[test]
    fn visibility_and_pause_write_the_islands_file() {
        let (mut state, dir) = state("general");
        act(&mut state, &Target::new("visibility", 0), Act::Choose(1));
        assert_eq!(state.files.visibility(), 1);
        act(&mut state, &Target::new("pause", 0), Act::Press);
        let until = state.files.paused_until().unwrap();
        assert!(until > reui_protocol::unix_ms() + PAUSE_MS - 60_000);
        act(&mut state, &Target::new("pause", 0), Act::Press);
        assert_eq!(state.files.paused_until(), None);
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("roaming").join("settings.json")).unwrap()).unwrap();
        assert_eq!(saved["visibility"], "when-active");
        _ = std::fs::remove_dir_all(&dir);
    }
}
