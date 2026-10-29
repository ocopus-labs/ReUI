//! Devices: earbuds just connected (`reui-hub/src/modules/devices/view.rs`, `connect_view`):
//! the earbuds' glyph tile, "Connected" → the name → the battery, and the battery's ring
//! at the right. Its accent is the look's (red only when the battery is low).

use reui_design::look::Look;
use reui_design::tokens::{component, typography};
use windows::core::Result;

use super::parts::{glyph_tile, ring, Card, Hero};
use crate::render::Painter;
use crate::ui::stage::{self, text_height};
use crate::ui::Moment;

const EARBUDS: char = '\u{F4C0}';
const BATTERY: f32 = 0.82;

pub fn draw(p: &Painter, look: &Look, e: f32) -> Result<()> {
    let (width, h) = stage::size(Moment::Devices, look);
    let c = Card { p, look, accent: look.accent, width, radius: look.radius(), roomy: false };
    c.shape(h)?;
    let inset = look.inset();
    let (lead, trail) = (component::art::EXPANDED, component::ring::EXPANDED);
    let (left, text_w) = c.columns(lead, trail);
    let text_h = text_height(true, typography::HEADLINE.line_height, true);
    let mid = inset + lead.max(trail) / 2.0;
    let text_top = if text_h <= lead { mid - text_h / 2.0 } else { inset };
    glyph_tile(p, look, c.radius, false, EARBUDS, inset, mid - lead / 2.0, look.accent)?;
    ring(p, look, width - inset - trail / 2.0, mid, trail, component::ring::STROKE_EXPANDED, BATTERY * e, look.accent)?;
    c.text(left, text_top, text_w, "Connected", Hero::Text("Wireless Earbuds"), &format!("{}%", (BATTERY * 100.0).round()))
}
