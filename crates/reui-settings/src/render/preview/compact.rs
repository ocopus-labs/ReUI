//! General: the island as it sits most of the time, a compact pill with a second activity
//! beside it. Music playing (no artwork, so its glyph tile; the bars at rest) and a timer
//! counting down in the bubble, or in a notch's trailing end.

use reui_design::look::Look;
use reui_design::tokens::{component, island, notch};
use windows::core::Result;

use super::parts::{bars, glyph_tile, ring, shape};
use super::{music, timers};
use crate::render::Painter;
use crate::ui::layout::Rect;
use crate::ui::stage;

pub fn draw(p: &Painter, look: &Look, e: f32) -> Result<()> {
    let (w, h) = (stage::music_pill(), island::HEIGHT);
    shape(p, look, Rect::at(0.0, 0.0, w, h), look.rounded(h), (true, false))?;
    let a = component::art::COMPACT;
    glyph_tile(p, look, look.radius(), true, music::GLYPH, island::compact::SIDE_MARGIN, (h - a) / 2.0, music::ACCENT)?;
    bars(p, w - island::compact::TEXT_MARGIN - stage::bars_width(), h / 2.0, music::ACCENT, e)?;
    // The second activity: detached beside the pill, or the notch's end joined to it.
    let size = island::minimal::SIZE;
    let x = w + if look.notch() { notch::JOIN_GAP } else { island::minimal::GAP };
    shape(p, look, Rect::at(x, 0.0, size, size), look.rounded(size), (false, true))?;
    ring(p, look, x + size / 2.0, size / 2.0, component::ring::COMPACT, component::ring::STROKE_COMPACT, timers::LEFT * e, timers::ACCENT)
}
