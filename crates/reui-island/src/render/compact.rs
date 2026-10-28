//! The compact presentation: two sides around the centre gap, plus the animated logo mark.


use reui_design::tokens::{component, island, opacity};
use reui_design::Color;
use reui_protocol::{Art, CompactView, Element, ImageId};
use windows::core::Result;
use windows::UI::Composition::CompositionDrawingSurface;

use crate::render::{Align, CompactLayout, Renderer, Slot, COMPACT_SURFACE};
use crate::render::elements::shown_level;

impl Renderer {
    /// Draws the compact presentation centred in its surface. Both sides get the same
    /// width (the wider one's, margins included), as in Apple's compact layout.
    /// `level` is the activity's level as shown right now (eased), over the view's own.
    pub fn draw_compact(&self, surface: &CompositionDrawingSurface, view: Option<&CompactView>, accent: Color, level: Option<f32>) -> Result<CompactLayout> {
        let mut layout = CompactLayout { width: island::idle::WIDTH, bars: None, logo: None };
        self.paint(surface, |rt| {
            let Some(view) = view else { return Ok(()) };
            let (leading, trailing) = (shown_level(&view.leading, level), shown_level(&view.trailing, level));
            let (lw, _) = self.measure(&leading, Slot::Compact)?;
            let (tw, _) = self.measure(&trailing, Slot::Compact)?;
            let (lm, tm) = (compact_margin(&view.leading), compact_margin(&view.trailing));
            let side = (lw + lm).max(tw + tm);
            let width = island::compact::CENTER_GAP + 2.0 * side;
            let left = (COMPACT_SURFACE.0 - width) / 2.0;
            let mid = COMPACT_SURFACE.1 / 2.0;
            let leading_x = left + lm;
            let trailing_x = left + width - tm;
            self.element(rt, &leading, Slot::Compact, leading_x, mid, Align::Start, accent)?;
            self.element(rt, &trailing, Slot::Compact, trailing_x, mid, Align::End, accent)?;
            layout.width = width;
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

    /// The mark alone, centred in a square surface, for the pill's animated logo visual.
    pub fn draw_logo(&self, surface: &CompositionDrawingSurface, image: ImageId, size: f32, color: Color) -> Result<()> {
        self.paint(surface, |rt| {
            let mark = size * opacity::LOGO;
            self.svg(rt, image, (size - mark) / 2.0, (size - mark) / 2.0, mark, color)
        })
    }
}

/// Round content sits concentric in the capsule's end; text and level bars keep clear of
/// the curve.
fn compact_margin(element: &Element) -> f32 {
    match element {
        Element::Art(_) | Element::Ring(_) | Element::ClockRing { .. } => island::compact::SIDE_MARGIN,
        _ => island::compact::TEXT_MARGIN,
    }
}
