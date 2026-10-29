//! Each feature's island, drawn into its page's header card: the same black shapes, sizes,
//! type and accents as the island (from the island's tokens), scaled down onto a stage that
//! stands for the top of the screen, lit by the page's colour. Still at rest; when a page
//! opens, the island emerges from the top edge once, like it does on screen.
//!
//! Shapes here are in the island's own units (inside [`Painter::scaled`]).

mod agents;
mod appearance;
mod clipboard;
mod devices;
mod levels;
mod music;
mod timers;

use reui_design::tokens::{color, component, island, opacity, prefs as s};
use reui_design::Color;
use windows::core::Result;

use super::{Align, Painter};
use crate::ui::layout::{Font, IslandFont, Rect};
use crate::ui::Preview;

pub const BLACK: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };

/// One island shown on the stage: its size in island units and how it draws itself with
/// its top-left at the origin.
pub struct Moment {
    pub w: f32,
    pub draw: fn(&Painter, f32) -> Result<()>,
}

pub fn draw(p: &Painter, preview: Preview, stage: Rect, tint: Color, enter: f32) -> Result<()> {
    if let Preview::Appearance(look) = preview {
        return appearance::draw(p, &look, stage, tint, enter);
    }
    p.stage(stage, s::group::RADIUS, s::color::STAGE, tint.with_alpha(s::opacity::STAGE_TINT))?;
    let (cx, top) = ((stage.l + stage.r) / 2.0, stage.t);
    p.hairline(stage.l, stage.r, stage.b - p.pixel(), Color { r: 1.0, g: 1.0, b: 1.0, a: 0.04 })?;
    let (moments, scale): (Vec<Moment>, f32) = match preview {
        Preview::Idle => (vec![idle()], 1.0),
        Preview::Music => (music::moments(), 0.7),
        Preview::Clipboard => (vec![clipboard::panel()], 0.72),
        Preview::Levels => (levels::moments(), 1.0),
        Preview::Devices => (devices::moments(), 0.8),
        Preview::Timers => (timers::moments(), 0.8),
        Preview::Agents => (agents::moments(), 0.7),
        Preview::Appearance(_) => unreachable!("drawn above"),
    };
    let e = ease_out(enter);
    let gap = 44.0;
    let total: f32 = moments.iter().map(|m| m.w * scale).sum::<f32>() + gap * (moments.len().saturating_sub(1)) as f32;
    let mut x = cx - total / 2.0;
    // Emerging from the top edge: down from above, up from a smaller size.
    let rise = island::TOP * scale + (1.0 - e) * -motion_rise();
    let grow = 0.85 + 0.15 * e;
    p.alpha.set(e.min(1.0));
    for m in &moments {
        let w = m.w * scale;
        let (mx, my) = (x + w * (1.0 - grow) / 2.0, top + rise);
        p.scaled(p.snap(mx), p.snap(my), scale * grow, || (m.draw)(p, e))?;
        x += w + gap;
    }
    p.alpha.set(1.0);
    Ok(())
}

fn motion_rise() -> f32 {
    reui_design::tokens::motion::show::RISE
}

pub fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// The resting pill.
fn idle() -> Moment {
    Moment { w: island::idle::WIDTH, draw: |p, _| pill(p, Rect::at(0.0, 0.0, island::idle::WIDTH, island::HEIGHT)) }
}

/// The black capsule.
pub fn pill(p: &Painter, r: Rect) -> Result<()> {
    p.fill(r, r.h() / 2.0, BLACK)
}

/// The compact presentation's width for sides `leading` and `trailing` wide (margins
/// included): both sides as wide as the wider, around the centre gap.
pub fn compact_width(leading: f32, trailing: f32) -> f32 {
    island::compact::CENTER_GAP + 2.0 * leading.max(trailing)
}

/// Artwork: a gradient in a circle or a concentric tile.
pub fn art(p: &Painter, r: Rect, radius: f32, top: Color, bottom: Color) -> Result<()> {
    p.gradient(r, radius, top, bottom)?;
    p.edge(r, radius, Color { r: 1.0, g: 1.0, b: 1.0, a: opacity::ART_STROKE })
}

