//! Which model a device is, from everything known about it, in the research doc's order
//! (docs/research-device-personalization.md §5.1): the user's choice, the Device ID record,
//! the Apple advert, a non-generic Windows container icon, the name, the class. Each step
//! only speaks when it's sure; a step that doesn't know hands on to the next, so a
//! white-label pair of buds named "AirPods" gets the AirPods outline but never the name
//! "AirPods Pro (2nd generation)". Pure.

use super::super::kinds::Kind;
use super::{apple, lookup, Colour, DeviceIds, Family, Lookup, Model};

/// What's known about one device.
#[derive(Debug, Clone, Copy, Default)]
pub struct Known<'a> {
    /// The user's own choice in Settings (not built yet; always `None` for now).
    pub chosen: Option<&'static Model>,
    /// The Device ID record (devnode properties or hardware ids).
    pub ids: Option<DeviceIds>,
    /// An accepted Apple proximity-pairing advert: its product id and colour.
    pub advert: Option<(u16, Option<Colour>)>,
    /// The family a non-generic Windows container icon shows (phase 3; always `None` now).
    pub icon: Option<Family>,
    /// The Bluetooth name (usually the user's own).
    pub name: &'a str,
    /// The kind from the class of device / appearance, refined by the name (`kinds.rs`).
    pub kind: Kind,
}

/// Which step decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Step {
    Chosen,
    Ids,
    Advert,
    Icon,
    Name,
    Class,
    #[default]
    Nothing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Identity {
    /// The exact model, when a step named one.
    pub model: Option<&'static Model>,
    /// The outline family, when known.
    pub family: Option<Family>,
    /// Always set: the family's kind, else the guessed one (`Other` draws as Bluetooth).
    pub kind: Kind,
    pub colour: Option<Colour>,
    pub by: Step,
}

impl Identity {
    pub fn name(&self) -> Option<&'static str> {
        self.model.map(|m| m.name)
    }
}

pub fn resolve(k: &Known) -> Identity {
    let advert_colour = k.advert.and_then(|(_, c)| c);
    let exact = |model: &'static Model, by: Step| Identity { model: Some(model), family: Some(model.family), kind: model.kind(), colour: advert_colour.or(model.colour), by };
    if let Some(model) = k.chosen {
        return exact(model, Step::Chosen);
    }
    if let Some(Lookup::Model(model)) = k.ids.map(lookup) {
        return exact(model, Step::Ids);
    }
    if let Some(model) = k.advert.and_then(|(pid, _)| apple(pid)) {
        return exact(model, Step::Advert);
    }
    let family = |family: Family, by| Identity { model: None, family: Some(family), kind: family.kind(), colour: advert_colour, by };
    if let Some(f) = k.icon {
        return family(f, Step::Icon);
    }
    if let Some(f) = name_family(k.name, k.kind) {
        return family(f, Step::Name);
    }
    match Family::of_kind(k.kind) {
        Some(f) => family(f, Step::Class),
        None => Identity { kind: k.kind, colour: advert_colour, ..Identity::default() },
    }
}

/// A brand family from the name, only where the name is unmistakable. Names are the user's
/// (and copycats copy them), so this never yields a model, only an outline.
pub fn name_family(name: &str, kind: Kind) -> Option<Family> {
    let n = name.to_lowercase();
    let has = |w: &str| n.contains(w);
    Some(if has("airpods max") {
        Family::AirpodsMax
    } else if has("airpods pro") {
        Family::AirpodsPro
    } else if has("airpods") {
        Family::Airpods
    } else if has("galaxy buds") {
        Family::GalaxyBuds
    } else if has("powerbeats") {
        Family::BeatsEarbuds
    } else if has("beats") {
        match kind {
            Kind::Headphones | Kind::Headset => Family::BeatsHeadphones,
            Kind::Speaker => Family::Speaker,
            _ => Family::BeatsEarbuds,
        }
    } else if has("dualsense") {
        Family::Dualsense
    } else if has("dualshock") {
        Family::Dualshock
    } else if has("pro controller") && kind == Kind::Gamepad {
        Family::SwitchPro
    } else if has("xbox") && kind == Kind::Gamepad {
        Family::XboxController
    } else {
        return None;
    })
}

#[cfg(test)]
mod tests {
    use super::super::{VidSource, APPLE};
    use super::*;

    fn bt(vid: u16, pid: u16) -> Option<DeviceIds> {
        Some(DeviceIds { source: VidSource::Bluetooth, vid, pid })
    }

    #[test]
    fn devices_resolve_the_user_choice_wins() {
        let max = apple(0x202D).unwrap();
        let id = resolve(&Known { chosen: Some(max), ids: bt(APPLE, 0x2014), name: "AirPods Pro", kind: Kind::Earbuds, ..Default::default() });
        assert_eq!((id.name(), id.kind, id.by), (Some("AirPods Max 2"), Kind::Headphones, Step::Chosen));
    }

