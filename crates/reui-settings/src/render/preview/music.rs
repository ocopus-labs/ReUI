//! Now Playing: the open card as the media module sends it for a track without artwork
//! (`reui-hub/src/modules/media/card.rs`): the glyph tile, app → title → artist, the bars,
//! the timeline with its times at the transport's ends, then the volume.

use reui_design::layout::{compact, row_gap, rows};
use reui_design::look::Look;
use reui_design::tokens::component;
use reui_design::Color;
use windows::core::Result;

use super::parts::{bars, glyph_tile, Card, Hero, Role};
use super::rgb;
use crate::render::Painter;
use crate::ui::stage;
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
    let open = stage::card(Moment::Music, look).expect("an open card");
    let c = Card::new(p, look, ACCENT, open.card);
    let width = c.width;
    c.shape(open.height)?;
    let inset = look.inset();
    let (lead, trail) = (component::art::EXPANDED, compact::bars_width());
    let (left, text_w) = c.columns(lead, trail);
    // The tile one inset from the top, the bars on its centre line; text centred on it
    // when it fits beside it, else from the inset.
    let (mid, text_top) = (open.header.mid, open.header.text_top);
    glyph_tile(p, look, c.radius, false, GLYPH, inset, mid - lead / 2.0, ACCENT)?;
    bars(p, width - inset - trail, mid, ACCENT, e)?;
    c.text(&open.text, left, text_top, text_w, "Media Player", Hero::Text("Clair de Lune"), "Claude Debussy")?;
    // The timeline, then the transport between its times.
    let mut y = open.rows;
    c.meter(inset, width - inset, y, 92.0 / 303.0)?;
    y += component::progress::HEIGHT + rows::UNDER_BAR;
    let row = c.transport(&[(PREVIOUS, Role::Plain), (PAUSE, Role::Primary), (NEXT, Role::Plain)], y)?;
    c.captions("1:32", "-3:31", y + row / 2.0)?;
    y += row + row_gap(look, 1, false);
    c.slider(VOLUME, 0.6, OUTPUT, y)
}
