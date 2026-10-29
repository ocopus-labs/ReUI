//! Volume & brightness: the compact meter pills (the pill as the track, the fill, the glyph
//! and the percentage), as the keys show them.

use reui_design::tokens::{color::{label, system}, icon, island, opacity};
use reui_design::Color;
use windows::core::Result;

use super::{figures, pill, Moment};
use crate::render::{Align, Painter};
use crate::ui::icons;
use crate::ui::layout::{IslandFont, Rect};

pub fn moments() -> Vec<Moment> {
    let w = island::compact::METER_WIDTH;
    vec![Moment { w, draw: |p, e| draw(p, icons::VOLUME, 0.64 * e, system::ORANGE) }, Moment { w, draw: |p, e| draw(p, icons::BRIGHTNESS, 0.8 * e, system::YELLOW) }]
}

fn draw(p: &Painter, glyph: char, value: f32, accent: Color) -> Result<()> {
    let (w, h) = (island::compact::METER_WIDTH, island::HEIGHT);
    pill(p, Rect::at(0.0, 0.0, w, h))?;
    let inset = island::compact::METER_INSET;
    let inner = h - 2.0 * inset;
    p.fill(Rect::at(inset, inset, ((w - 2.0 * inset) * value).max(inner), inner), inner / 2.0, accent.with_alpha(opacity::METER_FILL))?;
    let g = icon::COMPACT;
    p.glyph(glyph, g, Rect::at((h - g) / 2.0, (h - g) / 2.0, g, g), label::PRIMARY)?;
    let m = island::compact::TEXT_MARGIN;
    figures(p, &format!("{}%", (value * 100.0).round() as u32), IslandFont::Compact, w - m - 60.0, h / 2.0, 60.0, Align::End, label::PRIMARY)
}
