//! The welcome (`--welcome`): shown once, on first run, and again from About's "Replay the
//! tour". Three steps in the same window: what ReUI shows (a card per feature), the
//! clipboard shortcut, start with Windows. Finishing it any way (Done, Open Settings, the
//! window's ✕) writes `welcomed: true` into `onboarding.json`, keeping its other keys.

use serde_json::{json, Value};

use super::{button, flag, group, Effect, Page};
use crate::model::doc::Doc;
use crate::model::files::File;
use crate::model::hotkey;
use crate::state::State;
use crate::system::{self, Startup};
use crate::ui::{icons, Act, Block, Part, Role, Row, Target, Tone};
use reui_design::tokens::color::system as palette;

/// Shared with the hub's onboarding module (first-run demo, tips, offers).
pub const ONBOARDING: &str = "onboarding.json";

/// What each step says: (hero, one quiet line).
fn words(page: Page, state: &State) -> (&'static str, &'static str) {
    match page {
        Page::WelcomeFeatures if state.restart_note => ("What ReUI shows", "Features you turn on or off start or stop when ReUI restarts."),
        Page::WelcomeFeatures => ("What ReUI shows", "Each is a card in the island when it matters."),
        Page::WelcomeShortcut => ("Your clipboard shortcut", "Opens your clipboard history from any app."),
        _ => ("Start with Windows", "ReUI opens when you sign in."),
    }
}

pub fn build(page: Page, state: &State) -> Vec<Block> {
    let (title, body) = words(page, state);
    let mut blocks = vec![Block::Intro { eyebrow: Some(format!("Step {} of 3", page.index() + 1)), title: title.into(), body: Some(body.into()), icon: Some(page.icon()), preview: page.preview(), row: None }];
    let next = || group("", vec![Row::new("next", "").part(button("Next", Role::Primary)).bare()]);
    match page {
        Page::WelcomeFeatures => {
            blocks.push(group(
                "",
                vec![
                    flag(state, File::Media, "enabled", "Now Playing", "Whatever plays, with its controls").icon(icons::now_playing()),
                    flag(state, File::Devices, "enabled", "Devices", "Bluetooth, batteries, charging and drives").icon(icons::devices()),
                    flag(state, File::Clipboard, "enabled", "Clipboard history", "What you copy, kept on this PC").icon(icons::clipboard()),
                    flag(state, File::Osd, "volume", "Volume pop-up", "The level when the volume changes").icon(icons::osd()),
                ],
            ));
            blocks.push(group(
                "",
                vec![
                    flag(state, File::Osd, "replace_windows_flyout", "Replace Windows' volume pop-up", "Hides Windows' pop-up while ReUI runs; restored when you quit").icon(icons::gray(icons::MONITOR)),
                    flag(state, File::Media, "lyrics", "Lyrics", "Song titles go to lrclib.net").icon(icons::tile(icons::LYRICS, palette::PINK)),
                    flag(state, File::Devices, "airpods", "AirPods detail", "Case and bud batteries from a short Bluetooth scan nearby").icon(icons::gray(icons::HEADPHONES)),
                ],
            ));
            blocks.push(next());
        }
        Page::WelcomeShortcut => {
            let ok = state.files.doc(File::Clipboard).error().is_none();
            let row = Row::new("hotkey", "Shortcut");
            let row = match state.hotkey_note.clone() {
                Some((note, tone)) => row.detail(note).tone(tone),
                None => row,
            };
            let row = row.part(Part::Hotkey { caps: hotkey::caps(&state.files.hotkey()), recording: state.recording }).enabled(ok);
            blocks.push(group("", vec![row]));
            blocks.push(next());
        }
        _ => {
            let row = Row::new("startup", "Open at sign-in");
            let row = match &state.startup {
                _ if !state.paths.hub_exe().exists() => row.detail("reui-hub.exe isn't next to Settings").tone(Tone::Warning).part(Part::Toggle { on: false }).enabled(false),
                Startup::DisabledInTaskManager => row.detail("Turned off in Task Manager").tone(Tone::Warning).part(button("Open Task Manager", Role::Secondary)),
                Startup::Elsewhere(path) => row.detail(format!("Starts {path}")).tone(Tone::Warning).part(Part::Toggle { on: true }),
                Startup::On => row.part(Part::Toggle { on: true }),
                Startup::Off => row.part(Part::Toggle { on: false }),
            };
            blocks.push(group("", vec![row]));
            blocks.push(group("", vec![Row::new("finish", "").part(button("Open Settings", Role::Secondary)).part(button("Done", Role::Primary)).bare()]));
        }
    }
    blocks
}

