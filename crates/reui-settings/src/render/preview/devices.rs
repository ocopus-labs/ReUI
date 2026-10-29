//! Devices: earbuds just connected (`reui-hub/src/modules/devices/view.rs`, `connect_view`,
//! with the `Mark` `peripherals.rs` makes): ReUI's earbuds drawing bare in the leading
//! spot, "Connected" → the name → the battery, and the battery's ring at the right. Its
//! accent is the look's (red only when the battery is low).

use reui_design::devices;
use reui_design::look::Look;
use reui_design::tokens::{component, opacity};
use windows::core::Result;

use super::parts::{ring, Card, Hero};
use crate::render::Painter;
use crate::ui::layout::Rect;
use crate::ui::stage;
use crate::ui::Moment;

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
    // The full drawing as the island draws an `Art::Logo` in the card's spot
    // (`render/images.rs`): bare, whole DIPs of `opacity.logo-bare`, in the label ink.
    let earbuds = std::str::from_utf8(devices::svg("earbuds", false).expect("the earbuds drawing")).expect("UTF-8");
    let mark = (lead * opacity::LOGO_BARE).round();
    let top = mid - lead / 2.0;
    p.line_art(earbuds, Rect::at(inset + (lead - mark) / 2.0, top + (lead - mark) / 2.0, mark, mark), look.palette.label_primary)?;
    ring(p, look, width - inset - trail / 2.0, mid, trail, component::ring::STROKE_EXPANDED, BATTERY * e, look.accent)?;
    c.text(&open.text, left, text_top, text_w, "Connected", Hero::Text("Wireless Earbuds"), &format!("{}%", (BATTERY * 100.0).round()))
}
