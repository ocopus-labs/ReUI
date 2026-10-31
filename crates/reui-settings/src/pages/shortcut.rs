//! Shortcut rows (the clipboard history's, the timers', the stats card's, hiding and
//! opening the island): showing one, and what the recorder does with the chord it heard.

use serde_json::Value;

use crate::keys;
use crate::model::files::Shortcut;
use crate::model::hotkey::{self, Hotkey, Problem};
use crate::state::State;
use crate::ui::{Part, Row, Tone};

/// What each shortcut does, for a clash ("Win+Alt+V already opens Clipboard history").
const OWNERS: [(Shortcut, &str); 5] = [
    (Shortcut::Clipboard, "opens Clipboard history"),
    (Shortcut::Timers, "opens Timers"),
    (Shortcut::Stats, "opens System stats"),
    (Shortcut::HideIsland, "hides the island"),
    (Shortcut::OpenIsland, "opens the island"),
];

/// The row's id: pages with two shortcuts (General) tell them apart by it.
pub fn row_id(shortcut: Shortcut) -> &'static str {
    match shortcut {
        Shortcut::OpenIsland => "open_hotkey",
        _ => "hotkey",
    }
}

/// A "Shortcut" row: its caps (or the recorder's prompt), and the recorder's last note in
/// place of `detail`.
pub fn row(state: &State, shortcut: Shortcut, detail: &str) -> Row {
    named(state, shortcut, "Shortcut", detail)
}

/// The same, titled for a page where "Shortcut" alone wouldn't say what it's for.
pub fn named(state: &State, shortcut: Shortcut, title: &str, detail: &str) -> Row {
    let (detail, tone) = match &state.hotkey_note {
        Some((s, text, tone)) if *s == shortcut => (text.clone(), *tone),
        _ => (detail.to_string(), Tone::Secondary),
    };
    let ok = state.files.doc(shortcut.file()).error().is_none();
    let row = Row::new(row_id(shortcut), title);
    let row = if detail.is_empty() { row } else { row.detail(detail).tone(tone) };
    row.part(Part::Hotkey { caps: hotkey::caps(&state.files.shortcut(shortcut)), recording: state.recording == Some(shortcut) })
        .enabled(ok)
}

/// A chord from the recorder: checked, then saved, or why not (P-3).
pub fn chord(state: &mut State, mods: u32, vk: u32) {
    let Some(shortcut) = state.recording.take() else { return };
    let note = |state: &mut State, text: &str, tone: Tone| state.hotkey_note = Some((shortcut, text.to_string(), tone));
    let key = match Hotkey::from_keys(mods, vk, keys::char_of) {
        Ok(key) => key,
        Err(Problem::NeedsModifier) => return note(state, "Add Ctrl, Alt, Shift or Win to that key", Tone::Negative),
        Err(Problem::Unsupported) => return note(state, "That key can't be a shortcut", Tone::Negative),
    };
    let current = Hotkey::parse(&state.files.shortcut(shortcut), keys::vk_of);
    if current.as_ref() == Some(&key) {
        state.hotkey_note = None;
        return;
    }
    // Another of ReUI's own shortcuts.
    if let Some((_, name)) = OWNERS.iter().find(|(other, _)| *other != shortcut && Hotkey::parse(&state.files.shortcut(*other), keys::vk_of).as_ref() == Some(&key)) {
        return note(state, &format!("{} already {name}", key.label()), Tone::Negative);
    }
    let held = state.held_hotkeys.iter().any(|(s, h)| *s == shortcut && Hotkey::parse(h, keys::vk_of).is_some_and(|h| h == key));
    if !held && !keys::hotkey_free(&key) {
        let free = std::iter::successors(key.alternative(), Hotkey::alternative).find(keys::hotkey_free);
        let text = match free {
            Some(alt) => format!("{} is taken by another app; {} is free", key.label(), alt.label()),
            None => format!("{} is taken by another app", key.label()),
        };
        return note(state, &text, Tone::Negative);
    }
    state.set(shortcut.file(), shortcut.key(), Value::String(key.label()));
    // The hub registers the clipboard's shortcut when it starts; the others on reload.
    state.hotkey_note = (shortcut == Shortcut::Clipboard && state.hub_running()).then(|| (shortcut, "Takes effect when ReUI restarts".to_string(), Tone::Warning));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::tests::state;

    #[test]
    fn a_plain_letter_is_refused() {
        let (mut state, dir) = state("shortcut-letter");
        state.recording = Some(Shortcut::Clipboard);
        chord(&mut state, 0, u32::from(b'K'));
        assert_eq!(state.hotkey_note.as_ref().map(|n| (n.0, n.2)), Some((Shortcut::Clipboard, Tone::Negative)));
        assert_eq!(state.files.shortcut(Shortcut::Clipboard), "Win+Alt+V");
        assert_eq!(state.recording, None);
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn one_shortcut_never_opens_two_things() {
        let (mut state, dir) = state("shortcut-clash");
        state.recording = Some(Shortcut::Timers);
        // Win+Alt+V is the clipboard's.
        chord(&mut state, 0x0008 | 0x0001, u32::from(b'V'));
        let note = state.hotkey_note.clone().unwrap();
        assert_eq!((note.0, note.2), (Shortcut::Timers, Tone::Negative));
        assert!(note.1.contains("Clipboard history"));
        assert_eq!(state.files.shortcut(Shortcut::Timers), "Win+Ctrl+T");
        // The island's hide shortcut takes part too.
        state.recording = Some(Shortcut::Stats);
        chord(&mut state, 0x0008 | 0x0001, u32::from(b'I'));
        assert!(state.hotkey_note.clone().unwrap().1.contains("hides the island"));
        assert_eq!(state.files.shortcut(Shortcut::HideIsland), "Win+Alt+I");
        // So does the one that opens it, which shares the island's file.
        state.recording = Some(Shortcut::HideIsland);
        chord(&mut state, 0x0008 | 0x0001, u32::from(b'N'));
        assert!(state.hotkey_note.clone().unwrap().1.contains("opens the island"));
        assert_eq!(state.files.shortcut(Shortcut::OpenIsland), "Win+Alt+N");
        _ = std::fs::remove_dir_all(&dir);
    }
}
