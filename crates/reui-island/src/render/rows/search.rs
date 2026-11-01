//! The search field row: icon filters before it, text, caret, and its trailing buttons.


use reui_design::layout::rows::{self, Filters};
use reui_design::tokens::{component, icon};
use reui_design::Color;
use reui_protocol::{Button, ButtonRole, Tab};
use windows::core::Result;
use windows::Win32::Graphics::Direct2D::*;

use crate::render::{Align, Hit, Renderer};

impl Renderer {
    /// The search field (magnifier or the module's glyph, text or the quiet placeholder, a
    /// steady caret while the panel has the keyboard) with its icon filters before it and
    /// its buttons at the right; returns its height.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::render) fn search(&self, rt: &ID2D1RenderTarget, text: &str, placeholder: Option<&str>, glyph_char: Option<char>, caret: bool, filters: &[Tab], buttons: &[Button], y: f32, accent: Color, hits: &mut Vec<Hit>) -> Result<f32> {
        let h = component::search::HEIGHT;
        let pad = component::search::PADDING;
        let inset = self.look.inset();
        let mut right = self.width.get() - inset;
        for button in buttons.iter().rev() {
            // A bare glyph (settings) needs no more room than the glyph and a little air.
            let bw = match self.content_width(&button.content)? {
                None if button.role == ButtonRole::Plain => h * 0.75,
                text => rows::search_button(text),
            };
            right -= bw;
            self.button(rt, button, right, y, bw, h, accent, hits)?;
            right -= component::search::BUTTON_GAP;
        }
        let group = Filters::new(inset, filters.len());
        self.filters(rt, filters, &group, y, accent, hits)?;
        let field = group.field_left();
        unsafe { self.round_rect(rt, field, y, right, y + h, self.capsule(h, inset), self.pal().fill_quaternary)? };
        let glyph = icon::SEARCH;
        self.text(rt, &glyph_char.unwrap_or('\u{E721}').to_string(), &self.fonts.icon_search, field + pad, y + (h - glyph) / 2.0, glyph, Align::Center, self.pal().label_secondary)?;
        let left = rows::search_text_after(field);
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

    /// Icon filters in one capsule, the chosen one a filled circle. Once chosen, a filter
    /// with a menu opens it on a click instead (which collection "All" covers).
    fn filters(&self, rt: &ID2D1RenderTarget, filters: &[Tab], group: &Filters, y: f32, accent: Color, hits: &mut Vec<Hit>) -> Result<()> {
        if filters.is_empty() {
            return Ok(());
        }
        let h = component::search::HEIGHT;
        let inset = self.look.inset();
        unsafe { self.round_rect(rt, group.left, y, group.left + group.width(), y + h, self.capsule(h, inset), self.pal().fill_quaternary)? };
        let chip = Filters::chip();
        let top = y + component::search::FILTER_INSET;
        let (chosen_fill, chosen_ink) = self.button_colors(ButtonRole::Primary, accent);
        let glyph = icon::SEARCH;
        for (i, tab) in filters.iter().enumerate() {
            let l = group.chip_left(i);
            let ink = if tab.selected {
                if let Some(fill) = chosen_fill {
                    unsafe { self.round_rect(rt, l, top, l + chip, top + chip, chip / 2.0, fill)? };
                }
                chosen_ink
            } else {
                self.pal().label_secondary
            };
            match tab.glyph {
                Some(g) => self.text(rt, &g.to_string(), &self.fonts.icon_search, l, top + (chip - glyph) / 2.0, chip, Align::Center, ink)?,
                None => {
                    let line = self.fonts.tab.style.line_height;
                    self.text(rt, &tab.label, &self.fonts.tab, l, top + (chip - line) / 2.0, chip, Align::Center, ink)?
                }
            }
            let menu = tab.selected && !tab.menu.is_empty();
            hits.push(Hit { menu, ..Hit::new(l, top, l + chip, top + chip, &tab.action) });
        }
        Ok(())
    }
}
