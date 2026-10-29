//! A choice list (an agent's question): one row per option, its label and description,
//! checks when several may be picked, and a ring on the one under the keyboard cursor.

use reui_design::tokens::{color, component, island, opacity, radius, space};
use reui_design::Color;
use reui_protocol::Choice;
use windows::core::Result;
use windows::Win32::Graphics::Direct2D::*;

use crate::render::{Align, Hit, Renderer};

const GLYPH_UNCHECKED: char = '\u{EA3A}';
const GLYPH_CHECKED: char = '\u{EC61}';

impl Renderer {
    /// Height of one choice: its label, and its description under it when it has one.
    fn choice_height(&self, choice: &Choice) -> f32 {
        let label = self.fonts.choice.style.line_height;
        let detail = choice.detail.as_ref().map_or(0.0, |_| self.fonts.choice_detail.style.line_height);
        2.0 * component::choice::PADDING_Y + label + detail
    }

    /// The list at `y`, full width; returns its height. Each choice is a quiet well whose
    /// corners follow the island's (`radius.concentric`, a capsule when it's shorter); a
    /// picked one in a multi-select list is tinted with the accent and checked.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::render) fn choices(&self, rt: &ID2D1RenderTarget, options: &[Choice], multi: bool, cursor: Option<usize>, y: f32, accent: Color, hits: &mut Vec<Hit>) -> Result<f32> {
        let inset = island::expanded::INSET;
        let (l, r) = (inset, self.width.get() - inset);
        let pad = component::choice::PADDING_X;
        let check = component::choice::CHECK;
        let mut top = y;
        for (i, choice) in options.iter().enumerate() {
            if i > 0 {
                top += component::choice::GAP;
            }
            let h = self.choice_height(choice);
            let corner = radius::CONCENTRIC.min(h / 2.0);
            let picked = multi && choice.selected;
            let fill = if picked { accent.with_alpha(opacity::TINT_FILL) } else { color::fill::QUATERNARY };
            unsafe { self.round_rect(rt, l, top, r, top + h, corner, fill)? };
            if cursor == Some(i) {
                let s = self.scale;
                let half = component::choice::RING_WIDTH / 2.0;
                let ring = D2D1_ROUNDED_RECT { rect: self.rect(l + half, top + half, r - half, top + h - half), radiusX: (corner - half) * s, radiusY: (corner - half) * s };
                unsafe { rt.DrawRoundedRectangle(&ring, &self.brush(rt, accent)?, component::choice::RING_WIDTH * s, None) };
            }
            let text_r = if multi { r - pad - check - space::SM } else { r - pad };
            let mut line = top + component::choice::PADDING_Y;
            self.text(rt, &choice.label, &self.fonts.choice, l + pad, line, text_r - l - pad, Align::Start, color::label::PRIMARY)?;
            line += self.fonts.choice.style.line_height;
            if let Some(detail) = &choice.detail {
                self.text(rt, detail, &self.fonts.choice_detail, l + pad, line, text_r - l - pad, Align::Start, color::label::SECONDARY)?;
            }
            if multi {
                let (glyph, ink) = if choice.selected { (GLYPH_CHECKED, accent) } else { (GLYPH_UNCHECKED, color::label::TERTIARY) };
                self.text(rt, &glyph.to_string(), &self.fonts.icon_check, r - pad - check, top + (h - check) / 2.0, check, Align::Center, ink)?;
            }
            hits.push(Hit { choice: Some(i), ..Hit::new(l, top, r, top + h, &choice.action) });
            top += h;
        }
        Ok(top - y)
    }
}
