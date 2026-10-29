//! Buttons: a centred row of them, one button, and the source switcher (pager).


use reui_design::layout::{header, rows};
use reui_design::tokens::{icon, opacity};
use reui_design::Color;
use reui_protocol::{Button, ButtonContent, ButtonRole, Pager};
use windows::core::Result;
use windows::Win32::Graphics::Direct2D::*;

use crate::render::{card, Align, Hit, Renderer};

impl Renderer {
    /// All-glyph rows are transport-style circles centred as a group; any text makes the
    /// row equal-width capsules across the full width.
    pub(in crate::render) fn button_row(&self, rt: &ID2D1RenderTarget, buttons: &[Button], y: f32, accent: Color, hits: &mut Vec<Hit>) -> Result<f32> {
        let w = self.width.get();
        if buttons.iter().all(|b| matches!(b.content, ButtonContent::Glyph(_))) {
            let t = rows::transport(w, buttons.len());
            let mut x = t.left;
            for button in buttons {
                self.button(rt, button, x, y, t.size, t.size, accent, hits)?;
                x += t.size + t.gap;
            }
            Ok(t.size)
        } else {
            let row = rows::text_buttons(&self.look, card(self.size.get()), w, buttons.len());
            for (i, button) in buttons.iter().enumerate() {
                self.button(rt, button, row.x(i), y, row.width, row.height, accent, hits)?;
            }
            Ok(row.height)
        }
    }

    /// A capsule (or circle when square) button.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::render) fn button(&self, rt: &ID2D1RenderTarget, button: &Button, x: f32, y: f32, w: f32, h: f32, accent: Color, hits: &mut Vec<Hit>) -> Result<()> {
        let (fill, ink) = self.button_colors(button.role, accent);
        if let Some(fill) = fill {
            let inset = rows::button_inset(&self.look, card(self.size.get()));
            unsafe { self.round_rect(rt, x, y, x + w, y + h, self.capsule(h, inset), fill)? };
        }
        match &button.content {
            ButtonContent::Glyph(g) => {
                let size = self.fonts.icon_button.style.size;
                self.text(rt, &g.to_string(), &self.fonts.icon_button, x, y + (h - size) / 2.0, w, Align::Center, ink)?;
            }
            ButtonContent::Text(t) => {
                let line = self.fonts.button.style.line_height;
                self.text(rt, t, &self.fonts.button, x, y + (h - line) / 2.0, w, Align::Center, ink)?;
            }
        }
        hits.push(Hit::new(x, y, x + w, y + h, &button.action));
        Ok(())
    }

    /// "‹ 1/2 ›" ending at `right`, centred on `mid`: quiet tabular text between two chevron
    /// targets, the whole of it a wheel target. Returns its left edge.
    pub(in crate::render) fn pager(&self, rt: &ID2D1RenderTarget, pager: &Pager, right: f32, mid: f32, hits: &mut Vec<Hit>) -> Result<f32> {
        let label = format!("{}/{}", pager.index + 1, pager.count);
        let font = &self.fonts.pager;
        let label_w = self.text_width(&label, font)?;
        let p = header::pager(right, label_w);
        let (hit, prev_l, label_l, next_l) = (p.hit, p.prev, p.label, p.next);
        let glyph = icon::PAGER;
        let (top, bottom) = (mid - hit / 2.0, mid + hit / 2.0);
        let ink = self.pal().label_secondary;
        self.text(rt, "\u{E76B}", &self.fonts.icon_pager, prev_l, mid - glyph / 2.0, hit, Align::Center, ink)?;
        self.text(rt, &label, font, label_l, mid - font.style.line_height / 2.0, label_w + 1.0, Align::Start, ink)?;
        self.text(rt, "\u{E76C}", &self.fonts.icon_pager, next_l, mid - glyph / 2.0, hit, Align::Center, ink)?;
        hits.push(Hit { pager: true, ..Hit::new(prev_l, top, prev_l + hit, bottom, &pager.prev) });
        hits.push(Hit { pager: true, ..Hit::new(next_l, top, right, bottom, &pager.next) });
        hits.push(Hit { pager: true, ..Hit::new(prev_l, top, right, bottom, "") });
        Ok(prev_l)
    }

    /// (fill, ink)
    fn button_colors(&self, role: ButtonRole, accent: Color) -> (Option<Color>, Color) {
        let p = self.pal();
        match role {
            ButtonRole::Primary => (Some(p.label_primary), p.label_on_accent),
            ButtonRole::Secondary => (Some(p.fill_secondary), p.label_primary),
            ButtonRole::Accent => (Some(accent.with_alpha(opacity::TINT_FILL)), accent),
            ButtonRole::Destructive => (Some(p.system_red.with_alpha(opacity::TINT_FILL)), p.system_red),
            ButtonRole::Plain => (None, p.label_primary),
        }
    }
}
