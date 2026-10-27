//! The minimal bubble: one element, centred.


use reui_design::Color;
use reui_protocol::Element;
use windows::core::Result;
use windows::UI::Composition::CompositionDrawingSurface;

use crate::render::{Align, Renderer, Slot, MINIMAL_SURFACE};
use crate::render::elements::{bars_width, shown_level};

impl Renderer {
    /// Draws the minimal bubble's element; returns where live bars go (left, centre, live).
    /// `level` is the activity's level as shown right now (eased); a ring there is that
    /// level in the bubble (the hub sends a ring for minimal), so it eases too.
    pub fn draw_minimal(&self, surface: &CompositionDrawingSurface, element: Option<&Element>, accent: Color, level: Option<f32>) -> Result<Option<(f32, f32, bool)>> {
        let (cx, cy) = (MINIMAL_SURFACE.0 / 2.0, MINIMAL_SURFACE.1 / 2.0);
        self.paint(surface, |rt| match (element, level) {
            (Some(Element::Ring(_)), Some(level)) => self.element(rt, &Element::Ring(level), Slot::Minimal, cx, cy, Align::Center, accent),
            (Some(e), _) => self.element(rt, &shown_level(e, level), Slot::Minimal, cx, cy, Align::Center, accent),
            (None, _) => Ok(()),
        })?;
        Ok(match element {
            Some(Element::Bars { live }) => Some((cx - bars_width() / 2.0, cy, *live)),
            _ => None,
        })
    }
}
