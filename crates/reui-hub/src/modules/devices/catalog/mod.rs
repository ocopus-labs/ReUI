//! The device catalog: what a connected device exactly is, from what Windows and the device
//! already say, offline (docs/research-device-personalization.md, phase 1).
//!
//! - `ids.rs`: the Device ID profile's vendor / product ids, from devnode properties or
//!   hardware ids;
//! - `apple.rs`, `gamepads.rs`: the models, keyed by those ids (and, for Apple, the same
//!   product id the proximity-pairing advert carries);
//! - `vendors.rs`: chip vendors whose ids name the silicon, not the product;
//! - `resolve.rs`: the lookup order over everything known about one device.
//!
//! A model gives the exact name (the connect card's detail line), the family (one outline
//! drawing each) and through it the kind (one glyph each, `Kind::id`); colour comes from the
//! Apple advert. The catalog is bundled data, never
//! looked up online: nothing about the user's devices leaves the PC. Every table row cites
//! where its facts come from. Adding a device is a PR.

mod apple;
mod gamepads;
pub mod ids;
pub mod resolve;
mod vendors;

use super::kinds::Kind;

pub use self::ids::{DeviceIds, VidSource};
pub use self::resolve::{resolve, Identity, Known};

/// Apple's Bluetooth SIG company id.
pub const APPLE: u16 = 0x004C;

/// A model family: one outline drawing each. The ids are shared with the art, so they never
/// change once shipped. Every family is of one kind (`Family::kind`); the glyph comes from
/// the kind unless art for the family is wired in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Family {
    Airpods,
    AirpodsPro,
    AirpodsMax,
    BeatsEarbuds,
    BeatsHeadphones,
    GalaxyBuds,
    Earbuds,
    Headphones,
    Headset,
    Speaker,
    XboxController,
    Dualsense,
    Dualshock,
    SwitchPro,
    Gamepad,
    Mouse,
    Keyboard,
    Pen,
    Watch,
    Phone,
}

impl Family {
    // Read by the island's per-family art once it's wired in (research doc phase 2).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn id(self) -> &'static str {
        match self {
            Family::Airpods => "airpods",
            Family::AirpodsPro => "airpods-pro",
            Family::AirpodsMax => "airpods-max",
            Family::BeatsEarbuds => "beats-earbuds",
            Family::BeatsHeadphones => "beats-headphones",
            Family::GalaxyBuds => "galaxy-buds",
            Family::Earbuds => "earbuds",
            Family::Headphones => "headphones",
            Family::Headset => "headset",
            Family::Speaker => "speaker",
            Family::XboxController => "xbox-controller",
            Family::Dualsense => "dualsense",
            Family::Dualshock => "dualshock",
            Family::SwitchPro => "switch-pro",
            Family::Gamepad => "gamepad",
            Family::Mouse => "mouse",
            Family::Keyboard => "keyboard",
            Family::Pen => "pen",
            Family::Watch => "watch",
            Family::Phone => "phone",
        }
    }

    pub fn kind(self) -> Kind {
        match self {
            Family::Airpods | Family::AirpodsPro | Family::BeatsEarbuds | Family::GalaxyBuds | Family::Earbuds => Kind::Earbuds,
            Family::AirpodsMax | Family::BeatsHeadphones | Family::Headphones => Kind::Headphones,
            Family::Headset => Kind::Headset,
            Family::Speaker => Kind::Speaker,
            Family::XboxController | Family::Dualsense | Family::Dualshock | Family::SwitchPro | Family::Gamepad => Kind::Gamepad,
            Family::Mouse => Kind::Mouse,
            Family::Keyboard => Kind::Keyboard,
            Family::Pen => Kind::Pen,
            Family::Watch => Kind::Watch,
            Family::Phone => Kind::Phone,
        }
    }

    /// The generic family for a kind: what's left when nothing names the model.
    pub fn of_kind(kind: Kind) -> Option<Family> {
        match kind {
            Kind::Earbuds => Some(Family::Earbuds),
            Kind::Headphones => Some(Family::Headphones),
            Kind::Headset => Some(Family::Headset),
            Kind::Speaker => Some(Family::Speaker),
            Kind::Gamepad => Some(Family::Gamepad),
            Kind::Mouse => Some(Family::Mouse),
            Kind::Keyboard => Some(Family::Keyboard),
            Kind::Pen => Some(Family::Pen),
            Kind::Watch => Some(Family::Watch),
            Kind::Phone => Some(Family::Phone),
            Kind::Touchpad | Kind::Other => None,
        }
    }
}

