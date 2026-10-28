//! Volume & brightness: the compact level pills (glyph, level bar), as the keys show them.

use reui_design::tokens::{color::system, component, icon, island};
use reui_design::Color;
use windows::core::Result;

use super::{compact_width, level, pill, Moment};
use crate::render::Painter;
use crate::ui::icons;
use crate::ui::layout::Rect;

fn width() -> f32 {
    compact_width(icon::COMPACT + island::compact::TEXT_MARGIN, component::level::WIDTH + island::compact::TEXT_MARGIN)
}

pub fn moments() -> Vec<Moment> {
    vec![Moment { w: width(), draw: |p, e| draw(p, icons::VOLUME, 0.64 * e, system::ORANGE) }, Moment { w: width(), draw: |p, e| draw(p, icons::BRIGHTNESS, 0.8 * e, system::YELLOW) }]
}

fn draw(p: &Painter, glyph: char, value: f32, accent: Color) -> Result<()> {
    let (w, h) = (width(), island::HEIGHT);
    pill(p, Rect::at(0.0, 0.0, w, h))?;
    let m = island::compact::TEXT_MARGIN;
    let g = icon::COMPACT;
    p.glyph(glyph, g, Rect::at(m, (h - g) / 2.0, g, g), accent)?;
    let lw = component::level::WIDTH;
    let lh = component::level::HEIGHT;
    level(p, Rect::at(w - m - lw, (h - lh) / 2.0, lw, lh), value, accent)
}
