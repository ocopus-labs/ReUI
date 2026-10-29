//! The compact pill and the minimal bubble. Positions are from the pill's left edge; the
//! island centres the pill in its surface ([`left`]), a preview draws it at 0.

use crate::look::{concentric, Look};
use crate::tokens::{component, island, notch};

/// Level bars in a meter.
pub const BARS: usize = 4;

/// Widest compact presentation: two maximal sides around the centre gap.
pub const MAX_WIDTH: f32 = island::compact::CENTER_GAP + 2.0 * (island::compact::MAX_SIDE + island::compact::TEXT_MARGIN);
/// The compact surface: the widest pill, as tall as the island.
pub const SURFACE: (f32, f32) = (MAX_WIDTH, island::HEIGHT);
/// The minimal bubble's surface.
pub const MINIMAL_SURFACE: (f32, f32) = (island::minimal::SIZE, island::minimal::SIZE);

/// Width of the level bars.
pub fn bars_width() -> f32 {
    BARS as f32 * component::bars::WIDTH + (BARS as f32 - 1.0) * component::bars::GAP
}

/// A side's margin from the pill's end: round content (art, a ring) sits concentric in
/// the capsule's end; text and level bars keep clear of the curve.
pub fn margin(round: bool) -> f32 {
    if round {
        island::compact::SIDE_MARGIN
    } else {
        island::compact::TEXT_MARGIN
    }
}

/// Where a pill `width` wide starts in the compact surface (centred).
pub fn left(width: f32) -> f32 {
    (SURFACE.0 - width) / 2.0
}

/// Two sides around the centre gap.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sides {
    pub width: f32,
    /// The leading element's left edge.
    pub leading: f32,
    /// The trailing element's right edge.
    pub trailing: f32,
}

/// Both sides get the same width (the wider one's, margin included), as in Apple's compact
/// layout, so the pill stays centred on the gap. `leading`/`trailing`: each element's
/// width; `*_margin`: its [`margin`].
pub fn sides(leading: f32, leading_margin: f32, trailing: f32, trailing_margin: f32) -> Sides {
    let side = (leading + leading_margin).max(trailing + trailing_margin);
    let width = island::compact::CENTER_GAP + 2.0 * side;
    Sides { width, leading: leading_margin, trailing: width - trailing_margin }
}

/// A text label beside its mark.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Joined {
    pub width: f32,
    /// The text's left edge and the width it's drawn in.
    pub text_x: f32,
    pub text_w: f32,
}

/// Windows has no camera cutout to keep clear: a label reads with its mark, side by side,
/// and the pill is only as wide as the two (never narrower than the resting pill). `mark`:
/// the mark's width, `margin` its [`margin`]; `text`: the label's measured width.
pub fn joined(mark: f32, margin: f32, text: f32) -> Joined {
    let text_w = text.min(island::compact::JOINED_TEXT);
    let gap = island::compact::JOINED_GAP;
    let width = (margin + mark + gap + text_w + island::compact::TEXT_MARGIN).max(island::idle::WIDTH);
    Joined { width, text_x: margin + mark + gap, text_w }
}

/// A level (volume, brightness) as the whole pill: the pill is the track, an accent fill
/// grows from the left, inset so its round ends are concentric with the pill's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Meter {
    pub width: f32,
    /// The fill's distance from the pill's top, bottom and left.
    pub inset: f32,
    /// The fill's length from its left edge; `None` at zero (no fill).
    pub fill: Option<f32>,
    pub corner: f32,
    /// The glyph's centre: in the fill's left end.
    pub glyph_x: f32,
    /// The percentage's right edge.
    pub percent_right: f32,
}

pub fn meter(look: &Look, value: f32) -> Meter {
    let width = island::compact::METER_WIDTH;
    let h = island::HEIGHT;
    let inset = island::compact::METER_INSET;
    let inner = h - 2.0 * inset;
    let value = value.clamp(0.0, 1.0);
    // Anything above zero shows at least the round end, so "on but quiet" never reads as off.
    let fill = (value > 0.0).then(|| ((width - 2.0 * inset) * value).max(inner));
    let corner = concentric(look.rounded(h), inset).min(inner / 2.0);
    Meter { width, inset, fill, corner, glyph_x: h / 2.0, percent_right: width - island::compact::TEXT_MARGIN }
}

/// Corner radius of artwork in the pill: concentric with the pill's end, at most a circle.
pub fn art_corner(look: &Look) -> f32 {
    let size = component::art::COMPACT;
    concentric(look.rounded(island::HEIGHT), island::compact::SIDE_MARGIN).min(size / 2.0)
}

/// Between the pill and the bubble beside it: a gap, or in a notch the narrow joint that
/// makes the bubble the notch's other end.
pub fn bubble_gap(look: &Look) -> f32 {
    if look.notch() {
        notch::JOIN_GAP
    } else {
        island::minimal::GAP
    }
}
