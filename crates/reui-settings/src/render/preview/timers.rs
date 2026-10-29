//! Timers (and Appearance): a running timer's card (`reui-hub/src/modules/timers/view.rs`,
//! `running`): the ring of time left, the label → the time left → when it ends, and pause
//! and cancel inline at the right.

use reui_design::layout::header;
use reui_design::look::Look;
use reui_design::tokens::component;
use reui_design::Color;
use windows::core::Result;

use super::parts::{ring, Card, Hero, Role};
use super::rgb;
use crate::render::Painter;
use crate::ui::stage;
use crate::ui::Moment;

/// The module's orange.
pub const ACCENT: Color = rgb(255, 159, 10);
/// Share of the timer still to run.
pub const LEFT: f32 = 0.74;
const PAUSE: char = '\u{E769}';
const CANCEL: char = '\u{E711}';

pub fn draw(p: &Painter, look: &Look, e: f32) -> Result<()> {
    let open = stage::card(Moment::Timer, look).expect("an open card");
    let c = Card::new(p, look, ACCENT, open.card);
    let width = c.width;
    c.shape(open.height)?;
    let inset = look.inset();
    let lead = component::ring::EXPANDED;
    let b = component::button::INLINE;
    let (left, text_w) = c.columns(lead, header::inline_width(2));
    let (mid, text_top) = (open.header.mid, open.header.text_top);
    ring(p, look, inset + lead / 2.0, mid, lead, component::ring::STROKE_EXPANDED, LEFT * e, ACCENT)?;
    c.glyph_button(PAUSE, Role::Accent, header::inline_x(look, width, 2, 0), mid - b / 2.0, b)?;
    c.glyph_button(CANCEL, Role::Secondary, header::inline_x(look, width, 2, 1), mid - b / 2.0, b)?;
    c.text(&open.text, left, text_top, text_w, "Tea", Hero::Value("3:42"), "Ends 3:46 PM")
}
