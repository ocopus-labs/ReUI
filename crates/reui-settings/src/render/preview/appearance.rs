//! Appearance: the island in the chosen look, drawn from the same `Look` the island uses
//! (palette, roundness, padding, size, gap, style): a compact pill with its second activity
//! (a bubble beside it, or inside a notch's trailing end) and an open timer card. In the
//! Notch style both are joined to the stage's top edge with concave ears.

use reui_design::look::{concentric, Look};
use reui_design::tokens::{component, island, notch, opacity, prefs as s, prefs::section, typography};
use reui_design::Color;
use windows::core::Result;

use super::{art, bars, bars_width, compact_width, ease_out, figures, text};
use crate::render::{Align, Painter};
use crate::ui::layout::{IslandFont, Rect};

const MUSIC: Color = section::NOW_PLAYING;
/// The island's timer orange (the module's accent).
const TIMER: Color = Color { r: 1.0, g: 159.0 / 255.0, b: 10.0 / 255.0, a: 1.0 };
const ART_TOP: Color = Color { r: 1.0, g: 0.62, b: 0.35, a: 1.0 };
const ART_BOTTOM: Color = Color { r: 0.75, g: 0.16, b: 0.45, a: 1.0 };
const LEVELS: [f32; 4] = [0.55, 1.0, 0.7, 0.4];
const CARD_WIDTH: f32 = 400.0;
const CANCEL: char = '\u{E711}';
/// Share of the island's own size the stage shows it at.
const SCALE: f32 = 0.62;
const GAP: f32 = 40.0;

pub fn draw(p: &Painter, look: &Look, stage: Rect, tint: Color, enter: f32) -> Result<()> {
    p.stage(stage, s::group::RADIUS, s::color::STAGE, tint.with_alpha(s::opacity::STAGE_TINT))?;
    p.hairline(stage.l, stage.r, stage.b - p.pixel(), Color { r: 1.0, g: 1.0, b: 1.0, a: 0.04 })?;
    let e = ease_out(enter);
    let k = SCALE * look.size;
    let (pill_w, pair_w) = compact_widths(look);
    let total = (pair_w + CARD_WIDTH) * k + GAP;
    let left = (stage.l + stage.r) / 2.0 - total / 2.0;
    // Emerging from the top edge as the page opens.
    let top = stage.t + look.top() * k - (1.0 - e) * reui_design::tokens::motion::show::RISE;
    p.alpha.set(e.min(1.0));
    let result = (|| {
        p.scaled(p.snap(left), p.snap(top), k, || compact(p, look, pill_w, e))?;
        p.scaled(p.snap(left + pair_w * k + GAP), p.snap(top), k, || card(p, look, e))
    })();
    p.alpha.set(1.0);
    result
}

/// The compact pill's width, and the pill with its second activity.
fn compact_widths(look: &Look) -> (f32, f32) {
    let w = compact_width(component::art::COMPACT + island::compact::SIDE_MARGIN, bars_width(4) + island::compact::TEXT_MARGIN);
    let joint = if look.notch() { notch::JOIN_GAP } else { island::minimal::GAP };
    (w, w + joint + island::minimal::SIZE)
}

/// The island's shape `r`: a rounded rectangle, or in the Notch style one whose top is
/// flush and square, with ears where it meets the edge (`ears`: left, right).
fn shape(p: &Painter, look: &Look, r: Rect, radius: f32, ears: (bool, bool)) -> Result<()> {
    let pal = &look.palette;
    p.fill(r, radius, pal.background)?;
    if !look.notch() {
        return p.edge(r, radius, pal.hairline);
    }
    p.fill(Rect::new(r.l, r.t, r.r, r.t + radius.min(r.h())), 0.0, pal.background)?;
    // The hairline along the sides and bottom only: the top is the screen's edge.
    p.clip(Rect::new(r.l - 1.0, r.t + radius, r.r + 1.0, r.b + 1.0), || p.edge(r, radius, pal.hairline))?;
    let e = notch::EAR;
    if ears.0 {
        ear(p, r.l, r.t, e, -1.0, pal.background)?;
    }
    if ears.1 {
        ear(p, r.r, r.t, e, 1.0, pal.background)?;
    }
    Ok(())
}

