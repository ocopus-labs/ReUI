//! ReUI's own line drawings of devices (`design/assets/devices/`), one per kind, as bytes.
//!
//! Here rather than in the hub so there's one copy of them: the hub sends them to the
//! island as `Art::Logo` (`reui-hub/src/modules/devices/art.rs`, which also maps model
//! families to kinds), and Settings draws the same bytes in its Devices preview. Each is
//! single-colour (`currentColor`): whoever draws it sets the ink.

/// One drawing at both detail levels: `full` for the open card's 56 DIP spot (48 grid),
/// `small` for the compact pill's 24 DIP spot (21 grid).
struct Drawing {
    kind: &'static str,
    full: &'static [u8],
    small: &'static [u8],
}

macro_rules! drawing {
    ($kind:literal) => {
        Drawing {
            kind: $kind,
            full: include_bytes!(concat!("../../../design/assets/devices/", $kind, ".svg")),
            small: include_bytes!(concat!("../../../design/assets/devices/", $kind, "-small.svg")),
        }
    };
}

const DRAWINGS: [Drawing; 10] = [
    drawing!("earbuds"),
    drawing!("headphones"),
    drawing!("speaker"),
    drawing!("gamepad"),
    drawing!("mouse"),
    drawing!("keyboard"),
    drawing!("pen"),
    drawing!("watch"),
    drawing!("phone"),
    drawing!("bluetooth"),
];

/// Every kind with a drawing. `bluetooth` is the fallback for a device of unknown kind.
pub const KINDS: [&str; 10] = {
    let mut kinds = [""; 10];
    let mut i = 0;
    while i < kinds.len() {
        kinds[i] = DRAWINGS[i].kind;
        i += 1;
    }
    kinds
};

/// The SVG for `kind` (exact id), simplified for the compact pill when `small`; `None`
/// for a kind without a drawing.
pub fn svg(kind: &str, small: bool) -> Option<&'static [u8]> {
    let d = DRAWINGS.iter().find(|d| d.kind == kind)?;
    Some(if small { d.small } else { d.full })
}