/// A device's colour, as plain names (marketing names like "Midnight" aren't needed on the
/// island). The art can pick a fill from it so a black device doesn't vanish on black.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Colour {
    White,
    Black,
    Red,
    Blue,
    Pink,
    Grey,
    Silver,
    Gold,
    RoseGold,
    SpaceGrey,
    DarkBlue,
    LightBlue,
    Yellow,
}

impl Colour {
    // Read by the island's art once it's wired in (research doc phase 2).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn id(self) -> &'static str {
        match self {
            Colour::White => "white",
            Colour::Black => "black",
            Colour::Red => "red",
            Colour::Blue => "blue",
            Colour::Pink => "pink",
            Colour::Grey => "grey",
            Colour::Silver => "silver",
            Colour::Gold => "gold",
            Colour::RoseGold => "rose-gold",
            Colour::SpaceGrey => "space-grey",
            Colour::DarkBlue => "dark-blue",
            Colour::LightBlue => "light-blue",
            Colour::Yellow => "yellow",
        }
    }

    /// The colour byte of Apple's proximity-pairing advert. Codes as documented in
    /// fischejo/airpods-notify's `doc/proximity_protocol.md` (the community's reading, [RE]);
    /// codes seen but not understood (0x10 on AirPods Max, 0x11 on Powerbeats Pro) and
    /// anything else are `None`.
    pub fn from_apple(code: u8) -> Option<Colour> {
        Some(match code {
            0x00 => Colour::White,
            0x01 => Colour::Black,
            0x02 => Colour::Red,
            0x03 => Colour::Blue,
            0x04 => Colour::Pink,
            0x05 => Colour::Grey,
            0x06 => Colour::Silver,
            0x07 => Colour::Gold,
            0x08 => Colour::RoseGold,
            0x09 => Colour::SpaceGrey,
            0x0A => Colour::DarkBlue,
            0x0B => Colour::LightBlue,
            0x0C => Colour::Yellow,
            _ => return None,
        })
    }
}

/// One product. `name` is the marketed name, as the vendor writes it; `colour` is set only
/// when the product comes in that one colour.
#[derive(Debug, PartialEq, Eq)]
pub struct Model {
    pub vid: u16,
    pub pid: u16,
    pub name: &'static str,
    pub family: Family,
    pub colour: Option<Colour>,
    /// Where the id and name come from.
    pub source: &'static str,
}

impl Model {
    pub fn kind(&self) -> Kind {
        self.family.kind()
    }
}

/// What the ids say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lookup {
    Model(&'static Model),
    /// The vendor id is a chip maker's (white-label buds): it says nothing about the product.
    ChipVendor,
    Unknown,
}

/// The model for a set of Device ID values.
pub fn lookup(ids: DeviceIds) -> Lookup {
    if vendors::is_chip_vendor(ids.source, ids.vid) {
        return Lookup::ChipVendor;
    }
    let table: &[Model] = match ids.source {
        VidSource::Bluetooth if ids.vid == APPLE => apple::MODELS,
        VidSource::Usb => gamepads::MODELS,
        _ => &[],
    };
    table.iter().find(|m| m.vid == ids.vid && m.pid == ids.pid).map_or(Lookup::Unknown, Lookup::Model)
}