/// A concave ear at the top of a notch's side at `x`: the square `e` beside it minus a
/// quarter circle, so the side curves out into the screen's edge. `dir` -1 is the left.
fn ear(p: &Painter, x: f32, top: f32, e: f32, dir: f32, color: Color) -> Result<()> {
    let (cx, cy) = (x + dir * e, top + e);
    let mut points = vec![(x + dir * e, top), (x, top), (x, top + e)];
    const STEPS: usize = 12;
    for i in 0..=STEPS {
        // From the notch's side (angle 0 or 180°) up to the edge (270°).
        let t = i as f32 / STEPS as f32 * std::f32::consts::FRAC_PI_2;
        let (dx, dy) = (-dir * t.cos(), -t.sin());
        points.push((cx + dx * e, cy + dy * e));
    }
    p.polygon(&points, color)
}

fn compact(p: &Painter, look: &Look, w: f32, e: f32) -> Result<()> {
    let h = island::HEIGHT;
    let notch = look.notch();
    shape(p, look, Rect::at(0.0, 0.0, w, h), look.rounded(h), (true, false))?;
    let a = component::art::COMPACT;
    let m = island::compact::SIDE_MARGIN;
    let corner = concentric(look.rounded(h), m).min(a / 2.0);
    art(p, Rect::at(m, (h - a) / 2.0, a, a), corner, ART_TOP, ART_BOTTOM)?;
    bars(p, w - island::compact::TEXT_MARGIN - bars_width(4), h / 2.0, &LEVELS, MUSIC, e)?;
    // The second activity: a bubble beside the pill, or the notch's trailing end.
    let size = island::minimal::SIZE;
    let x = w + if notch { notch::JOIN_GAP } else { island::minimal::GAP };
    shape(p, look, Rect::at(x, 0.0, size, size), look.rounded(size), (false, true))?;
    ring(p, look, x + size / 2.0, size / 2.0, component::ring::COMPACT, component::ring::STROKE_COMPACT, 0.74 * e, TIMER)
}

fn card(p: &Painter, look: &Look, e: f32) -> Result<()> {
    let pal = &look.palette;
    let inset = look.inset();
    let a = component::art::EXPANDED;
    let h = 2.0 * inset + a;
    let radius = look.radius();
    shape(p, look, Rect::at(0.0, 0.0, CARD_WIDTH, h), radius, (true, true))?;
    let mid = inset + a / 2.0;
    ring(p, look, inset + a / 2.0, mid, component::ring::EXPANDED, component::ring::STROKE_EXPANDED, 0.74 * e, TIMER)?;
    // Pause (tinted with the accent) and cancel (grey), inline at the right; capsules only
    // while the concentric radius allows.
    let b = component::button::INLINE;
    let corner = look.capsule(b, radius, inset);
    let cancel_x = CARD_WIDTH - inset - b;
    let pause_x = cancel_x - component::button::INLINE_GAP - b;
    p.fill(Rect::at(pause_x, mid - b / 2.0, b, b), corner, TIMER.with_alpha(opacity::TINT_FILL))?;
    let (bar_w, bar_h, gap) = (4.0, 14.0, 4.0);
    for dx in [-(gap / 2.0 + bar_w), gap / 2.0] {
        p.fill(Rect::at(pause_x + b / 2.0 + dx, mid - bar_h / 2.0, bar_w, bar_h), 1.5, TIMER)?;
    }
    p.fill(Rect::at(cancel_x, mid - b / 2.0, b, b), corner, pal.fill_tertiary)?;
    p.glyph(CANCEL, 14.0, Rect::at(cancel_x, mid - b / 2.0, b, b), pal.label_primary)?;
    // Eyebrow (accent) → hero → detail, beside the ring.
    let x = inset + a + look.column_gap();
    let w = pause_x - look.column_gap() - x;
    let (eh, hh, dh) = (typography::EYEBROW.line_height, typography::HEADLINE.line_height, typography::DETAIL.line_height);
    let gap = island::expanded::LINE_GAP;
    let top = mid - (eh + hh + dh + 2.0 * gap) / 2.0;
    text(p, "Tea", IslandFont::Eyebrow, x, top + eh / 2.0, w, Align::Start, TIMER)?;
    figures(p, "3:42", IslandFont::Headline, x, top + eh + gap + hh / 2.0, w, Align::Start, pal.label_primary)?;
    text(p, "Ends 3:46 PM", IslandFont::Detail, x, top + eh + hh + 2.0 * gap + dh / 2.0, w, Align::Start, pal.label_secondary)
}

/// A ring on the theme's track.
#[allow(clippy::too_many_arguments)]
fn ring(p: &Painter, look: &Look, cx: f32, cy: f32, size: f32, stroke: f32, value: f32, accent: Color) -> Result<()> {
    let radius = (size - stroke) / 2.0;
    p.stroke(Rect::new(cx - radius, cy - radius, cx + radius, cy + radius), radius, stroke, look.palette.fill_track)?;
    p.arc(cx, cy, radius, stroke, value, accent)
}
