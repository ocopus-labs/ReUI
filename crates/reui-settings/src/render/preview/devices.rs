//! Devices: the connect pill (the earbuds on a tinted circle, the battery) and the open
//! card (tile, eyebrow, name, then each part's battery as a ring with its figure).

use reui_design::tokens::{color, component, icon, island, prefs::section, radius};
use reui_design::Color;
use windows::core::Result;

use super::{compact_width, figures, header_text, pill, ring, text, tinted, Moment, BLACK};
use crate::render::{Align, Painter};
use crate::ui::icons;
use crate::ui::layout::{IslandFont, Rect};

const ACCENT: Color = section::DEVICES;
const WIDTH: f32 = 420.0;
const READING: &str = "82 %";
const PARTS: [(&str, f32); 3] = [("Left", 0.82), ("Right", 0.8), ("Case", 0.64)];
/// The part rings: smaller than a hero ring, with their figure inside.
const RING: f32 = 44.0;
const RING_STROKE: f32 = 4.5;

fn compact() -> f32 {
    compact_width(component::art::COMPACT + island::compact::SIDE_MARGIN, 38.0 + island::compact::TEXT_MARGIN)
}

fn height() -> f32 {
    use island::expanded as ex;
    2.0 * ex::INSET + component::art::EXPANDED + ex::SECTION_GAP + RING + 4.0 + reui_design::tokens::typography::CAPTION.line_height
}

pub fn moments() -> Vec<Moment> {
    vec![Moment { w: compact(), draw: draw_compact }, Moment { w: WIDTH, draw: draw_card }]
}

fn draw_compact(p: &Painter, _e: f32) -> Result<()> {
    let (w, h) = (compact(), island::HEIGHT);
    pill(p, Rect::at(0.0, 0.0, w, h))?;
    let a = component::art::COMPACT;
    let m = island::compact::SIDE_MARGIN;
    tinted(p, Rect::at(m, (h - a) / 2.0, a, a), a / 2.0, icons::HEADPHONES, icon::TILE_COMPACT, ACCENT)?;
    let tm = island::compact::TEXT_MARGIN;
    figures(p, READING, IslandFont::Compact, w - tm - 60.0, h / 2.0, 60.0, Align::End, ACCENT)
}

fn draw_card(p: &Painter, e: f32) -> Result<()> {
    use island::expanded as ex;
    p.fill(Rect::at(0.0, 0.0, WIDTH, height()), ex::RADIUS, BLACK)?;
    let a = component::art::EXPANDED;
    tinted(p, Rect::at(ex::INSET, ex::INSET, a, a), radius::CONCENTRIC, icons::HEADPHONES, icon::TILE, ACCENT)?;
    let x = ex::INSET + a + ex::COLUMN_GAP;
    header_text(p, x, WIDTH - x - ex::INSET, "Connected", "AirPods Pro", "Bluetooth", ACCENT)?;
    // Each part's battery, evenly across.
    let top = ex::INSET + a + ex::SECTION_GAP;
    let column = (WIDTH - 2.0 * ex::INSET) / PARTS.len() as f32;
    for (i, (name, level)) in PARTS.iter().enumerate() {
        let cx = ex::INSET + column * (i as f32 + 0.5);
        let cy = top + RING / 2.0;
        ring(p, cx, cy, RING, RING_STROKE, level * e, ACCENT)?;
        figures(p, &format!("{}", (level * 100.0).round()), IslandFont::Caption, cx - RING / 2.0, cy, RING, Align::Center, color::label::PRIMARY)?;
        text(p, name, IslandFont::Caption, cx - column / 2.0, top + RING + 4.0 + 8.0, column, Align::Center, color::label::SECONDARY)?;
    }
    Ok(())
}
