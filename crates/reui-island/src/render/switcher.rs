//! The view switcher's tabs: glyphs in circles, the selected one on a soft fill (as the
//! clipboard's filter tabs), the one under the pointer on the quietest fill. No accent:
//! that belongs to the activity in the card. Where they go is `layout::switcher`; the
//! capsule itself (material, clip, hairline) is the pill's visual.
//!
//! Beside it, a decision waiting behind the open place: its accent dot in a bubble of
//! the island's material (`layout::switcher::Waiting`). The dot is the one bit of colour
//! there, and it's the decision's own.

use reui_design::layout::switcher::{Switcher, Waiting, TAB};
use reui_design::tokens::{icon, island};
use reui_design::Color;
use reui_protocol::Element;
use windows::core::Result;
use windows::UI::Composition::CompositionDrawingSurface;
use windows::Win32::Graphics::Direct2D::ID2D1RenderTarget;

use crate::render::{Align, Renderer, Slot};

impl Renderer {
    /// Draws `glyphs` as `layout`'s tabs; `selected` and `hover` are tab indexes.
    pub fn draw_switcher(&self, surface: &CompositionDrawingSurface, layout: &Switcher, glyphs: &[char], selected: Option<usize>, hover: Option<usize>) -> Result<()> {
        self.paint(surface, |rt| self.place_tabs(rt, layout, 0.0, glyphs, selected, hover))
    }

    /// The tabs from `x` (DIPs into the surface); shared with the idle pill's row of places.
    pub(super) fn place_tabs(&self, rt: &ID2D1RenderTarget, layout: &Switcher, x: f32, glyphs: &[char], selected: Option<usize>, hover: Option<usize>) -> Result<()> {
        let top = island::switcher::INSET;
        let glyph_top = top + (TAB - icon::COMPACT) / 2.0;
        for (i, (&left, glyph)) in layout.tabs.iter().zip(glyphs).enumerate() {
            let left = x + left;
            let fill = if Some(i) == selected {
                Some(self.pal().fill_secondary)
            } else if Some(i) == hover {
                Some(self.pal().fill_quaternary)
            } else {
                None
            };
            if let Some(fill) = fill {
                unsafe { self.round_rect(rt, left, top, left + TAB, top + TAB, layout.tab_radius, fill)? };
            }
            let ink = if Some(i) == selected { self.pal().label_primary } else { self.pal().label_secondary };
            self.text(rt, &glyph.to_string(), &self.fonts.icon_compact, left, glyph_top, TAB, Align::Center, ink)?;
        }
        Ok(())
    }

    /// The waiting bubble's face, into a surface `waiting.size` square: the decision's
    /// accent dot, on the quietest fill while the pointer is on it.
    pub fn draw_waiting(&self, surface: &CompositionDrawingSurface, waiting: &Waiting, accent: Color, hover: bool) -> Result<()> {
        self.paint(surface, |rt| {
            let (inset, mid) = ((waiting.size - waiting.inner) / 2.0, waiting.size / 2.0);
            if hover {
                unsafe { self.round_rect(rt, inset, inset, inset + waiting.inner, inset + waiting.inner, waiting.inner_radius, self.pal().fill_quaternary)? };
            }
            self.element(rt, &Element::Dot, Slot::Minimal, mid, mid, Align::Center, accent)
        })
    }
}
