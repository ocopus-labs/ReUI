//! Hotkeys as the clipboard module writes and reads them ("Win+Alt+V"): the label format,
//! its key caps, and turning a pressed chord into one. Same rules as the hub's parser
//! (`clipboard/os/hotkey.rs`): modifiers Win, Ctrl, Alt, Shift in that order; keys A–Z,
//! 0–9, F1–F24, a few named keys, and punctuation the layout types unshifted; a key alone
//! only if it's a function key.

/// `RegisterHotKey` modifier bits.
pub const MOD_ALT: u32 = 0x1;
pub const MOD_CONTROL: u32 = 0x2;
pub const MOD_SHIFT: u32 = 0x4;
pub const MOD_WIN: u32 = 0x8;

const ORDER: [(u32, &str); 4] = [(MOD_WIN, "Win"), (MOD_CONTROL, "Ctrl"), (MOD_ALT, "Alt"), (MOD_SHIFT, "Shift")];

const NAMED: [(u32, &str); 11] = [
    (0x20, "Space"),
    (0x2D, "Insert"),
    (0x2E, "Delete"),
    (0x24, "Home"),
    (0x23, "End"),
    (0x21, "PageUp"),
    (0x22, "PageDown"),
    (0x26, "Up"),
    (0x28, "Down"),
    (0x25, "Left"),
    (0x27, "Right"),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hotkey {
    pub mods: u32,
    pub vk: u32,
    /// The key's name in the label ("V", "F9", "PageUp", "`").
    pub key: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Problem {
    /// A plain key would swallow normal typing.
    NeedsModifier,
    /// A key the clipboard module can't register (Tab, Enter, media keys...).
    Unsupported,
}

impl Hotkey {
    pub fn label(&self) -> String {
        let mut label = String::new();
        for (bit, name) in ORDER {
            if self.mods & bit != 0 {
                label.push_str(name);
                label.push('+');
            }
        }
        label.push_str(&self.key);
        label
    }

    /// A pressed chord. `char_of`: the character the layout types for a key unshifted
    /// (punctuation only matters here).
    pub fn from_keys(mods: u32, vk: u32, char_of: impl Fn(u32) -> Option<char>) -> Result<Self, Problem> {
        let key = key_name(vk).or_else(|| char_of(vk).filter(|c| c.is_ascii_graphic() && !c.is_ascii_alphanumeric()).map(String::from)).ok_or(Problem::Unsupported)?;
        if mods == 0 && !(0x70..=0x87).contains(&vk) {
            return Err(Problem::NeedsModifier);
        }
        Ok(Self { mods, vk, key })
    }

    /// Reads a label back. `vk_of`: the key a punctuation character is on (unshifted).
    pub fn parse(text: &str, vk_of: impl Fn(char) -> Option<u32>) -> Option<Self> {
        let caps = caps(text);
        let (key, mods) = caps.split_last()?;
        let mut bits = 0;
        for m in mods {
            bits |= match m.to_ascii_lowercase().as_str() {
                "win" | "windows" | "super" | "meta" => MOD_WIN,
                "ctrl" | "control" => MOD_CONTROL,
                "alt" => MOD_ALT,
                "shift" => MOD_SHIFT,
                _ => return None,
            };
        }
        let lower = key.to_ascii_lowercase();
        let vk = if key.chars().count() == 1 {
            let c = key.chars().next()?.to_ascii_uppercase();
            if c.is_ascii_alphanumeric() {
                u32::from(c)
            } else {
                vk_of(c)?
            }
        } else if let Some(n) = lower.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()).filter(|n| (1..=24).contains(n)) {
            0x6F + n
        } else {
            NAMED.iter().find(|(_, name)| name.eq_ignore_ascii_case(key) || (lower == "pgup" && *name == "PageUp") || (lower == "pgdn" && *name == "PageDown") || (lower == "del" && *name == "Delete") || (lower == "ins" && *name == "Insert")).map(|(vk, _)| *vk)?
        };
        Self::from_keys(bits, vk, |_| key.chars().next()).ok()
    }

    /// The same chord with Shift added (or Alt, if it has Shift), to suggest when this one is
    /// taken.
    pub fn alternative(&self) -> Option<Self> {
        let extra = [MOD_SHIFT, MOD_ALT, MOD_CONTROL].into_iter().find(|bit| self.mods & bit == 0)?;
        Some(Self { mods: self.mods | extra, ..self.clone() })
    }
}

/// Key caps of a label: "Ctrl+Shift+V" → ["Ctrl", "Shift", "V"]; "Ctrl++" → ["Ctrl", "+"].
pub fn caps(label: &str) -> Vec<String> {
    let label = label.trim();
    let (mods, key) = match label.strip_suffix("++") {
        Some(mods) => (mods, "+"),
        None => label.rsplit_once('+').unwrap_or(("", label)),
    };
    let mut caps: Vec<String> = mods.split('+').map(str::trim).filter(|m| !m.is_empty()).map(String::from).collect();
    if !key.trim().is_empty() {
        caps.push(key.trim().to_string());
    }
    caps
}

fn key_name(vk: u32) -> Option<String> {
    match vk {
        0x30..=0x39 | 0x41..=0x5A => char::from_u32(vk).map(String::from),
        0x70..=0x87 => Some(format!("F{}", vk - 0x6F)),
        _ => NAMED.iter().find(|(v, _)| *v == vk).map(|(_, name)| name.to_string()),
    }
}

/// A modifier key's bit (either side), or `None` for any other key.
pub fn modifier_bit(vk: u32) -> Option<u32> {
    match vk {
        0x5B | 0x5C => Some(MOD_WIN),             // VK_LWIN, VK_RWIN
        0x11 | 0xA2 | 0xA3 => Some(MOD_CONTROL),  // VK_CONTROL, VK_LCONTROL, VK_RCONTROL
        0x12 | 0xA4 | 0xA5 => Some(MOD_ALT),      // VK_MENU, VK_LMENU, VK_RMENU
        0x10 | 0xA0 | 0xA1 => Some(MOD_SHIFT),    // VK_SHIFT, VK_LSHIFT, VK_RSHIFT
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_layout(_: u32) -> Option<char> {
        None
    }

    #[test]
    fn labels_follow_the_modules_order() {
        let key = Hotkey::from_keys(MOD_SHIFT | MOD_WIN | MOD_ALT, u32::from(b'V'), no_layout).unwrap();
        assert_eq!(key.label(), "Win+Alt+Shift+V");
        assert_eq!(Hotkey::from_keys(MOD_CONTROL, 0x78, no_layout).unwrap().label(), "Ctrl+F9");
        assert_eq!(Hotkey::from_keys(0, 0x87, no_layout).unwrap().label(), "F24");
        assert_eq!(Hotkey::from_keys(MOD_ALT, 0x21, no_layout).unwrap().label(), "Alt+PageUp");
        assert_eq!(Hotkey::from_keys(MOD_CONTROL, 0xC0, |_| Some('`')).unwrap().label(), "Ctrl+`");
    }

    #[test]
    fn chords_the_module_cant_use_are_refused() {
        assert_eq!(Hotkey::from_keys(0, u32::from(b'V'), no_layout), Err(Problem::NeedsModifier));
        assert_eq!(Hotkey::from_keys(MOD_CONTROL, 0x09, no_layout), Err(Problem::Unsupported)); // Tab
        assert_eq!(Hotkey::from_keys(MOD_CONTROL, 0xBA, |_| Some('é')), Err(Problem::Unsupported));
    }

    #[test]
    fn labels_parse_back() {
        let vk_of = |c: char| (c == '`').then_some(0xC0);
        for label in ["Win+Alt+V", "Ctrl+Shift+5", "Alt+F9", "F24", "Ctrl+PageDown", "Ctrl+`", "Win+Space"] {
            assert_eq!(Hotkey::parse(label, vk_of).map(|k| k.label()).as_deref(), Some(label));
        }
        assert_eq!(Hotkey::parse(" alt + win + v ", vk_of).unwrap().label(), "Win+Alt+V");
        assert_eq!(Hotkey::parse("ctrl+pgup", vk_of).unwrap().vk, 0x21);
        for bad in ["", "V", "Ctrl+", "Hyper+V", "Ctrl+F25", "Ctrl+Banana", "Ctrl+~"] {
            assert_eq!(Hotkey::parse(bad, vk_of), None, "{bad:?}");
        }
    }

    #[test]
    fn caps_split_labels_including_the_plus_key() {
        assert_eq!(caps("Win+Alt+V"), ["Win", "Alt", "V"]);
        assert_eq!(caps("Ctrl++"), ["Ctrl", "+"]);
        assert_eq!(caps("F9"), ["F9"]);
        assert!(caps("").is_empty());
    }

    #[test]
    fn alternatives_add_a_modifier() {
        let key = Hotkey::parse("Win+Alt+V", |_| None).unwrap();
        assert_eq!(key.alternative().unwrap().label(), "Win+Alt+Shift+V");
        let full = Hotkey::parse("Win+Ctrl+Alt+Shift+V", |_| None).unwrap();
        assert_eq!(full.alternative(), None);
    }

    #[test]
    fn modifiers_are_recognised_on_both_sides() {
        assert_eq!(modifier_bit(0x5C), Some(MOD_WIN));
        assert_eq!(modifier_bit(0xA3), Some(MOD_CONTROL));
        assert_eq!(modifier_bit(0xA4), Some(MOD_ALT));
        assert_eq!(modifier_bit(0xA0), Some(MOD_SHIFT));
        assert_eq!(modifier_bit(u32::from(b'V')), None);
    }
}
