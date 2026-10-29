//! The search field row: text, caret, and its trailing buttons.


use reui_design::tokens::{component, icon, space};
use reui_design::Color;
use reui_protocol::{Button, ButtonContent};
use windows::core::Result;
use windows::Win32::Graphics::Direct2D::*;

use crate::render::{Align, Hit, Renderer};

impl Renderer {
    /// The search field (magnifier or the module's glyph, text or the quiet placeholder, a steady caret while
    /// the panel has the keyboard) with its icon buttons at the right; returns its height.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::render) fn search(&self, rt: &ID2D1RenderTarget, text: &str, placeholder: Option<&str>, glyph_char: Option<char>, caret: bool, buttons: &[Button], y: f32, accent: Color, hits: &mut Vec<Hit>) -> Result<f32> {
        let h = component::search::HEIGHT;
        let pad = component::search::PADDING;
        let inset = self.look.inset();
        let mut right = self.width.get() - inset;
        for button in buttons.iter().rev() {
            let bw = match &button.content {
                ButtonContent::Glyph(_) => h,
                ButtonContent::Text(t) => self.text_width(t, &self.fonts.button)? + 2.0 * pad,
            };
            right -= bw;
            self.button(rt, button, right, y, bw, h, accent, hits)?;
            right -= component::search::BUTTON_GAP;
        }
        unsafe { self.round_rect(rt, inset, y, right, y + h, self.capsule(h, inset), self.pal().fill_quaternary)? };
        let glyph = icon::SEARCH;
        self.text(rt, &glyph_char.unwrap_or('\u{E721}').to_string(), &self.fonts.icon_search, inset + pad, y + (h - glyph) / 2.0, glyph, Align::Center, self.pal().label_secondary)?;
        let left = inset + pad + glyph + space::SM;
        let avail = (right - pad - left).max(0.0);
        let line = self.fonts.search.style.line_height;
        let top = y + (h - line) / 2.0;
        let mut end = left;
        if text.is_empty() {
            self.text(rt, placeholder.unwrap_or("Search"), &self.fonts.search, left, top, avail, Align::Start, self.pal().label_tertiary)?;
        } else {
            // Long queries show their end, where the typing is.
            let tw = self.text_width(text, &self.fonts.search)?;
            let x = if tw > avail { left + avail - tw } else { left };
            unsafe { rt.PushAxisAlignedClip(&self.rect(left, y, left + avail + component::search::CARET, y + h), D2D1_ANTIALIAS_MODE_ALIASED) };
            let drawn = self.text(rt, text, &self.fonts.search, x, top, tw + 1.0, Align::Start, self.pal().label_primary);
            unsafe { rt.PopAxisAlignedClip() };
            drawn?;
            end = x + tw;
        }
        if caret {
            let ch = line * 0.9;
            let x = (end * self.scale).round() / self.scale;
            unsafe { self.round_rect(rt, x, y + (h - ch) / 2.0, x + component::search::CARET, y + (h + ch) / 2.0, component::search::CARET / 2.0, accent)? };
        }
        Ok(h)
    }
}
