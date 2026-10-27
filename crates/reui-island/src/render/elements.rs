//! Elements in a slot (glyph, text, ring, bars, dot, level, art): their measured size and
//! how they draw.

use std::borrow::Cow;

use reui_design::tokens::{color, component, island, opacity};
use reui_design::Color;
use reui_protocol::Element;
use windows::core::Result;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;

use crate::render::{tone_color, Align, Renderer, Slot, BARS};
use crate::render::images::art_shape;

impl Renderer {
    pub(super) fn measure(&self, element: &Element, slot: Slot) -> Result<(f32, f32)> {
        Ok(match element {
            Element::Empty => (0.0, 0.0),
            Element::Text { text, .. } => {
                let font = self.text_font(slot);
                (self.text_width(text, font)?.min(island::compact::MAX_SIDE), font.style.line_height)
            }
            Element::Glyph { .. } => {
                let s = self.glyph_font(slot).style.size;
                (s, s)
            }
            Element::Art(_) => {
                let s = art_shape(slot).0;
                (s, s)
            }
            Element::Ring(_) => {
                let s = ring_size(slot).0;
                (s, s)
            }
            Element::Bars { .. } => (bars_width(), component::bars::HEIGHT),
            Element::Dot => (component::dot::SIZE, component::dot::SIZE),
            // The bubble has no room for a bar: a level there is a ring.
            Element::Level { .. } if slot == Slot::Minimal => {
                let s = ring_size(slot).0;
                (s, s)
            }
            Element::Level { .. } => (component::level::WIDTH, component::level::HEIGHT),
        })
    }

    /// Draws `element` anchored at `x` (by `align`) and vertically centred on `mid`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn element(&self, rt: &ID2D1RenderTarget, element: &Element, slot: Slot, x: f32, mid: f32, align: Align, accent: Color) -> Result<()> {
        let (w, h) = self.measure(element, slot)?;
        let left = match align {
            Align::Start => x,
            Align::Center => x - w / 2.0,
            Align::End => x - w,
        };
        let top = mid - h / 2.0;
        match element {
            Element::Empty => {}
            Element::Text { text, tone } => self.text(rt, text, self.text_font(slot), left, top, w, Align::Start, tone_color(*tone, accent))?,
            Element::Glyph { glyph, tone } => self.text(rt, &glyph.to_string(), self.glyph_font(slot), left, top, w, Align::Center, tone_color(*tone, accent))?,
            Element::Art(art) => self.art(rt, art, slot, left, top, accent)?,
            Element::Ring(progress) => self.ring(rt, *progress, slot, left + w / 2.0, mid, accent)?,
            // Level bars are always live visuals the pill moves; nothing to draw here.
            Element::Bars { .. } => {}
            Element::Dot => unsafe {
                let r = component::dot::SIZE / 2.0;
                let ellipse = D2D1_ELLIPSE { point: self.point(left + r, mid), radiusX: r * self.scale, radiusY: r * self.scale };
                rt.FillEllipse(&ellipse, &self.brush(rt, accent)?);
            },
            Element::Level { value, dim } => {
                let fill = if *dim { accent.with_alpha(accent.a * opacity::LEVEL_DIM) } else { accent };
                match slot {
                    Slot::Minimal => self.ring(rt, *value, slot, left + w / 2.0, mid, fill)?,
                    _ => self.level(rt, *value, left, top, w, h, fill)?,
                }
            }
        }
        Ok(())
    }

    /// A level bar: the track, and the fill from the left. Anything above zero shows at
    /// least a round dot, so "on but quiet" never reads as off.
    #[allow(clippy::too_many_arguments)]
    fn level(&self, rt: &ID2D1RenderTarget, value: f32, left: f32, top: f32, w: f32, h: f32, fill: Color) -> Result<()> {
        let value = value.clamp(0.0, 1.0);
        // On whole pixels, so a thin bar's edges stay crisp.
        let s = self.scale;
        let (left, top, h) = ((left * s).round() / s, (top * s).round() / s, (h * s).round() / s);
        unsafe {
            self.round_rect(rt, left, top, left + w, top + h, h / 2.0, color::fill::TRACK)?;
            if value > 0.0 {
                self.round_rect(rt, left, top, left + (w * value).max(h), top + h, h / 2.0, fill)?;
            }
        }
        Ok(())
    }

    fn ring(&self, rt: &ID2D1RenderTarget, progress: f32, slot: Slot, cx: f32, cy: f32, accent: Color) -> Result<()> {
        let (size, stroke) = ring_size(slot);
        let s = self.scale;
        // Centre on a pixel boundary (or pixel centre for odd-width strokes) so the stroke
        // edges land on whole pixels instead of being antialiased across two.
        let odd = ((stroke * s).round() as i32) % 2 == 1;
        let snap = |v: f32| ((v * s).round() + if odd { 0.5 } else { 0.0 }) / s;
        let (cx, cy) = (snap(cx), snap(cy));
        let radius = (size - stroke) / 2.0;
        let progress = progress.clamp(0.0, 1.0);
        unsafe {
            let circle = D2D1_ELLIPSE { point: self.point(cx, cy), radiusX: radius * s, radiusY: radius * s };
            rt.DrawEllipse(&circle, &self.brush(rt, color::fill::TRACK)?, stroke * s, &self.round);
            if progress >= 0.999 {
                rt.DrawEllipse(&circle, &self.brush(rt, accent)?, stroke * s, &self.round);
                return Ok(());
            }
            if progress <= 0.001 {
                return Ok(());
            }
            // Clockwise from 12 o'clock.
            let angle = -std::f32::consts::FRAC_PI_2 + progress * std::f32::consts::TAU;
            let base: &ID2D1Factory = &self.factory;
            let path = base.CreatePathGeometry()?;
            let sink = path.Open()?;
            sink.BeginFigure(self.point(cx, cy - radius), D2D1_FIGURE_BEGIN_HOLLOW);
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: self.point(cx + radius * angle.cos(), cy + radius * angle.sin()),
                size: D2D_SIZE_F { width: radius * s, height: radius * s },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: if progress > 0.5 { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
            });
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
            sink.Close()?;
            if opacity::GLOW > 0.0 && !matches!(slot, Slot::Compact | Slot::Minimal) {
                rt.DrawGeometry(&path, &self.brush(rt, accent.with_alpha(opacity::GLOW))?, stroke * 2.0 * s, &self.round);
            }
            rt.DrawGeometry(&path, &self.brush(rt, accent)?, stroke * s, &self.round);
        }
        Ok(())
    }
}

/// (diameter, stroke)
fn ring_size(slot: Slot) -> (f32, f32) {
    match slot {
        Slot::Compact | Slot::Minimal => (component::ring::COMPACT, component::ring::STROKE_COMPACT),
        Slot::Leading | Slot::Trailing => (component::ring::EXPANDED, component::ring::STROKE_EXPANDED),
    }
}

pub fn bars_width() -> f32 {
    BARS as f32 * component::bars::WIDTH + (BARS as f32 - 1.0) * component::bars::GAP
}

/// `element` with a level's value replaced by `level` (its eased value), if both are there.
pub(super) fn shown_level(element: &Element, level: Option<f32>) -> Cow<'_, Element> {
    match (element, level) {
        (Element::Level { dim, .. }, Some(value)) => Cow::Owned(Element::Level { value, dim: *dim }),
        _ => Cow::Borrowed(element),
    }
}
