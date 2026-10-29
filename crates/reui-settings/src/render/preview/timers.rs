//! Timers (and Appearance): a running timer's card (`reui-hub/src/modules/timers/view.rs`,
//! `running`): the ring of time left, the label → the time left → when it ends, and pause
//! and cancel inline at the right.

use reui_design::look::Look;
use reui_design::tokens::{component, typography};
use reui_design::Color;
use windows::core::Result;

use super::parts::{ring, Card, Hero, Role};
use super::rgb;
use crate::render::Painter;
use crate::ui::stage::{self, text_height};
use crate::ui::Moment;

/// The module's orange.
pub const ACCENT: Color = rgb(255, 159, 10);
/// Share of the timer still to run.
pub const LEFT: f32 = 0.74;
const PAUSE: char = '\u{E769}';
const CANCEL: char = '\u{E711}';

pub fn draw(p: &Painter, look: &Look, e: f32) -> Result<()> {
    let (width, h) = stage::size(Moment::Timer, look);
    let c = Card { p, look, accent: ACCENT, width, radius: look.radius(), roomy: false };
    c.shape(h)?;
    let inset = look.inset();
    let lead = component::ring::EXPANDED;
    let (b, gap) = (component::button::INLINE, component::button::INLINE_GAP);
    let trail = 2.0 * b + gap;
    let (left, text_w) = c.columns(lead, trail);
    let text_h = text_height(true, typography::VALUE.line_height, true);
    let mid = inset + lead.max(b) / 2.0;
    let text_top = if text_h <= lead { mid - text_h / 2.0 } else { inset };
    ring(p, look, inset + lead / 2.0, mid, lead, component::ring::STROKE_EXPANDED, LEFT * e, ACCENT)?;
    let x = width - inset - trail;
    c.glyph_button(PAUSE, Role::Accent, x, mid - b / 2.0, b)?;
    c.glyph_button(CANCEL, Role::Secondary, x + b + gap, mid - b / 2.0, b)?;
    c.text(left, text_top, text_w, "Tea", Hero::Value("3:42"), "Ends 3:46 PM")
}
