//! A choice list (an agent's question): one row per option, its label and description,
//! checks when several may be picked, and a ring on the one under the keyboard cursor.
//! At most `choice.max-height` tall: a longer list scrolls inside the card (the wheel, or
//! the cursor moving), so the card never grows past its max-height (E-A6).

use reui_design::layout::rows;
use reui_design::tokens::{component, opacity};
use reui_design::Color;
use reui_protocol::Choice;
use windows::core::Result;
use windows::Win32::Graphics::Direct2D::*;

use crate::render::{Align, Hit, Renderer};

const GLYPH_UNCHECKED: char = '\u{EA3A}';
const GLYPH_CHECKED: char = '\u{EC61}';

/// A drawn list: its height, how far it can scroll, and each choice's span in it.
pub(in crate::render) struct ChoiceList {
    pub height: f32,
    pub max_scroll: f32,
    pub spans: Vec<(f32, f32)>,
}

impl Renderer {
    /// One choice's height and whether its description wraps: the label, then the
    /// description on one line, or two when it doesn't fit.
    fn choice_height(&self, choice: &Choice, text_w: f32) -> Result<(f32, bool)> {
        let measured = match &choice.detail {
            Some(d) => Some(self.text_width(d, &self.fonts.choice_detail)?),
            None => None,
        };
        let lines = rows::choice_detail_lines(measured, text_w);
        Ok((rows::choice_height(lines), lines > 1.0))
    }

    /// The list at `y`, full width, scrolled by `scroll` (clamped here). Each choice is a
    /// quiet well whose corners follow the island's (`radius.concentric`, a capsule when
    /// it's shorter); a picked one in a multi-select list is tinted with the accent and
    /// checked. Only what shows is drawn and hit-tested.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::render) fn choices(&self, rt: &ID2D1RenderTarget, options: &[Choice], multi: bool, cursor: Option<usize>, y: f32, scroll: f32, accent: Color, hits: &mut Vec<Hit>) -> Result<ChoiceList> {
        let cols = rows::choice_columns(&self.look, self.width.get(), multi);
        let (l, r, text_w) = (cols.left, cols.right, cols.text_w);
        let pad = component::choice::PADDING_X;
        let check = component::choice::CHECK;
        let mut sized = Vec::with_capacity(options.len());
        for choice in options {
            sized.push(self.choice_height(choice, text_w)?);
        }
        let heights: Vec<f32> = sized.iter().map(|&(h, _)| h).collect();
        let list = rows::choice_list(&heights, self.taken.get());
        let rows: Vec<(f32, f32, bool)> = list.tops.iter().zip(&sized).map(|(&top, &(h, wraps))| (top, h, wraps)).collect();
        let (shown, max_scroll) = (list.shown, list.max_scroll);
        let scroll = scroll.clamp(0.0, max_scroll);
        let (clip_top, clip_bottom) = (y, y + shown);
        unsafe { rt.PushAxisAlignedClip(&self.rect(l, clip_top, r, clip_bottom), D2D1_ANTIALIAS_MODE_ALIASED) };
        let drawn = (|| -> Result<()> {
            for (i, (choice, &(at, h, wraps))) in options.iter().zip(&rows).enumerate() {
                let top = y + at - scroll;
                if top + h <= clip_top || top >= clip_bottom {
                    continue;
                }
                let corner = self.concentric().min(h / 2.0);
                let picked = multi && choice.selected;
                let fill = if picked { accent.with_alpha(opacity::TINT_FILL) } else { self.pal().fill_quaternary };
                unsafe { self.round_rect(rt, l, top, r, top + h, corner, fill)? };
                if cursor == Some(i) {
                    let s = self.scale;
                    let half = component::choice::RING_WIDTH / 2.0;
                    let ring = D2D1_ROUNDED_RECT { rect: self.rect(l + half, top + half, r - half, top + h - half), radiusX: (corner - half) * s, radiusY: (corner - half) * s };
                    unsafe { rt.DrawRoundedRectangle(&ring, &self.brush(rt, accent)?, component::choice::RING_WIDTH * s, None) };
                }
                let mut line = top + component::choice::PADDING_Y;
                self.text(rt, &choice.label, &self.fonts.choice, l + pad, line, text_w, Align::Start, self.pal().label_primary)?;
                line += self.fonts.choice.style.line_height;
                match &choice.detail {
                    Some(detail) if wraps => {
                        let lines = 2.0 * self.fonts.choice_detail.style.line_height;
                        self.text_block(rt, detail, &self.fonts.choice_detail_wrap, l + pad, line, text_w, lines, self.pal().label_secondary)?
                    }
                    Some(detail) => self.text(rt, detail, &self.fonts.choice_detail, l + pad, line, text_w, Align::Start, self.pal().label_secondary)?,
                    None => {}
                }
                if multi {
                    let (glyph, ink) = if choice.selected { (GLYPH_CHECKED, accent) } else { (GLYPH_UNCHECKED, self.pal().label_tertiary) };
                    self.text(rt, &glyph.to_string(), &self.fonts.icon_check, r - pad - check, top + (h - check) / 2.0, check, Align::Center, ink)?;
                }
                // Hit-tested only where it shows; the wheel anywhere on the list scrolls it.
                hits.push(Hit { choice: Some(i), block: max_scroll > 0.0, ..Hit::new(l, top.max(clip_top), r, (top + h).min(clip_bottom), &choice.action) });
            }
            Ok(())
        })();
        unsafe { rt.PopAxisAlignedClip() };
        drawn?;
        if max_scroll > 0.0 {
            // The gaps between choices scroll the list too.
            hits.push(Hit { block: true, ..Hit::new(l, clip_top, r, clip_bottom, "") });
        }
        Ok(ChoiceList { height: shown, max_scroll, spans: rows.iter().map(|&(at, h, _)| (at, at + h)).collect() })
    }
}
