//! The keyboard map of a card that has the keyboard (the clipboard panel, the timer field,
//! a moment's choices and answer field): which keys it reads and what they mean, and the
//! text editing they do. Pure; `clipboard_keys` is the hook that feeds it.

use reui_protocol::ItemKey;

/// Keys a captured panel handles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Type(char),
    Backspace,
    BackspaceWord,
    Escape,
    Move(i32),
    First,
    Last,
    Activate,
    Item(ItemKey),
    /// Alt+1…9: the nth card in view.
    Nth(usize),
    /// Tab / Shift+Tab.
    Tab(i32),
    Undo,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

pub mod vk {
    pub const BACK: u16 = 0x08;
    pub const TAB: u16 = 0x09;
    pub const RETURN: u16 = 0x0D;
    pub const ESCAPE: u16 = 0x1B;
    pub const END: u16 = 0x23;
    pub const HOME: u16 = 0x24;
    pub const LEFT: u16 = 0x25;
    pub const UP: u16 = 0x26;
    pub const RIGHT: u16 = 0x27;
    pub const DOWN: u16 = 0x28;
    pub const DELETE: u16 = 0x2E;
    pub const KEY_1: u16 = 0x31;
    pub const KEY_9: u16 = 0x39;
    pub const KEY_P: u16 = 0x50;
    pub const KEY_Z: u16 = 0x5A;
    pub const NUMPAD1: u16 = 0x61;
    pub const NUMPAD9: u16 = 0x69;
}

/// The keyboard map. `text` gives the character the key types with the current layout and
/// modifiers (asked only when needed). `None` means "not ours": the key goes on to the app
/// in front. Ctrl+Alt together is AltGr, which types characters on many layouts.
pub fn map_key(key: u16, mods: Mods, text: impl FnOnce() -> Option<char>) -> Option<Key> {
    let Mods { shift, ctrl, alt } = mods;
    let bare = !ctrl && !alt;
    let ctrl_only = ctrl && !alt && !shift;
    Some(match key {
        vk::ESCAPE if bare => Key::Escape,
        vk::BACK if !alt => {
            if ctrl {
                Key::BackspaceWord
            } else {
                Key::Backspace
            }
        }
        vk::LEFT | vk::UP if bare => Key::Move(-1),
        vk::RIGHT | vk::DOWN if bare => Key::Move(1),
        vk::HOME if bare => Key::First,
        vk::END if bare => Key::Last,
        vk::RETURN if !alt => match (shift, ctrl) {
            (false, false) => Key::Activate,
            (true, false) => Key::Item(ItemKey::ShiftEnter),
            (false, true) => Key::Item(ItemKey::CtrlEnter),
            (true, true) => return None,
        },
        vk::DELETE if bare && !shift => Key::Item(ItemKey::Delete),
        vk::TAB if bare => Key::Tab(if shift { -1 } else { 1 }),
        vk::KEY_P if ctrl_only => Key::Item(ItemKey::CtrlP),
        vk::KEY_Z if ctrl_only => Key::Undo,
        vk::KEY_1..=vk::KEY_9 if alt && !ctrl => Key::Nth((key - vk::KEY_1) as usize),
        vk::NUMPAD1..=vk::NUMPAD9 if alt && !ctrl => Key::Nth((key - vk::NUMPAD1) as usize),
        _ if ctrl != alt => return None,
        _ => Key::Type(text().filter(|c| !c.is_control())?),
    })
}

/// Ctrl+Backspace: drops trailing spaces, then the word before them.
pub fn delete_word(text: &mut String) {
    let trimmed = text.trim_end_matches(char::is_whitespace).len();
    text.truncate(trimmed);
    let word = text.trim_end_matches(|c: char| !c.is_whitespace()).len();
    text.truncate(word);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_map() {
        let none = Mods::default();
        let shift = Mods { shift: true, ..none };
        let ctrl = Mods { ctrl: true, ..none };
        let alt = Mods { alt: true, ..none };
        let altgr = Mods { ctrl: true, alt: true, ..none };
        let a = || Some('a');
        assert_eq!(map_key(0x41, none, a), Some(Key::Type('a')));
        assert_eq!(map_key(0x41, ctrl, a), None, "Ctrl+A goes to the app");
        assert_eq!(map_key(0x51, altgr, || Some('@')), Some(Key::Type('@')));
        assert_eq!(map_key(0x41, none, || None), None, "dead keys type nothing");
        assert_eq!(map_key(0x41, none, || Some('\u{1}')), None);
        assert_eq!(map_key(vk::BACK, none, a), Some(Key::Backspace));
        assert_eq!(map_key(vk::BACK, ctrl, a), Some(Key::BackspaceWord));
        assert_eq!(map_key(vk::ESCAPE, none, a), Some(Key::Escape));
        assert_eq!(map_key(vk::LEFT, none, a), Some(Key::Move(-1)));
        assert_eq!(map_key(vk::DOWN, none, a), Some(Key::Move(1)));
        assert_eq!(map_key(vk::HOME, none, a), Some(Key::First));
        assert_eq!(map_key(vk::END, none, a), Some(Key::Last));
        assert_eq!(map_key(vk::RETURN, none, a), Some(Key::Activate));
        assert_eq!(map_key(vk::RETURN, shift, a), Some(Key::Item(ItemKey::ShiftEnter)));
        assert_eq!(map_key(vk::RETURN, ctrl, a), Some(Key::Item(ItemKey::CtrlEnter)));
        assert_eq!(map_key(vk::DELETE, none, a), Some(Key::Item(ItemKey::Delete)));
        assert_eq!(map_key(vk::KEY_P, ctrl, a), Some(Key::Item(ItemKey::CtrlP)));
        assert_eq!(map_key(vk::KEY_Z, ctrl, a), Some(Key::Undo));
        assert_eq!(map_key(vk::TAB, none, a), Some(Key::Tab(1)));
        assert_eq!(map_key(vk::TAB, shift, a), Some(Key::Tab(-1)));
        assert_eq!(map_key(0x33, alt, a), Some(Key::Nth(2)));
        assert_eq!(map_key(vk::NUMPAD1, alt, a), Some(Key::Nth(0)));
        assert_eq!(map_key(0x46, alt, a), None, "Alt+F goes to the app");
        assert_eq!(map_key(0x20, none, || Some(' ')), Some(Key::Type(' ')));
    }

    #[test]
    fn ctrl_backspace_deletes_a_word() {
        let mut s = String::from("figma file  ");
        delete_word(&mut s);
        assert_eq!(s, "figma ");
        delete_word(&mut s);
        assert_eq!(s, "");
        delete_word(&mut s);
        assert_eq!(s, "");
    }
}
