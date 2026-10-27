//! The history hotkey: parsing a setting ("Win+Alt+V") into modifiers and a virtual key.

use windows::Win32::UI::Input::KeyboardAndMouse::*;

/// The default history hotkey: Win + Alt + V.
pub const HOTKEY_LABEL: &str = "Win+Alt+V";

/// The history hotkey, from settings ("Win+Alt+V", "Ctrl+Shift+V").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hotkey {
    /// `HOT_KEY_MODIFIERS` bits (MOD_WIN, MOD_ALT, ...), without MOD_NOREPEAT.
    pub modifiers: u32,
    pub vk: u32,
    pub label: String,
}

impl Default for Hotkey {
    fn default() -> Self {
        Self { modifiers: (MOD_WIN | MOD_ALT).0, vk: u32::from(b'V'), label: HOTKEY_LABEL.into() }
    }
}

impl Hotkey {
    /// "Mod+Mod+Key", case-insensitive. Modifiers: Win, Ctrl, Alt, Shift. Keys: A–Z, 0–9,
    /// F1–F24, a few named keys (Space, Insert, Home, ...) and punctuation the current layout
    /// types without modifiers ("Ctrl+`"). The label comes back normalised ("Win+Alt+V").
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        // "Ctrl++" means the plus key.
        let (mods, key) = match text.strip_suffix("++") {
            Some(mods) => (mods, "+"),
            None => text.rsplit_once('+').unwrap_or(("", text)),
        };
        let mut modifiers = 0;
        for part in mods.split('+').filter(|_| !mods.is_empty()) {
            modifiers |= match part.trim().to_ascii_lowercase().as_str() {
                "win" | "windows" | "super" | "meta" => MOD_WIN.0,
                "ctrl" | "control" => MOD_CONTROL.0,
                "alt" => MOD_ALT.0,
                "shift" => MOD_SHIFT.0,
                _ => return None,
            };
        }
        let (vk, name) = key_code(key.trim())?;
        // A bare letter or symbol would swallow normal typing; only function keys may go alone.
        if modifiers == 0 && !(0x70..=0x87).contains(&vk) {
            return None;
        }
        let mut label = String::new();
        for (bit, part) in [(MOD_WIN, "Win+"), (MOD_CONTROL, "Ctrl+"), (MOD_ALT, "Alt+"), (MOD_SHIFT, "Shift+")] {
            if modifiers & bit.0 != 0 {
                label.push_str(part);
            }
        }
        label.push_str(&name);
        Some(Self { modifiers, vk, label })
    }
}

/// A key name → its virtual-key code and display name.
fn key_code(key: &str) -> Option<(u32, String)> {
    let mut chars = key.chars();
    let first = chars.next()?;
    if chars.as_str().is_empty() {
        let upper = first.to_ascii_uppercase();
        if upper.is_ascii_alphanumeric() {
            return Some((u32::from(upper), upper.to_string()));
        }
        // Punctuation sits on different keys per layout, so ask the current one; only take
        // characters it types without Shift/Ctrl/Alt, or the label would name the wrong chord.
        let mut units = [0u16; 2];
        if first.encode_utf16(&mut units).len() != 1 {
            return None;
        }
        let scan = unsafe { VkKeyScanW(units[0]) } as u16;
        let (vk, shift_state) = (scan & 0xFF, scan >> 8);
        return (vk != 0xFF && shift_state == 0).then(|| (u32::from(vk), first.to_string()));
    }
    let lower = key.to_ascii_lowercase();
    if let Some(n) = lower.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()).filter(|n| (1..=24).contains(n)) {
        return Some((0x6F + n, format!("F{n}")));
    }
    let named: [(&[&str], VIRTUAL_KEY, &str); 11] = [
        (&["space"], VK_SPACE, "Space"),
        (&["insert", "ins"], VK_INSERT, "Insert"),
        (&["delete", "del"], VK_DELETE, "Delete"),
        (&["home"], VK_HOME, "Home"),
        (&["end"], VK_END, "End"),
        (&["pageup", "pgup"], VK_PRIOR, "PageUp"),
        (&["pagedown", "pgdn"], VK_NEXT, "PageDown"),
        (&["up"], VK_UP, "Up"),
        (&["down"], VK_DOWN, "Down"),
        (&["left"], VK_LEFT, "Left"),
        (&["right"], VK_RIGHT, "Right"),
    ];
    named.iter().find(|(names, ..)| names.contains(&lower.as_str())).map(|(_, vk, name)| (u32::from(vk.0), name.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkeys_parse_and_normalise() {
        assert_eq!(Hotkey::parse("win+alt+v"), Some(Hotkey::default()));
        assert_eq!(Hotkey::parse(" Alt + Win + V "), Some(Hotkey::default()));
        let key = Hotkey::parse("shift+CTRL+v").unwrap();
        assert_eq!((key.modifiers, key.vk, key.label.as_str()), ((MOD_CONTROL | MOD_SHIFT).0, u32::from(b'V'), "Ctrl+Shift+V"));
        let key = Hotkey::parse("alt+f9").unwrap();
        assert_eq!((key.modifiers, key.vk, key.label.as_str()), (MOD_ALT.0, 0x78, "Alt+F9"));
        assert_eq!(Hotkey::parse("F24").map(|k| (k.vk, k.label)), Some((0x87, "F24".into())));
        assert_eq!(Hotkey::parse("Ctrl+Shift+5").map(|k| k.label), Some("Ctrl+Shift+5".into()));
        assert_eq!(Hotkey::parse("ctrl+pgup").map(|k| (k.vk, k.label)), Some((0x21, "Ctrl+PageUp".into())));
        // Layout-dependent: only check the label when the layout types it unshifted.
        if let Some(key) = Hotkey::parse("ctrl+`") {
            assert_eq!(key.label, "Ctrl+`");
        }
        for bad in ["", "V", "Ctrl+", "Hyper+V", "Ctrl+F25", "Ctrl+F0", "Ctrl+Banana", "Win+Alt+VV", "+V"] {
            assert_eq!(Hotkey::parse(bad), None, "{bad:?}");
        }
    }
}
