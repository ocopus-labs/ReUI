//! General: start with Windows, where and when the island shows, and pausing ReUI.

use serde_json::Value;

use super::{button, flag, footed, group, head, Effect, Page};
use crate::ui::icons;
use reui_design::tokens::color::system as palette;
use crate::model::files::{File, HOVER_DELAYS, MONITOR_CURSOR, MONITOR_PRIMARY, OPEN_ON, REVEAL_ZONE, VISIBILITY};
use crate::state::State;
use crate::system::{self, Startup};
use crate::ui::{Act, Block, Part, Role, Row, Target, Tone};

const PAUSE_MS: u64 = 3_600_000;

pub fn build(state: &State) -> Vec<Block> {
    let mut blocks = head(state, File::Island, Page::General, "The island at the top of your screen: where it sits, when it shows, and whether it starts with Windows.");

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
    blocks.push(group("Startup", vec![startup.icon(icons::startup())]));

    let doc_ok = state.files.doc(File::Island).error().is_none();
    let selected = state.files.visibility();
    let detail = ["The resting pill stays at the top of the screen", "Shows while something is happening", "Shows for questions, alerts and changes you make"][selected];
    let visibility = Row::new("visibility", "Show the island")
        .detail(detail)
        .part(Part::Segmented { options: vec!["Always".into(), "When active".into(), "When needed".into()], selected })
        .enabled(doc_ok)
        .icon(icons::tile(icons::VIEW, palette::BLUE));
    // How much of the top edge brings a hidden island back (docs/PRD-behaviour.md §4.2); only
    // means something when the island would otherwise hide.
    let zone = state.files.reveal_zone();
    let reveal = Row::new("reveal_zone", "Reveal from the top edge")
        .detail(["The top edge never shows it", "Pushing the pointer to the edge above the pill shows it", "Pushing the pointer to the edge near the island shows it"][zone])
        .part(Part::Segmented { options: vec!["Off".into(), "Pill".into(), "Wide".into()], selected: zone })
        .enabled(doc_ok && selected != 0)
        .icon(icons::tile(icons::UP, palette::GREEN));
    // How a rest on the pill is read (docs/PRD-behaviour.md §4.1).
    let open_on = state.files.open_on();
    let opening = Row::new("open_on", "Open the island")
        .detail(["A click opens a card; resting the pointer peeks", "Resting the pointer opens a card"][open_on])
        .part(Part::Segmented { options: vec!["On click".into(), "On hover".into()], selected: open_on })
        .enabled(doc_ok)
        .icon(icons::tile(icons::POINTER, palette::BLUE));
    let delay = Row::new("hover_delay", "Hover delay")
        .detail("How long the pointer rests before the island responds")
        .part(Part::Segmented { options: vec!["Quick".into(), "Normal".into(), "Relaxed".into()], selected: state.files.hover_delay() })
        .enabled(doc_ok)
        .icon(icons::tile(icons::STOPWATCH, palette::BLUE));
    let monitor = Row::new("monitor", "Display").detail("Where the island sits").part(Part::Dropdown { label: monitor_label(&state.files.monitor()) }).enabled(doc_ok).icon(icons::tile(icons::MONITOR, palette::TEAL));
    // Only means something when the island would otherwise hide.
    let desktop = flag(state, File::Island, "show_on_desktop", "Always show on the desktop", "In apps it shows only as chosen above")
        .enabled(doc_ok && selected != 0)
        .icon(icons::tile(icons::WALLPAPER, palette::GREEN));
    let fullscreen = flag(state, File::Island, "hide_in_fullscreen", "Hide in full screen", "Games, videos and presentations keep the whole screen").icon(icons::tile(icons::FULLSCREEN, palette::PURPLE));
    // Tabs under an open place's card (docs/PRD-view-switcher.md).
    let switcher = flag(state, File::Island, "view_switcher", "View switcher", "Tabs under an open card: Clipboard, Timers, Devices, System stats")
        .icon(icons::tile(icons::TABS, palette::ORANGE));
    // The time in the resting pill, for people who hide the taskbar (a peek shows it anyway).
    let clock = flag(state, File::Island, "idle_clock", "Clock in the resting island", "The time while nothing else is showing")
        .icon(icons::tile(icons::CLOCK, palette::INDIGO));
    // Hides it until pressed again; the hub registers it (`modules/hide.rs`).
    let hide = super::shortcut::named(state, File::Island, "Hide the island", "Until you press it again; permission requests still show")
        .icon(icons::tile(icons::KEYBOARD, palette::GRAY));
    blocks.push(group("Island", vec![visibility, desktop, reveal, opening, delay, monitor, fullscreen, switcher, clock, hide]));
    blocks.extend(super::quiet::blocks(state, doc_ok));

    let left = state.files.paused_until().and_then(|until| until.checked_sub(reui_protocol::unix_ms())).filter(|&ms| ms > 0);
    let pause = match left {
        Some(ms) => Row::new("pause", "Paused").detail(format!("Resumes in {}", system::time_left(ms))).tone(Tone::Warning).part(button("Resume", Role::Primary)),
        None => Row::new("pause", "Pause ReUI").detail("Quiets the island for an hour").part(button("Pause for 1 hour", Role::Secondary)),
    };
    blocks.push(footed("", vec![pause.enabled(doc_ok).icon(icons::tile(icons::MOON, palette::INDIGO))], "The clipboard isn't recorded meanwhile; permission requests still show."));
    blocks
}