/// An accent-tinted circle or tile with a glyph in the accent (`Art::Glyph`).
pub fn tinted(p: &Painter, r: Rect, radius: f32, glyph: char, size: f32, accent: Color) -> Result<()> {
    p.fill(r, radius, accent.with_alpha(opacity::TINT_FILL))?;
    p.glyph(glyph, size, r, accent)
}

/// Level bars rising from their resting levels as the preview settles.
pub fn bars(p: &Painter, x: f32, mid: f32, levels: &[f32], accent: Color, e: f32) -> Result<()> {
    let (w, gap, h) = (component::bars::WIDTH, component::bars::GAP, component::bars::HEIGHT);
    for (i, level) in levels.iter().enumerate() {
        let bh = (h * level * (0.3 + 0.7 * e)).max(w);
        let l = x + i as f32 * (w + gap);
        p.fill(Rect::new(l, mid - bh / 2.0, l + w, mid + bh / 2.0), w / 2.0, accent)?;
    }
    Ok(())
}

pub fn bars_width(count: usize) -> f32 {
    count as f32 * component::bars::WIDTH + (count.saturating_sub(1)) as f32 * component::bars::GAP
}

/// A level: an accent fill on a track (volume, brightness, progress).
pub fn level(p: &Painter, r: Rect, value: f32, accent: Color) -> Result<()> {
    p.fill(r, r.h() / 2.0, color::fill::TRACK)?;
    p.fill(Rect::new(r.l, r.t, r.l + (r.w() * value).max(r.h()), r.b), r.h() / 2.0, accent)
}

/// A ring: the track, and the accent over `value` of it.
pub fn ring(p: &Painter, cx: f32, cy: f32, size: f32, stroke: f32, value: f32, accent: Color) -> Result<()> {
    let radius = (size - stroke) / 2.0;
    p.stroke(Rect::new(cx - radius, cy - radius, cx + radius, cy + radius), radius, stroke, color::fill::TRACK)?;
    p.arc(cx, cy, radius, stroke, value, accent)
}

/// Island text, one line.
#[allow(clippy::too_many_arguments)]
pub fn text(p: &Painter, text: &str, font: IslandFont, x: f32, mid: f32, w: f32, align: Align, ink: Color) -> Result<()> {
    let h = super::fonts::style(Font::Island(font)).line_height;
    p.text(text, Font::Island(font), Rect::at(x, mid - h / 2.0, w, h), align, ink)
}

/// Island figures (tabular), one line.
#[allow(clippy::too_many_arguments)]
pub fn figures(p: &Painter, text: &str, font: IslandFont, x: f32, mid: f32, w: f32, align: Align, ink: Color) -> Result<()> {
    let h = super::fonts::style(Font::Island(font)).line_height;
    p.figures(text, Font::Island(font), Rect::at(x, mid - h / 2.0, w, h), align, ink)
}

/// The expanded header's eyebrow, hero and detail beside a 56 leading tile, from `x`.
pub fn header_text(p: &Painter, x: f32, w: f32, eyebrow: &str, hero: &str, detail: &str, accent: Color) -> Result<()> {
    use island::expanded as ex;
    let (eh, hh, dh) = (reui_design::tokens::typography::EYEBROW.line_height, reui_design::tokens::typography::HEADLINE.line_height, reui_design::tokens::typography::DETAIL.line_height);
    let total = eh + hh + dh + 2.0 * ex::LINE_GAP;
    let top = ex::INSET + (component::art::EXPANDED - total) / 2.0;
    text(p, eyebrow, IslandFont::Eyebrow, x, top + eh / 2.0, w, Align::Start, accent)?;
    let hy = top + eh + ex::LINE_GAP;
    text(p, hero, IslandFont::Headline, x, hy + hh / 2.0, w, Align::Start, color::label::PRIMARY)?;
    let dy = hy + hh + ex::LINE_GAP;
    text(p, detail, IslandFont::Detail, x, dy + dh / 2.0, w, Align::Start, color::label::SECONDARY)
}
