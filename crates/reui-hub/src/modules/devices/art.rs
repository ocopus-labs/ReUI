//! ReUI's own line drawings of devices (`design/assets/devices/`), looked up by id.
//!
//! The island shows one drawing per **kind** (earbuds, headphones, a gamepad…): what a
//! device is, not whose it is. Model-family ids (`airpods-pro`, `dualsense`…) are accepted
//! too and resolve to their kind's drawing, so the family mapping can name the family now
//! and get its own drawing later without a change here beyond the table.
//!
//! The SVG is single-colour (`stroke="currentColor"`): a module sends it as
//! `Art::Logo { image: ctx.image(svg.to_vec()), animated: false }` and the island inks it
//! in the label colour of its theme, so dark and light need no extra assets. `small` picks
//! the simplified drawing for the compact pill (24 DIP); the other is for the open card's
//! 56 DIP tile.

/// One drawing at both detail levels.
struct Art {
    id: &'static str,
    full: &'static [u8],
    small: &'static [u8],
}

macro_rules! art {
    ($id:literal) => {
        Art {
            id: $id,
            full: include_bytes!(concat!("../../../../../design/assets/devices/", $id, ".svg")),
            small: include_bytes!(concat!("../../../../../design/assets/devices/", $id, "-small.svg")),
        }
    };
}

/// Every kind with a drawing. `bluetooth` is the fallback for a device of unknown kind.
const KINDS: [Art; 10] = [
    art!("earbuds"),
    art!("headphones"),
    art!("speaker"),
    art!("gamepad"),
    art!("mouse"),
    art!("keyboard"),
    art!("pen"),
    art!("watch"),
    art!("phone"),
    art!("bluetooth"),
];

/// Model families and the kind they're drawn as. Brand silhouettes were left out on
/// purpose: one quiet drawing per kind reads better at 24 DIP than eleven near-alike ones.
const FAMILIES: [(&str, &str); 11] = [
    ("airpods", "earbuds"),
    ("airpods-pro", "earbuds"),
    ("beats-earbuds", "earbuds"),
    ("galaxy-buds", "earbuds"),
    ("airpods-max", "headphones"),
    ("beats-headphones", "headphones"),
    ("headset", "headphones"),
    ("xbox-controller", "gamepad"),
    ("dualsense", "gamepad"),
    ("dualshock", "gamepad"),
    ("switch-pro", "gamepad"),
];

/// The SVG for a kind or model-family `id`, simplified for the compact pill when `small`.
/// `None` for an id without a drawing (the caller keeps its Fluent glyph).
pub fn svg(id: &str, small: bool) -> Option<&'static [u8]> {
    let kind = FAMILIES.iter().find(|(family, _)| *family == id).map_or(id, |(_, kind)| kind);
    let art = KINDS.iter().find(|a| a.id == kind)?;
    Some(if small { art.small } else { art.full })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(bytes: &[u8]) -> &str {
        std::str::from_utf8(bytes).expect("UTF-8")
    }

    #[test]
    fn every_kind_has_both_drawings() {
        for art in &KINDS {
            for small in [false, true] {
                let svg = text(svg(art.id, small).unwrap_or_else(|| panic!("{} small={small}", art.id)));
                assert!(svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""), "{}: an SVG root first", art.id);
                assert!(svg.trim_end().ends_with("</svg>"), "{}", art.id);
                // The island inks the drawing; a colour of its own would ignore the theme.
                assert!(!svg.contains('#'), "{}: no colours of its own", art.id);
                assert!(svg.contains("currentColor"), "{}", art.id);
                // The grid the drawing is made on (see design/assets/devices/README.md).
                let grid = if small { "viewBox=\"0 0 21 21\"" } else { "viewBox=\"0 0 48 48\"" };
                assert!(svg.contains(grid), "{} small={small}: on its grid", art.id);
                // Images go over the pipe once per surface; these are a few hundred bytes.
                assert!((100..4096).contains(&svg.len()), "{}: {} bytes", art.id, svg.len());
            }
            assert_ne!(art.full, art.small, "{}: two detail levels", art.id);
        }
    }

    #[test]
    fn families_resolve_to_their_kind() {
        assert_eq!(svg("airpods-pro", false), svg("earbuds", false));
        assert_eq!(svg("airpods-max", true), svg("headphones", true));
        assert_eq!(svg("dualsense", false), svg("gamepad", false));
        for (family, kind) in FAMILIES {
            assert!(KINDS.iter().any(|a| a.id == kind), "{family} → {kind}");
            assert!(svg(family, true).is_some(), "{family}");
        }
    }

    #[test]
    fn unknown_ids_have_no_drawing() {
        assert_eq!(svg("toaster", false), None);
        assert_eq!(svg("", true), None);
        assert_eq!(svg("Earbuds", false), None, "ids are exact");
    }
}
