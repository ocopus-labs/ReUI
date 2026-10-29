//! Devices: earbuds just connected (`reui-hub/src/modules/devices/view.rs`, `connect_view`):
//! the earbuds' glyph tile, "Connected" → the name → the battery, and the battery's ring
//! at the right. Its accent is the look's (red only when the battery is low).

use reui_design::look::Look;
use reui_design::tokens::component;
use windows::core::Result;

use super::parts::{glyph_tile, ring, Card, Hero};
use crate::render::Painter;
use crate::ui::stage;
use crate::ui::Moment;

const EARBUDS: char = '\u{F4C0}';
const BATTERY: f32 = 0.82;

pub fn draw(p: &Painter, look: &Look, e: f32) -> Result<()> {
    let open = stage::card(Moment::Devices, look).expect("an open card");
    let c = Card::new(p, look, look.accent, open.card);
    let width = c.width;
    c.shape(open.height)?;
    let inset = look.inset();
    let (lead, trail) = (component::art::EXPANDED, component::ring::EXPANDED);
    let (left, text_w) = c.columns(lead, trail);
    let (mid, text_top) = (open.header.mid, open.header.text_top);
    glyph_tile(p, look, c.radius, false, EARBUDS, inset, mid - lead / 2.0, look.accent)?;
    ring(p, look, width - inset - trail / 2.0, mid, trail, component::ring::STROKE_EXPANDED, BATTERY * e, look.accent)?;
    c.text(&open.text, left, text_top, text_w, "Connected", Hero::Text("Wireless Earbuds"), &format!("{}%", (BATTERY * 100.0).round()))
}
