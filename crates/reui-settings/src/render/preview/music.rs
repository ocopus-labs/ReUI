//! Now Playing: the compact pill (artwork, bars) and the open card (artwork tile, eyebrow,
//! title, artist, the timeline with its times, transport).

use reui_design::tokens::{color, component, island, prefs::section, radius};
use reui_design::Color;
use windows::core::Result;

use super::{art, bars, bars_width, compact_width, header_text, level, pill, Moment, BLACK};
use crate::render::Align;
use crate::ui::layout::{IslandFont, Rect};

const ACCENT: Color = section::NOW_PLAYING;
const ART_TOP: Color = Color { r: 1.0, g: 0.62, b: 0.35, a: 1.0 };
const ART_BOTTOM: Color = Color { r: 0.75, g: 0.16, b: 0.45, a: 1.0 };
const LEVELS: [f32; 4] = [0.55, 1.0, 0.7, 0.4];
const WIDTH: f32 = 420.0;
const HEIGHT: f32 = 192.0;

pub fn moments() -> Vec<Moment> {
    vec![Moment { w: compact(), draw: draw_compact }, Moment { w: WIDTH, draw: draw_card }]
}

fn compact() -> f32 {
    compact_width(component::art::COMPACT + island::compact::SIDE_MARGIN, bars_width(4) + island::compact::TEXT_MARGIN)
}

fn draw_compact(p: &crate::render::Painter, e: f32) -> Result<()> {
    let w = compact();
    let h = island::HEIGHT;
    pill(p, Rect::at(0.0, 0.0, w, h))?;
    let a = component::art::COMPACT;
    let m = island::compact::SIDE_MARGIN;
    art(p, Rect::at(m, (h - a) / 2.0, a, a), a / 2.0, ART_TOP, ART_BOTTOM)?;
    bars(p, w - island::compact::TEXT_MARGIN - bars_width(4), h / 2.0, &LEVELS, ACCENT, e)
}

fn draw_card(p: &crate::render::Painter, e: f32) -> Result<()> {
    use island::expanded as ex;
    p.fill(Rect::at(0.0, 0.0, WIDTH, HEIGHT), ex::RADIUS, BLACK)?;
    let inset = ex::INSET;
    let a = component::art::EXPANDED;
    art(p, Rect::at(inset, inset, a, a), radius::CONCENTRIC, ART_TOP, ART_BOTTOM)?;
    let x = inset + a + ex::COLUMN_GAP;
    let trailing = bars_width(4);
    header_text(p, x, WIDTH - x - inset - trailing - ex::COLUMN_GAP, "Now Playing", "Midnight Drive", "Aurora Lane", ACCENT)?;
    bars(p, WIDTH - inset - trailing, inset + a / 2.0, &LEVELS, ACCENT, e)?;
    // Timeline and its times.
    let t = inset + a + ex::SECTION_GAP;
    let bar_h = component::progress::HEIGHT;
    level(p, Rect::at(inset, t, WIDTH - 2.0 * inset, bar_h), 0.38 * (0.6 + 0.4 * e), ACCENT)?;
    let cap = t + bar_h + 4.0 + 8.0;
    let tertiary = color::label::TERTIARY;
    super::figures(p, "1:32", IslandFont::Caption, inset, cap, 80.0, Align::Start, tertiary)?;
    super::figures(p, "-2:31", IslandFont::Caption, WIDTH - inset - 80.0, cap, 80.0, Align::End, tertiary)?;
    // Transport: previous and next bare, play / pause in a white circle.
    let mid = cap + 8.0 + ex::ROW_GAP + component::button::TRANSPORT / 2.0 - 4.0;
    let size = component::button::TRANSPORT;
    let cx = WIDTH / 2.0;
    p.circle(cx, mid, size / 2.0, color::label::PRIMARY)?;
    transport(p, cx, mid)?;
    Ok(())
}

/// Previous, pause (in the white circle) and next, drawn as shapes: the glyph font's
/// versions are outlines, the island's are solid.
fn transport(p: &crate::render::Painter, cx: f32, mid: f32) -> Result<()> {
    let (bar_w, bar_h, gap) = (4.5, 15.0, 4.5);
    for dx in [-(gap / 2.0 + bar_w), gap / 2.0] {
        p.fill(Rect::at(cx + dx, mid - bar_h / 2.0, bar_w, bar_h), 1.5, BLACK)?;
    }
    let step = component::button::TRANSPORT + component::button::TRANSPORT_GAP;
    let (tw, th) = (11.0, 13.0);
    for dir in [-1.0f32, 1.0] {
        let x = cx + dir * step;
        // The triangle points outwards; a bar stands at its tip.
        let (base, tip) = (x - dir * tw / 2.0 + dir * 1.5, x + dir * tw / 2.0 + dir * 1.5);
        p.polygon(&[(base, mid - th / 2.0), (tip, mid), (base, mid + th / 2.0)], color::label::PRIMARY)?;
        let bar_x = if dir > 0.0 { tip } else { tip - 2.5 };
        p.fill(Rect::at(bar_x, mid - th / 2.0, 2.5, th), 1.0, color::label::PRIMARY)?;
    }
    Ok(())
}
