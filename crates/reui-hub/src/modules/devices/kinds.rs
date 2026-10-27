//! What kind of device something is (Bluetooth Class of Device, LE Appearance, a name hint as
//! the last resort) and its Segoe Fluent Icons glyph.
//!
//! Glyphs:
//! - E7F6 Headphone (headphones), F4C0 Earbud (earbuds), E95B Headset, E7F5 Speakers;
//! - E962 Mouse, E92E KeyboardStandard, E7FC Game (controllers), EFA5 Touchpad;
//! - E8EA CellPhone (phones, iPhone included);
//! - E916 Stopwatch for watches (the font has no watch glyph);
//! - E702 Bluetooth for anything else;
//! - E945 LightningBolt (charging), E850–E859 / E83F Battery0–10 (the laptop by level);
//! - EDA2 HardDrive, E7F1 SDCard (drives); E772 Devices (the overview).

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Kind {
    Headphones,
    Earbuds,
    Headset,
    Speaker,
    Mouse,
    Keyboard,
    Touchpad,
    Gamepad,
    Phone,
    Watch,
    #[default]
    Other,
}

pub const GLYPH_BOLT: char = '\u{E945}';
pub const GLYPH_DRIVE: char = '\u{EDA2}';
pub const GLYPH_SD_CARD: char = '\u{E7F1}';
pub const GLYPH_DEVICES: char = '\u{E772}';
pub const GLYPH_PHONE: char = '\u{E8EA}';

impl Kind {
    pub fn glyph(self) -> char {
        match self {
            Kind::Headphones => '\u{E7F6}',
            Kind::Earbuds => '\u{F4C0}',
            Kind::Headset => '\u{E95B}',
            Kind::Speaker => '\u{E7F5}',
            Kind::Mouse => '\u{E962}',
            Kind::Keyboard => '\u{E92E}',
            Kind::Touchpad => '\u{EFA5}',
            Kind::Gamepad => '\u{E7FC}',
            Kind::Phone => GLYPH_PHONE,
            Kind::Watch => '\u{E916}',
            Kind::Other => '\u{E702}',
        }
    }

    pub fn is_audio(self) -> bool {
        matches!(self, Kind::Headphones | Kind::Earbuds | Kind::Headset | Kind::Speaker)
    }

    /// Kinds that get a connect card (D1: audio, input, phones and watches); the rest only
    /// show in the overview.
    pub fn announces(self) -> bool {
        self != Kind::Other
    }
}

/// Bluetooth Classic Class of Device: major class and the 6-bit minor class as Windows
/// reports them (`System.Devices.Aep.Bluetooth.Cod.Major` / `.Minor`).
pub fn from_cod(major: u32, minor: u32) -> Kind {
    match major {
        // Phone.
        2 => Kind::Phone,
        // Audio/video.
        4 => match minor {
            1 => Kind::Headset,
            2 => Kind::Headset,
            5 | 7 | 10 => Kind::Speaker,
            6 => Kind::Headphones,
            _ => Kind::Other,
        },
        // Peripheral: bits 4–5 keyboard / pointing, bits 0–3 the subtype.
        5 => match (minor & 0x30, minor & 0x0F) {
            (_, 1 | 2) => Kind::Gamepad,
            (0x10 | 0x30, _) => Kind::Keyboard,
            (0x20, _) => Kind::Mouse,
            _ => Kind::Other,
        },
        // Wearable: wristwatch.
        7 if minor == 1 => Kind::Watch,
        _ => Kind::Other,
    }
}

/// Bluetooth LE GAP Appearance: category in the top ten bits, subcategory in the low six.
pub fn from_appearance(appearance: u32) -> Kind {
    let (category, sub) = (appearance >> 6, appearance & 0x3F);
    match category {
        0x001 => Kind::Phone,
        0x003 => Kind::Watch,
        0x00F => match sub {
            1 => Kind::Keyboard,
            2 => Kind::Mouse,
            3 | 4 => Kind::Gamepad,
            9 => Kind::Touchpad,
            _ => Kind::Other,
        },
        // Audio sink: speakers, soundbars.
        0x021 => Kind::Speaker,
        // Wearable audio device: earbud, headset, headphones, neckband.
        0x025 => match sub {
            1 => Kind::Earbuds,
            2 => Kind::Headset,
            _ => Kind::Headphones,
        },
        // Hearing aids sit in the ear.
        0x029 => Kind::Earbuds,
        _ => Kind::Other,
    }
}

/// When the class says nothing (or only "audio"), the name usually does.
pub fn from_name(name: &str) -> Kind {
    let n = name.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| n.contains(w));
    if has(&["airpods max", "headphone", "wh-1000", "bose qc", "quietcomfort"]) {
        Kind::Headphones
    } else if has(&["airpods", "buds", "earbud", "wf-", "beats fit", "powerbeats", "studio buds"]) {
        Kind::Earbuds
    } else if has(&["headset"]) {
        Kind::Headset
    } else if has(&["mouse", "mx master", "mx anywhere"]) {
        Kind::Mouse
    } else if has(&["keyboard", "keys"]) {
        Kind::Keyboard
    } else if has(&["controller", "gamepad", "dualsense", "dualshock", "joy-con"]) {
        Kind::Gamepad
    } else if has(&["speaker", "soundbar", "soundlink", "boom"]) {
        Kind::Speaker
    } else if has(&["watch"]) {
        Kind::Watch
    } else if has(&["iphone", "pixel", "galaxy s", "phone"]) {
        Kind::Phone
    } else {
        Kind::Other
    }
}

