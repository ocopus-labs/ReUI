//! The view switcher's tabs: glyphs in circles, the selected one on a soft fill (as the
//! clipboard's filter tabs), the one under the pointer on the quietest fill. No accent:
//! that belongs to the activity in the card. Where they go is `layout::switcher`; the
//! capsule itself (material, clip, hairline) is the pill's visual.

use reui_design::layout::switcher::{Switcher, TAB};
use reui_design::tokens::{icon, island};
use windows::core::Result;
use windows::UI::Composition::CompositionDrawingSurface;

use crate::render::{Align, Renderer};

impl Renderer {
    /// Draws `glyphs` as `layout`'s tabs; `selected` and `hover` are tab indexes.
    pub fn draw_switcher(&self, surface: &CompositionDrawingSurface, layout: &Switcher, glyphs: &[char], selected: Option<usize>, hover: Option<usize>) -> Result<()> {
        let top = island::switcher::INSET;
        let glyph_top = top + (TAB - icon::COMPACT) / 2.0;
        self.paint(surface, |rt| {
            for (i, (&left, glyph)) in layout.tabs.iter().zip(glyphs).enumerate() {
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
        })
    }
}