/// An Apple or Beats model by its product id (the Device ID PID, which is also the model
/// field of its proximity-pairing advert).
pub fn apple(pid: u16) -> Option<&'static Model> {
    apple::MODELS.iter().find(|m| m.pid == pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_catalog_family_ids_and_kinds() {
        use Family::*;
        let all = [Airpods, AirpodsPro, AirpodsMax, BeatsEarbuds, BeatsHeadphones, GalaxyBuds, Earbuds, Headphones, Headset, Speaker, XboxController, Dualsense, Dualshock, SwitchPro, Gamepad, Mouse, Keyboard, Pen, Watch, Phone];
        let ids: Vec<(&str, &str)> = all.iter().map(|f| (f.id(), f.kind().id())).collect();
        assert_eq!(
            ids,
            [
                ("airpods", "earbuds"),
                ("airpods-pro", "earbuds"),
                ("airpods-max", "headphones"),
                ("beats-earbuds", "earbuds"),
                ("beats-headphones", "headphones"),
                ("galaxy-buds", "earbuds"),
                ("earbuds", "earbuds"),
                ("headphones", "headphones"),
                ("headset", "headphones"),
                ("speaker", "speaker"),
                ("xbox-controller", "gamepad"),
                ("dualsense", "gamepad"),
                ("dualshock", "gamepad"),
                ("switch-pro", "gamepad"),
                ("gamepad", "gamepad"),
                ("mouse", "mouse"),
                ("keyboard", "keyboard"),
                ("pen", "pen"),
                ("watch", "watch"),
                ("phone", "phone"),
            ]
        );
        // Every generic family comes back from its kind.
        for f in all.iter().filter(|f| !matches!(f, Airpods | AirpodsPro | AirpodsMax | BeatsEarbuds | BeatsHeadphones | GalaxyBuds | XboxController | Dualsense | Dualshock | SwitchPro)) {
            assert_eq!(Family::of_kind(f.kind()), Some(*f));
        }
        assert_eq!(Family::of_kind(Kind::Other), None);
    }

    #[test]
    fn devices_catalog_looks_up_by_ids() {
        let bt = |vid, pid| DeviceIds { source: VidSource::Bluetooth, vid, pid };
        let usb = |vid, pid| DeviceIds { source: VidSource::Usb, vid, pid };
        let Lookup::Model(pro2) = lookup(bt(APPLE, 0x2014)) else { panic!("AirPods Pro 2") };
        assert_eq!((pro2.name, pro2.family, pro2.kind(), pro2.colour), ("AirPods Pro (2nd generation)", Family::AirpodsPro, Kind::Earbuds, Some(Colour::White)));
        let Lookup::Model(max) = lookup(bt(APPLE, 0x202D)) else { panic!("AirPods Max 2") };
        assert_eq!((max.family, max.kind(), max.colour), (Family::AirpodsMax, Kind::Headphones, None));
        let Lookup::Model(pad) = lookup(usb(0x054C, 0x0CE6)) else { panic!("DualSense") };
        assert_eq!((pad.name, pad.family, pad.kind()), ("DualSense", Family::Dualsense, Kind::Gamepad));
        // Bluetrum, the chip in the "Buds 3r" on the PC this was written on.
        assert_eq!(lookup(bt(0x0642, 0x0220)), Lookup::ChipVendor);
        // An Apple id the catalog doesn't know yet, and Apple's id read as a USB vendor.
        assert_eq!(lookup(bt(APPLE, 0x2FFF)), Lookup::Unknown);
        assert_eq!(lookup(usb(APPLE, 0x2014)), Lookup::Unknown);
        assert_eq!(apple(0x200E).map(|m| m.name), Some("AirPods Pro (1st generation)"));
        assert_eq!(apple::MODELS.len() + gamepads::MODELS.len(), 70);
    }

    #[test]
    fn devices_catalog_tables_are_unique_and_sourced() {
        for table in [apple::MODELS, gamepads::MODELS] {
            for (i, m) in table.iter().enumerate() {
                assert!(!m.name.is_empty() && !m.source.is_empty(), "{m:?}");
                assert!(m.kind().announces(), "{m:?}");
                assert!(table[..i].iter().all(|o| (o.vid, o.pid) != (m.vid, m.pid)), "duplicate {m:?}");
            }
        }
    }

    #[test]
    fn devices_catalog_apple_colours() {
        assert_eq!(Colour::from_apple(0x00), Some(Colour::White));
        assert_eq!(Colour::from_apple(0x09), Some(Colour::SpaceGrey));
        assert_eq!(Colour::from_apple(0x0C), Some(Colour::Yellow));
        assert_eq!(Colour::from_apple(0x10), None);
        assert_eq!(Colour::RoseGold.id(), "rose-gold");
    }
}
