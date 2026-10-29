//! Volume & brightness: the volume key's compact meter (`reui-island/src/render/compact.rs`,
//! `level_pill`): the whole pill is the meter, an accent fill from the left inset so its
//! round ends stay concentric with the pill's, the glyph in the fill's left end and the
//! percentage at the right. The OSD sends no accent: the look's own.

use reui_design::look::{concentric, Look};
use reui_design::tokens::{icon, island, opacity, typography};
use windows::core::Result;

use super::parts::{glyph, line, shape};
use crate::render::{Align, Painter};
use crate::ui::layout::{IslandFont, Rect};

const LEVEL: f32 = 0.56;
/// The speaker for a level between a third and two thirds.
const GLYPH: char = '\u{E994}';

pub fn draw(p: &Painter, look: &Look) -> Result<()> {
    let (w, h) = (island::compact::METER_WIDTH, island::HEIGHT);
    shape(p, look, Rect::at(0.0, 0.0, w, h), look.rounded(h), (true, true))?;
    let inset = island::compact::METER_INSET;
    let inner = h - 2.0 * inset;
    let right = inset + ((w - 2.0 * inset) * LEVEL).max(inner);
    let corner = concentric(look.rounded(h), inset).min(inner / 2.0);
    p.fill(Rect::new(inset, inset, right, h - inset), corner, look.accent.with_alpha(opacity::METER_FILL))?;
    let g = icon::COMPACT;
    glyph(p, GLYPH, g, h / 2.0 - g / 2.0, (h - g) / 2.0, look.palette.label_primary)?;
    let lh = typography::COMPACT.line_height;
    let m = island::compact::TEXT_MARGIN;
    line(p, &format!("{}%", (LEVEL * 100.0).round()), IslandFont::Compact, w - m - 60.0, (h - lh) / 2.0, 60.0, Align::End, look.palette.label_primary)
}