/// How the `monitor` value reads.
fn monitor_label(value: &str) -> String {
    match value {
        MONITOR_PRIMARY => "Main display".into(),
        MONITOR_CURSOR => "Where the pointer is".into(),
        device => system::monitor_label(device),
    }
}

/// The choices: main display, where the pointer is, then each display (and the saved one,
/// if it isn't connected now).
fn monitor_choices(state: &State) -> Vec<String> {
    let mut list = vec![MONITOR_PRIMARY.to_string(), MONITOR_CURSOR.to_string()];
    list.extend(state.monitors.iter().map(|(device, _)| device.clone()));
    let current = state.files.monitor();
    if !list.contains(&current) {
        list.push(current);
    }
    list
}

pub fn menu(state: &mut State, target: &Target) -> Option<(Vec<String>, Option<usize>)> {
    if target.row != "monitor" {
        return super::quiet::menu(state, target);
    }
    state.monitors = system::monitors();
    let list = monitor_choices(state);
    let current = state.files.monitor();
    Some((list.iter().map(|m| monitor_label(m)).collect(), list.iter().position(|m| *m == current)))
}

pub fn act(state: &mut State, target: &Target, act: Act) -> Effect {
    if super::quiet::act(state, target, &act) {
        return Effect::None;
    }
    match (target.row.as_str(), act) {
        ("hotkey", Act::Press) => return Effect::Record(File::Island),
        ("monitor", Act::Choose(i)) => {
            if let Some(choice) = monitor_choices(state).get(i).cloned() {
                state.set(File::Island, "monitor", Value::String(choice));
            }
        }
        ("startup", Act::Press) => {
            match state.startup {
                Startup::DisabledInTaskManager => system::open_task_manager(),
                Startup::On | Startup::Elsewhere(_) => report(state, system::set_startup(&state.paths, false)),
                Startup::Off => report(state, system::set_startup(&state.paths, true)),
            }
            state.startup = system::startup(&state.paths);
        }
        ("visibility", Act::Choose(i)) if i < VISIBILITY.len() => state.set(File::Island, "visibility", Value::String(VISIBILITY[i].into())),
        ("open_on", Act::Choose(i)) if i < OPEN_ON.len() => state.set(File::Island, "open_on", Value::String(OPEN_ON[i].into())),
        ("reveal_zone", Act::Choose(i)) if i < REVEAL_ZONE.len() => state.set(File::Island, "reveal_zone", Value::String(REVEAL_ZONE[i].into())),
        ("hover_delay", Act::Choose(i)) if i < HOVER_DELAYS.len() => state.set(File::Island, "hover_delay", Value::from(HOVER_DELAYS[i])),
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
        assert_eq!(saved["hide_in_fullscreen"], true, "a new file gets every default");
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_monitor_and_fullscreen_keys_are_written() {
        let (mut state, dir) = state("general-monitor");
        state.monitors = vec![(r"\\.\DISPLAY1".into(), "Display 1".into()), (r"\\.\DISPLAY2".into(), "Display 2".into())];
        act(&mut state, &Target::new("monitor", 0), Act::Choose(1));
        assert_eq!(state.files.monitor(), "cursor");
        act(&mut state, &Target::new("monitor", 0), Act::Choose(3));
        assert_eq!(state.files.monitor(), r"\\.\DISPLAY2");
        assert_eq!(monitor_label(&state.files.monitor()), "Display 2");
        // Unplugged: still listed, so the menu shows what's saved.
        state.monitors.clear();
        assert_eq!(monitor_choices(&state), ["primary", "cursor", r"\\.\DISPLAY2"]);
        act(&mut state, &Target::new("open_on", 0), Act::Choose(1));
        act(&mut state, &Target::new("hover_delay", 0), Act::Choose(0));
        assert_eq!((state.files.open_on(), state.files.hover_delay()), (1, 0));
        act(&mut state, &Target::new("reveal_zone", 0), Act::Choose(2));
        assert_eq!((state.files.reveal_zone(), state.files.doc(File::Island).str("reveal_zone")), (2, Some("wide")));
        assert_eq!(state.files.doc(File::Island).get("hover_delay").and_then(Value::as_u64), Some(150));
        crate::pages::act(crate::pages::Page::General, &mut state, &Target::new("island:hide_in_fullscreen", 0), Act::Press);
        assert!(!state.files.doc(File::Island).bool("hide_in_fullscreen"));
        _ = std::fs::remove_dir_all(&dir);
    }
}