/// The best guess from everything known: the class, refined by the name where the class is
/// coarse (Classic headphones and earbuds share a class), the name alone otherwise.
pub fn classify(cod: Option<(u32, u32)>, appearance: Option<u32>, name: &str) -> Kind {
    let by_name = from_name(name);
    let by_class = cod.map(|(major, minor)| from_cod(major, minor)).filter(|k| *k != Kind::Other).or_else(|| appearance.map(from_appearance).filter(|k| *k != Kind::Other));
    match by_class {
        // Earbuds advertise themselves as headsets or headphones.
        Some(Kind::Headset | Kind::Headphones) if by_name == Kind::Earbuds => Kind::Earbuds,
        Some(kind) => kind,
        None => by_name,
    }
}

/// Is this an Apple (or Beats) audio device, whose case and buds can be read from its
/// advertisements?
pub fn is_apple_audio(name: &str, kind: Kind) -> bool {
    let n = name.to_lowercase();
    kind.is_audio() && (n.contains("airpods") || n.contains("beats"))
}

/// The laptop's battery glyph by level (Battery0 … Battery10).
pub fn battery_glyph(percent: u32) -> char {
    match percent.min(100) / 10 {
        10 => '\u{E83F}',
        tenth => char::from_u32(0xE850 + tenth).unwrap_or('\u{E83F}'),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_class_of_device() {
        assert_eq!(from_cod(4, 1), Kind::Headset);
        assert_eq!(from_cod(4, 6), Kind::Headphones);
        assert_eq!(from_cod(4, 5), Kind::Speaker);
        assert_eq!(from_cod(5, 0x20), Kind::Mouse);
        assert_eq!(from_cod(5, 0x10), Kind::Keyboard);
        assert_eq!(from_cod(5, 0x30), Kind::Keyboard);
        assert_eq!(from_cod(5, 0x02), Kind::Gamepad);
        assert_eq!(from_cod(2, 3), Kind::Phone);
        assert_eq!(from_cod(7, 1), Kind::Watch);
        assert_eq!(from_cod(1, 3), Kind::Other);
    }

    #[test]
    fn devices_le_appearance() {
        assert_eq!(from_appearance(0x03C2), Kind::Mouse);
        assert_eq!(from_appearance(0x03C1), Kind::Keyboard);
        assert_eq!(from_appearance(0x03C4), Kind::Gamepad);
        assert_eq!(from_appearance(0x0941), Kind::Earbuds);
        assert_eq!(from_appearance(0x0943), Kind::Headphones);
        assert_eq!(from_appearance(0x0841), Kind::Speaker);
        assert_eq!(from_appearance(0x00C2), Kind::Watch);
        assert_eq!(from_appearance(0x0040), Kind::Phone);
        assert_eq!(from_appearance(0x0080), Kind::Other);
        assert_eq!(from_appearance(0), Kind::Other);
    }

    #[test]
    fn devices_classify_prefers_class_and_refines_earbuds() {
        assert_eq!(classify(Some((4, 1)), None, "Rohit's AirPods Pro"), Kind::Earbuds);
        assert_eq!(classify(Some((4, 1)), None, "Jabra Evolve"), Kind::Headset);
        assert_eq!(classify(None, Some(0x03C2), "MX Keys"), Kind::Mouse);
        assert_eq!(classify(None, None, "MX Keys Mini"), Kind::Keyboard);
        assert_eq!(classify(Some((1, 3)), None, "Desk PC"), Kind::Other);
        assert_eq!(classify(None, None, "Xbox Wireless Controller"), Kind::Gamepad);
        assert_eq!(classify(None, None, "AirPods Max"), Kind::Headphones);
    }

    #[test]
    fn devices_glyphs_and_announcing() {
        assert_eq!(Kind::Headphones.glyph(), '\u{E7F6}');
        assert_eq!(Kind::Earbuds.glyph(), '\u{F4C0}');
        assert_eq!(Kind::Mouse.glyph(), '\u{E962}');
        assert_eq!(Kind::Other.glyph(), '\u{E702}');
        assert!(Kind::Mouse.announces() && Kind::Watch.announces() && !Kind::Other.announces());
        assert!(Kind::Speaker.is_audio() && !Kind::Mouse.is_audio());
        assert!(is_apple_audio("AirPods Pro", Kind::Earbuds) && !is_apple_audio("AirPods Pro", Kind::Other) && !is_apple_audio("Galaxy Buds", Kind::Earbuds));
        assert_eq!(battery_glyph(0), '\u{E850}');
        assert_eq!(battery_glyph(47), '\u{E854}');
        assert_eq!(battery_glyph(100), '\u{E83F}');
        assert_eq!(battery_glyph(250), '\u{E83F}');
    }
}
