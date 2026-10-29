//! The compact presentation: two sides around the centre gap, plus the animated logo mark.
//! A level (volume, brightness) instead fills the whole pill as a meter.

use std::borrow::Cow;

use reui_design::layout::compact;
use reui_design::tokens::{component, island, opacity};
use reui_design::Color;
use reui_protocol::{Art, CompactView, Element, ImageId, Tone};
use windows::core::Result;
use windows::UI::Composition::CompositionDrawingSurface;
use windows::Win32::Graphics::Direct2D::ID2D1RenderTarget;

use crate::render::{Align, CompactLayout, Renderer, Slot, COMPACT_SURFACE};
use crate::render::elements::shown_level;

impl Renderer {
    /// Draws the compact presentation centred in its surface. Both sides get the same
    /// width (the wider one's, margins included), as in Apple's compact layout, except a
    /// text label, which sits beside its mark.
    /// `level` is the activity's level as shown right now (eased), over the view's own.
    pub fn draw_compact(&self, surface: &CompositionDrawingSurface, view: Option<&CompactView>, accent: Color, level: Option<f32>) -> Result<CompactLayout> {
        let mut layout = CompactLayout { width: island::idle::WIDTH, bars: None, logo: None };
        self.paint(surface, |rt| {
            let Some(view) = view else { return Ok(()) };
            let (leading, trailing) = (shown_level(&view.leading, level), shown_level(&view.trailing, level));
            if let Element::Level { value, dim } = trailing.as_ref() {
                self.level_pill(rt, &leading, *value, *dim, accent)?;
                layout.width = island::compact::METER_WIDTH;
                return Ok(());
            }
            // Windows has no camera cutout to keep clear: a label reads with its mark, side
            // by side, and the pill is only as wide as the two.
            if let (Element::Text { text, tone }, false) = (trailing.as_ref(), matches!(*leading, Element::Empty)) {
                let (lw, _) = self.measure(&leading, Slot::Compact)?;
                let lm = compact_margin(&view.leading);
                let font = self.text_font(Slot::Compact);
                let j = compact::joined(lw, lm, self.text_width(text, font)?);
                let left = compact::left(j.width);
                let mid = COMPACT_SURFACE.1 / 2.0;
                self.element(rt, &leading, Slot::Compact, left + lm, mid, Align::Start, accent)?;
                self.text(rt, text, font, left + j.text_x, mid - font.style.line_height / 2.0, j.text_w, Align::Start, self.tone_color(*tone, accent))?;
                layout.width = j.width;
                let tile = component::art::COMPACT;
                layout.logo = match &view.leading {
                    Element::Art(Art::Logo { image, animated: true }) => Some((*image, left + lm, mid - tile / 2.0)),
                    _ => None,
                };
                return Ok(());
            }
            let (lw, _) = self.measure(&leading, Slot::Compact)?;
            let (tw, _) = self.measure(&trailing, Slot::Compact)?;
            let sides = compact::sides(lw, compact_margin(&view.leading), tw, compact_margin(&view.trailing));
            let left = compact::left(sides.width);
            let mid = COMPACT_SURFACE.1 / 2.0;
            let leading_x = left + sides.leading;
            let trailing_x = left + sides.trailing;
            self.element(rt, &leading, Slot::Compact, leading_x, mid, Align::Start, accent)?;
            self.element(rt, &trailing, Slot::Compact, trailing_x, mid, Align::End, accent)?;
            layout.width = sides.width;
            layout.bars = match (&view.leading, &view.trailing) {
                (Element::Bars { live }, _) => Some((leading_x, mid, *live)),
                (_, Element::Bars { live }) => Some((trailing_x - tw, mid, *live)),
                _ => None,
            };
            let tile = component::art::COMPACT;
            layout.logo = match (&view.leading, &view.trailing) {
                (Element::Art(Art::Logo { image, animated: true }), _) => Some((*image, leading_x, mid - tile / 2.0)),
                (_, Element::Art(Art::Logo { image, animated: true })) => Some((*image, trailing_x - tile, mid - tile / 2.0)),
                _ => None,
            };
            Ok(())
        })?;
        Ok(layout)
    }

    /// A level (volume, brightness) as the whole pill: the pill is the track, an accent fill
    /// grows from the left (inset, so its round ends are concentric with the pill's), the
    /// glyph sits centred in the fill's left end and the percentage at the right end. A
    /// muted level keeps its fill, dimmed, and its number turns quiet.
    fn level_pill(&self, rt: &ID2D1RenderTarget, leading: &Element, value: f32, dim: bool, accent: Color) -> Result<()> {
        let m = compact::meter(&self.look, value);
        let (left, h) = (compact::left(m.width), COMPACT_SURFACE.1);
        let mid = h / 2.0;
        if let Some(fill) = m.fill {
            let alpha = opacity::METER_FILL * if dim { opacity::LEVEL_DIM } else { 1.0 };
            let right = left + m.inset + fill;
            unsafe { self.round_rect(rt, left + m.inset, m.inset, right, h - m.inset, m.corner, accent.with_alpha(accent.a * alpha))? };
        }
        // The fill is the tint here, so a glyph tile becomes the bare glyph.
        let glyph = match leading {
            Element::Art(Art::Glyph(g)) => Cow::Owned(Element::Glyph { glyph: *g, tone: Tone::Primary }),
            other => Cow::Borrowed(other),
        };
        self.element(rt, &glyph, Slot::Compact, left + m.glyph_x, mid, Align::Center, accent)?;
        let tone = if dim { Tone::Secondary } else { Tone::Primary };
        let value = value.clamp(0.0, 1.0);
        let percent = Element::Text { text: format!("{}%", (value * 100.0).round() as u32), tone };
        self.element(rt, &percent, Slot::Compact, left + m.percent_right, mid, Align::End, accent)
    }

    /// The mark alone, centred in a square surface, for the pill's animated logo visual.
    pub fn draw_logo(&self, surface: &CompositionDrawingSurface, image: ImageId, size: f32, color: Color) -> Result<()> {
        self.paint(surface, |rt| {
            let mark = super::images::mark_size(size, opacity::LOGO_BARE);
            self.svg(rt, image, (size - mark) / 2.0, (size - mark) / 2.0, mark, color)
        })
    }
}

/// Round content sits concentric in the capsule's end; text and level bars keep clear of
/// the curve.
fn compact_margin(element: &Element) -> f32 {
    compact::margin(matches!(element, Element::Art(_) | Element::Ring(_) | Element::ClockRing { .. }))
}