    #[test]
    fn devices_resolve_ids_name_the_model() {
        let id = resolve(&Known { ids: bt(APPLE, 0x2014), name: "Rohit's AirPods", kind: Kind::Earbuds, ..Default::default() });
        assert_eq!((id.name(), id.family, id.kind, id.colour, id.by), (Some("AirPods Pro (2nd generation)"), Some(Family::AirpodsPro), Kind::Earbuds, Some(Colour::White), Step::Ids));
        // A renamed AirPods Max classed as a headset is headphones.
        let max = resolve(&Known { ids: bt(APPLE, 0x200A), name: "Living room", kind: Kind::Headset, ..Default::default() });
        assert_eq!((max.family, max.kind), (Some(Family::AirpodsMax), Kind::Headphones));
        // The advert's colour beats the catalog's (only set for one-colour models anyway).
        let grey = resolve(&Known { ids: bt(APPLE, 0x200A), advert: Some((0x200A, Some(Colour::SpaceGrey))), ..Default::default() });
        assert_eq!(grey.colour, Some(Colour::SpaceGrey));
        // A controller's USB ids.
        let pad = resolve(&Known { ids: Some(DeviceIds { source: VidSource::Usb, vid: 0x045E, pid: 0x0B13 }), name: "Xbox Wireless Controller", kind: Kind::Gamepad, ..Default::default() });
        assert_eq!((pad.name(), pad.family, pad.kind), (Some("Xbox Series X|S Controller"), Some(Family::XboxController), Kind::Gamepad));
    }

    #[test]
    fn devices_resolve_advert_names_the_model_without_ids() {
        let id = resolve(&Known { advert: Some((0x201B, Some(Colour::White))), name: "AirPods", kind: Kind::Earbuds, ..Default::default() });
        assert_eq!((id.name(), id.by), (Some("AirPods 4 (ANC)"), Step::Advert));
        // An advert model the catalog doesn't know names nothing.
        let unknown = resolve(&Known { advert: Some((0x20FE, None)), name: "AirPods", kind: Kind::Earbuds, ..Default::default() });
        assert_eq!((unknown.model, unknown.family, unknown.by), (None, Some(Family::Airpods), Step::Name));
    }

    #[test]
    fn devices_resolve_icon_hook_comes_before_names() {
        let id = resolve(&Known { icon: Some(Family::Headphones), name: "AirPods", kind: Kind::Earbuds, ..Default::default() });
        assert_eq!((id.model, id.family, id.kind, id.by), (None, Some(Family::Headphones), Kind::Headphones, Step::Icon));
    }

    #[test]
    fn devices_resolve_refuses_to_guess_from_chip_vendors() {
        // The "Buds 3r": Bluetrum's ids, a name that names no brand, the earbuds class.
        let buds = resolve(&Known { ids: bt(0x0642, 0x0220), name: "Buds 3r", kind: Kind::Earbuds, ..Default::default() });
        assert_eq!((buds.model, buds.family, buds.kind, buds.by), (None, Some(Family::Earbuds), Kind::Earbuds, Step::Class));
        // Copycat "AirPods Pro" on an Airoha chip: the outline, never the model name.
        let fake = resolve(&Known { ids: bt(0x0094, 0x2014), name: "AirPods Pro", kind: Kind::Earbuds, ..Default::default() });
        assert_eq!((fake.model, fake.family, fake.by), (None, Some(Family::AirpodsPro), Step::Name));
        // An Apple product id the catalog doesn't know yet: no name either.
        let new = resolve(&Known { ids: bt(APPLE, 0x20FE), name: "Rohit's headphones", kind: Kind::Headphones, ..Default::default() });
        assert_eq!((new.model, new.family, new.by), (None, Some(Family::Headphones), Step::Class));
    }

    #[test]
    fn devices_resolve_falls_back_to_the_class_and_then_nothing() {
        // "100 wireless": no Device ID record, a headset class, a name without a brand.
        let id = resolve(&Known { name: "100 wireless", kind: Kind::Headset, ..Default::default() });
        assert_eq!((id.family, id.kind.id(), id.by), (Some(Family::Headset), "headphones", Step::Class));
        let other = resolve(&Known { name: "Desk PC", kind: Kind::Other, ..Default::default() });
        assert_eq!((other.family, other.kind.id(), other.by), (None, "bluetooth", Step::Nothing));
        assert_eq!(resolve(&Known { name: "Pixel 9", kind: Kind::Phone, ..Default::default() }).family, Some(Family::Phone));
        assert_eq!(resolve(&Known { name: "Surface Pen", kind: Kind::Pen, ..Default::default() }).family, Some(Family::Pen));
    }

    #[test]
    fn devices_resolve_name_families() {
        assert_eq!(name_family("Rohit's AirPods Max", Kind::Headphones), Some(Family::AirpodsMax));
        assert_eq!(name_family("Galaxy Buds2 Pro", Kind::Earbuds), Some(Family::GalaxyBuds));
        assert_eq!(name_family("Beats Studio Pro", Kind::Headphones), Some(Family::BeatsHeadphones));
        assert_eq!(name_family("Beats Pill", Kind::Speaker), Some(Family::Speaker));
        assert_eq!(name_family("Beats Flex", Kind::Earbuds), Some(Family::BeatsEarbuds));
        assert_eq!(name_family("Powerbeats Pro", Kind::Headset), Some(Family::BeatsEarbuds));
        assert_eq!(name_family("DualSense Wireless Controller", Kind::Gamepad), Some(Family::Dualsense));
        assert_eq!(name_family("Xbox Wireless Controller", Kind::Gamepad), Some(Family::XboxController));
        assert_eq!(name_family("Pro Controller", Kind::Gamepad), Some(Family::SwitchPro));
        // Plain "buds" isn't Samsung, and generic words are the class's job.
        assert_eq!(name_family("Buds 3r", Kind::Earbuds), None);
        assert_eq!(name_family("MX Master 3S", Kind::Mouse), None);
    }
}
