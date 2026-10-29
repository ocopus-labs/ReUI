//! Volume & brightness: the volume key's compact meter (`reui-island/src/render/compact.rs`,
//! `level_pill`): the whole pill is the meter, an accent fill from the left inset so its
//! round ends stay concentric with the pill's, the glyph in the fill's left end and the
//! percentage at the right. The OSD sends no accent: the look's own.

use reui_design::layout::compact;
use reui_design::look::Look;
use reui_design::tokens::{icon, island, opacity, typography};
use windows::core::Result;

use super::parts::{glyph, line, shape};
use crate::render::{Align, Painter};
use crate::ui::layout::{IslandFont, Rect};

const LEVEL: f32 = 0.56;
/// The speaker for a level between a third and two thirds.
const GLYPH: char = '\u{E994}';

pub fn draw(p: &Painter, look: &Look) -> Result<()> {
    let m = compact::meter(look, LEVEL);
    let (w, h) = (m.width, island::HEIGHT);
    shape(p, look, Rect::at(0.0, 0.0, w, h), look.rounded(h), (true, true))?;
    if let Some(fill) = m.fill {
        p.fill(Rect::new(m.inset, m.inset, m.inset + fill, h - m.inset), m.corner, look.accent.with_alpha(opacity::METER_FILL))?;
    }
    let g = icon::COMPACT;
    glyph(p, GLYPH, g, m.glyph_x - g / 2.0, (h - g) / 2.0, look.palette.label_primary)?;
    let lh = typography::COMPACT.line_height;
    line(p, &format!("{}%", (LEVEL * 100.0).round()), IslandFont::Compact, m.percent_right - 60.0, (h - lh) / 2.0, 60.0, Align::End, look.palette.label_primary)
}
