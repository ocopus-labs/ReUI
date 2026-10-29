//! Now Playing: the open card as the media module sends it for a track without artwork
//! (`reui-hub/src/modules/media/card.rs`): the glyph tile, app → title → artist, the bars,
//! the timeline with its times at the transport's ends, then the volume.

use reui_design::look::Look;
use reui_design::tokens::{component, typography};
use reui_design::Color;
use windows::core::Result;

use super::parts::{bars, glyph_tile, Card, Hero, Role, SM};
use super::rgb;
use crate::render::Painter;
use crate::ui::stage::{self, text_height};
use crate::ui::Moment;

/// The module's own accent while there's no artwork to take one from (`DEFAULT_ACCENT`).
pub const ACCENT: Color = rgb(255, 55, 95);
/// Artwork's stand-in (`GLYPH_MUSIC`).
pub const GLYPH: char = '\u{E8D6}';
const PREVIOUS: char = '\u{E892}';
const PAUSE: char = '\u{E769}';
const NEXT: char = '\u{E893}';
const VOLUME: char = '\u{E767}';
const OUTPUT: char = '\u{E7F3}';

pub fn draw(p: &Painter, look: &Look, e: f32) -> Result<()> {
    let (width, h) = stage::size(Moment::Music, look);
    let c = Card { p, look, accent: ACCENT, width, radius: look.radius(), roomy: false };
    c.shape(h)?;
    let inset = look.inset();
    let (lead, trail) = (component::art::EXPANDED, stage::bars_width());
    let (left, text_w) = c.columns(lead, trail);
    let text_h = text_height(true, typography::HEADLINE.line_height, true);
    // The tile one inset from the top, the bars on its centre line; text centred on it
    // when it fits beside it, else from the inset.
    let mid = inset + lead.max(component::bars::HEIGHT) / 2.0;
    let text_top = if text_h <= lead { mid - text_h / 2.0 } else { inset };
    glyph_tile(p, look, c.radius, false, GLYPH, inset, mid - lead / 2.0, ACCENT)?;
    bars(p, width - inset - trail, mid, ACCENT, e)?;
    c.text(left, text_top, text_w, "Media Player", Hero::Text("Clair de Lune"), "Claude Debussy")?;
    // The timeline, then the transport between its times.
    let mut y = inset + lead.max(text_h) + look.section_gap();
    c.meter(inset, width - inset, y, 92.0 / 303.0)?;
    y += component::progress::HEIGHT + SM;
    let row = c.transport(&[(PREVIOUS, Role::Plain), (PAUSE, Role::Primary), (NEXT, Role::Plain)], y)?;
    c.captions("1:32", "-3:31", y + row / 2.0)?;
    y += row + look.row_gap();
    c.slider(VOLUME, 0.6, OUTPUT, y)
}
