//! Timers: the counting pill (the ring of time left, the numerals) and the open card (the
//! ring, the label, the time left, when it ends, and pause and cancel beside it).

use reui_design::tokens::{color, component, island, opacity};
use reui_design::Color;
use windows::core::Result;

use super::{compact_width, figures, header_text, pill, ring, Moment, BLACK};
use crate::render::{Align, Painter};
use crate::ui::layout::{IslandFont, Rect};

/// The island's timer orange (the module's accent), not the page's tile colour.
const ACCENT: Color = Color { r: 1.0, g: 159.0 / 255.0, b: 10.0 / 255.0, a: 1.0 };
const WIDTH: f32 = 400.0;
const LEFT: &str = "3:42";
const FRACTION: f32 = 0.74;
const CANCEL: char = '\u{E711}';

fn compact() -> f32 {
    compact_width(component::ring::COMPACT + island::compact::SIDE_MARGIN, 38.0 + island::compact::TEXT_MARGIN)
}

fn height() -> f32 {
    2.0 * island::expanded::INSET + component::art::EXPANDED
}

pub fn moments() -> Vec<Moment> {
    vec![Moment { w: compact(), draw: draw_compact }, Moment { w: WIDTH, draw: draw_card }]
}

fn draw_compact(p: &Painter, e: f32) -> Result<()> {
    let (w, h) = (compact(), island::HEIGHT);
    pill(p, Rect::at(0.0, 0.0, w, h))?;
    let size = component::ring::COMPACT;
    let m = island::compact::SIDE_MARGIN;
    ring(p, m + size / 2.0, h / 2.0, size, component::ring::STROKE_COMPACT, FRACTION * e, ACCENT)?;
    let tm = island::compact::TEXT_MARGIN;
    figures(p, LEFT, IslandFont::Compact, w - tm - 60.0, h / 2.0, 60.0, Align::End, ACCENT)
}

fn draw_card(p: &Painter, e: f32) -> Result<()> {
    use island::expanded as ex;
    p.fill(Rect::at(0.0, 0.0, WIDTH, height()), ex::RADIUS, BLACK)?;
    let a = component::art::EXPANDED;
    let mid = ex::INSET + a / 2.0;
    ring(p, ex::INSET + a / 2.0, mid, component::ring::EXPANDED, component::ring::STROKE_EXPANDED, FRACTION * e, ACCENT)?;
    // Pause (tinted with the accent) and cancel (grey), inline at the right.
    let b = component::button::INLINE;
    let cancel_x = WIDTH - ex::INSET - b;
    let pause_x = cancel_x - component::button::INLINE_GAP - b;
    p.fill(Rect::at(pause_x, mid - b / 2.0, b, b), b / 2.0, ACCENT.with_alpha(opacity::TINT_FILL))?;
    let (bar_w, bar_h, gap) = (4.0, 14.0, 4.0);
    for dx in [-(gap / 2.0 + bar_w), gap / 2.0] {
        p.fill(Rect::at(pause_x + b / 2.0 + dx, mid - bar_h / 2.0, bar_w, bar_h), 1.5, ACCENT)?;
    }
    p.fill(Rect::at(cancel_x, mid - b / 2.0, b, b), b / 2.0, color::fill::TERTIARY)?;
    p.glyph(CANCEL, 14.0, Rect::at(cancel_x, mid - b / 2.0, b, b), color::label::PRIMARY)?;
    let x = ex::INSET + a + ex::COLUMN_GAP;
    header_text(p, x, pause_x - ex::COLUMN_GAP - x, "Tea", LEFT, "Ends 3:46 PM", ACCENT)
}
