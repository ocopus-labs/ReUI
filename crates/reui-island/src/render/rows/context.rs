//! The session strip: one quiet line above an agent moment's header saying whose it is.

use reui_design::tokens::space;
use reui_design::Color;
use reui_protocol::Context;
use windows::core::Result;
use windows::Win32::Graphics::Direct2D::*;

use crate::render::{Align, Renderer, Slot};

impl Renderer {
    /// The strip's height: its mark (a compact-sized circle) or its line, whichever is taller.
    pub(in crate::render) fn context_height(&self, context: &Context) -> Result<f32> {
        Ok(self.measure(&context.mark, Slot::Compact)?.1.max(self.fonts.context.style.line_height))
    }

    /// Mark, title (secondary) and facts (tertiary, after " · ") from the inset to `right`,
    /// on a line whose top is `top`. The title stays whole when it can (it's the short
    /// one, the project); the facts take what's left and are cut first.
    pub(in crate::render) fn context(&self, rt: &ID2D1RenderTarget, context: &Context, top: f32, right: f32, accent: Color) -> Result<()> {
        let inset = self.look.inset();
        let h = self.context_height(context)?;
        let mid = top + h / 2.0;
        let (mw, _) = self.measure(&context.mark, Slot::Compact)?;
        self.element(rt, &context.mark, Slot::Compact, inset, mid, Align::Start, accent)?;
        let font = &self.fonts.context;
        let line_top = mid - font.style.line_height / 2.0;
        let left = inset + if mw > 0.0 { mw + space::SM } else { 0.0 };
        let avail = (right - left).max(0.0);
        let title_w = self.text_width(&context.title, font)?.min(avail);
        self.text(rt, &context.title, font, left, line_top, title_w + 1.0, Align::Start, self.pal().label_secondary)?;
        if !context.facts.is_empty() {
            let facts = format!(" · {}", context.facts.join(" · "));
            let x = left + title_w;
            if right - x > space::LG {
                self.text(rt, &facts, font, x, line_top, right - x, Align::Start, self.pal().label_tertiary)?;
            }
        }
        Ok(())
    }
}