pub fn act(page: Page, state: &mut State, target: &Target, act: Act) -> Effect {
    match (target.row.as_str(), target.part, act) {
        ("next", _, Act::Press) => {
            let steps = Page::WELCOME;
            return Effect::Navigate(steps[(page.index() + 1).min(steps.len() - 1)]);
        }
        ("hotkey", _, Act::Press) => return Effect::Record,
        ("startup", _, Act::Press) => {
            let result = match state.startup {
                Startup::DisabledInTaskManager => {
                    system::open_task_manager();
                    Ok(())
                }
                Startup::On | Startup::Elsewhere(_) => system::set_startup(&state.paths, false),
                Startup::Off => system::set_startup(&state.paths, true),
            };
            state.write_error = result.err();
            state.startup = system::startup(&state.paths);
        }
        ("finish", part, Act::Press) => return Effect::Finish { open_settings: part == 0 },
        _ => {}
    }
    Effect::None
}

/// The welcome opens: features that are off for new users are written off, unless the
/// file already says otherwise (AirPods detail scans nearby, so it waits to be asked for).
pub fn start(state: &mut State) {
    let doc = state.files.doc(File::Devices);
    if doc.error().is_none() && !doc.has("airpods") {
        state.set(File::Devices, "airpods", Value::Bool(false));
    }
}

/// Done, Open Settings or ✕: never shown again on its own (P-1).
pub fn finish(state: &State) {
    let mut doc = Doc::open(state.paths.file(ONBOARDING), json!({}));
    if let Err(e) = doc.set("welcomed", Value::Bool(true)) {
        eprintln!("settings: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::tests::state;

    #[test]
    fn steps_have_an_eyebrow_a_hero_and_one_line() {
        let (state, dir) = state("welcome-steps");
        for (i, page) in Page::WELCOME.into_iter().enumerate() {
            let blocks = build(page, &state);
            let Block::Intro { eyebrow: Some(eyebrow), body: Some(_), .. } = &blocks[0] else { panic!("{page:?} starts with its intro") };
            assert_eq!(eyebrow, &format!("Step {} of 3", i + 1));
        }
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn feature_cards_write_their_keys_and_new_users_start_without_airpods_scans() {
        let (mut state, dir) = state("welcome-features");
        start(&mut state);
        assert!(!state.files.doc(File::Devices).bool("airpods"));
        let page = Page::WelcomeFeatures;
        for (row, file, key, before) in [("media:enabled", File::Media, "enabled", true), ("osd:replace_windows_flyout", File::Osd, "replace_windows_flyout", false), ("media:lyrics", File::Media, "lyrics", false)] {
            assert_eq!(state.files.doc(file).bool(key), before);
            crate::pages::act(page, &mut state, &Target::new(row, 0), Act::Press);
            assert_eq!(state.files.doc(file).bool(key), !before, "{row}");
        }
        // Someone who already chose keeps their choice.
        crate::pages::act(page, &mut state, &Target::new("devices:airpods", 0), Act::Press);
        start(&mut state);
        assert!(state.files.doc(File::Devices).bool("airpods"));
        assert_eq!(act(page, &mut state, &Target::new("next", 0), Act::Press), Effect::Navigate(Page::WelcomeShortcut));
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn finishing_marks_it_welcomed_and_keeps_other_keys() {
        let (mut state, dir) = state("welcome-finish");
        assert_eq!(act(Page::WelcomeStartup, &mut state, &Target::new("finish", 0), Act::Press), Effect::Finish { open_settings: true });
        assert_eq!(act(Page::WelcomeStartup, &mut state, &Target::new("finish", 1), Act::Press), Effect::Finish { open_settings: false });
        finish(&state);
        let path = dir.join("roaming").join(ONBOARDING);
        assert_eq!(serde_json::from_str::<Value>(&std::fs::read_to_string(&path).unwrap()).unwrap(), json!({ "welcomed": true }));
        std::fs::write(&path, r#"{ "demo_shown": true, "tips": { "copy": 3 }, "welcomed": false }"#).unwrap();
        finish(&state);
        assert_eq!(serde_json::from_str::<Value>(&std::fs::read_to_string(&path).unwrap()).unwrap(), json!({ "demo_shown": true, "tips": { "copy": 3 }, "welcomed": true }));
        _ = std::fs::remove_dir_all(&dir);
    }
}
