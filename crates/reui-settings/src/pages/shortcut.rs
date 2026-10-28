//! Shortcut rows (the clipboard history's, the timers'): showing one, and what the recorder
//! does with the chord it heard.

use serde_json::Value;

use crate::keys;
use crate::model::files::File;
use crate::model::hotkey::{self, Hotkey, Problem};
use crate::state::State;
use crate::ui::{Part, Row, Tone};

/// The files with a shortcut, and what each is called in a clash.
const OWNERS: [(File, &str); 2] = [(File::Clipboard, "Clipboard history"), (File::Timers, "Timers")];

/// A "Shortcut" row for `file`: its caps (or the recorder's prompt), and the recorder's
/// last note in place of `detail`.
pub fn row(state: &State, file: File, detail: &str) -> Row {
    let (detail, tone) = match &state.hotkey_note {
        Some((f, text, tone)) if *f == file => (text.clone(), *tone),
        _ => (detail.to_string(), Tone::Secondary),
    };
    let ok = state.files.doc(file).error().is_none();
    let row = Row::new("hotkey", "Shortcut");
    let row = if detail.is_empty() { row } else { row.detail(detail).tone(tone) };
    row.part(Part::Hotkey { caps: hotkey::caps(&state.files.shortcut(file)), recording: state.recording == Some(file) })
        .enabled(ok)
}

/// A chord from the recorder: checked, then saved, or why not (P-3).
pub fn chord(state: &mut State, mods: u32, vk: u32) {
    let Some(file) = state.recording.take() else { return };
    let Some(key_name) = file.shortcut_key() else { return };
    let note = |state: &mut State, text: &str, tone: Tone| state.hotkey_note = Some((file, text.to_string(), tone));
    let key = match Hotkey::from_keys(mods, vk, keys::char_of) {
        Ok(key) => key,
        Err(Problem::NeedsModifier) => return note(state, "Add Ctrl, Alt, Shift or Win to that key", Tone::Negative),
        Err(Problem::Unsupported) => return note(state, "That key can't be a shortcut", Tone::Negative),
    };
    let current = Hotkey::parse(&state.files.shortcut(file), keys::vk_of);
    if current.as_ref() == Some(&key) {
        state.hotkey_note = None;
        return;
    }
    // Another of ReUI's own shortcuts.
    if let Some((_, name)) = OWNERS.iter().find(|(other, _)| *other != file && Hotkey::parse(&state.files.shortcut(*other), keys::vk_of).as_ref() == Some(&key)) {
        return note(state, &format!("{} already opens {name}", key.label()), Tone::Negative);
    }
    let held = state.held_hotkeys.iter().any(|(f, h)| *f == file && Hotkey::parse(h, keys::vk_of).is_some_and(|h| h == key));
    if !held && !keys::hotkey_free(&key) {
        let free = std::iter::successors(key.alternative(), Hotkey::alternative).find(keys::hotkey_free);
        let text = match free {
            Some(alt) => format!("{} is taken by another app; {} is free", key.label(), alt.label()),
            None => format!("{} is taken by another app", key.label()),
        };
        return note(state, &text, Tone::Negative);
    }
    state.set(file, key_name, Value::String(key.label()));
    // The hub registers the clipboard's shortcut when it starts; the timers' on reload.
    state.hotkey_note = (file == File::Clipboard && state.hub_running()).then(|| (file, "Takes effect when ReUI restarts".to_string(), Tone::Warning));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::tests::state;

    #[test]
    fn a_plain_letter_is_refused() {
        let (mut state, dir) = state("shortcut-letter");
        state.recording = Some(File::Clipboard);
        chord(&mut state, 0, u32::from(b'K'));
        assert_eq!(state.hotkey_note.as_ref().map(|n| (n.0, n.2)), Some((File::Clipboard, Tone::Negative)));
        assert_eq!(state.files.shortcut(File::Clipboard), "Win+Alt+V");
        assert_eq!(state.recording, None);
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn one_shortcut_never_opens_two_things() {
        let (mut state, dir) = state("shortcut-clash");
        state.recording = Some(File::Timers);
        // Win+Alt+V is the clipboard's.
        chord(&mut state, 0x0008 | 0x0001, u32::from(b'V'));
        let note = state.hotkey_note.clone().unwrap();
        assert_eq!((note.0, note.2), (File::Timers, Tone::Negative));
        assert!(note.1.contains("Clipboard history"));
        assert_eq!(state.files.shortcut(File::Timers), "Win+Ctrl+T");
        _ = std::fs::remove_dir_all(&dir);
    }
}
