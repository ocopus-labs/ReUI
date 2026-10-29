//! Buttons: a centred row of them, one button, and the source switcher (pager).


use reui_design::tokens::{color, component, icon, island, opacity};
use reui_design::Color;
use reui_protocol::{Button, ButtonContent, ButtonRole, CardSize, Pager};
use windows::core::Result;
use windows::Win32::Graphics::Direct2D::*;

use crate::render::{Align, Hit, Renderer};

impl Renderer {
    /// All-glyph rows are transport-style circles centred as a group; any text makes the
    /// row equal-width capsules across the full width.
    pub(in crate::render) fn button_row(&self, rt: &ID2D1RenderTarget, buttons: &[Button], y: f32, accent: Color, hits: &mut Vec<Hit>) -> Result<f32> {
        let n = buttons.len() as f32;
        let w = self.width.get();
        let inset = island::expanded::INSET;
        if buttons.iter().all(|b| matches!(b.content, ButtonContent::Glyph(_))) {
            let size = component::button::TRANSPORT;
            let gap = component::button::TRANSPORT_GAP;
            let mut x = (w - (n * size + (n - 1.0) * gap)) / 2.0;
            for button in buttons {
                self.button(rt, button, x, y, size, size, accent, hits)?;
                x += size + gap;
            }
            Ok(size)
        } else {
            let h = component::button::HEIGHT;
            // A roomy card's buttons sit like an alert's: a little nearer the edge, further apart.
            let (inset, gap) = match self.size.get() {
                CardSize::Roomy => (component::button::ROOMY_INSET, component::button::ROOMY_GAP),
                _ => (inset, component::button::GAP),
            };
            let width = (w - 2.0 * inset - gap * (n - 1.0)) / n;
            for (i, button) in buttons.iter().enumerate() {
                self.button(rt, button, inset + i as f32 * (width + gap), y, width, h, accent, hits)?;
            }
            Ok(h)
        }
    }

    /// A capsule (or circle when square) button.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::render) fn button(&self, rt: &ID2D1RenderTarget, button: &Button, x: f32, y: f32, w: f32, h: f32, accent: Color, hits: &mut Vec<Hit>) -> Result<()> {
        let (fill, ink) = button_colors(button.role, accent);
        if let Some(fill) = fill {
            unsafe { self.round_rect(rt, x, y, x + w, y + h, h / 2.0, fill)? };
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
        let (hit, gap) = (component::pager::HIT, component::pager::GAP);
        let glyph = icon::PAGER;
        let (top, bottom) = (mid - hit / 2.0, mid + hit / 2.0);
        let next_l = right - hit;
        let label_l = next_l - gap - label_w;
        let prev_l = label_l - gap - hit;
        let ink = color::label::SECONDARY;
        self.text(rt, "\u{E76B}", &self.fonts.icon_pager, prev_l, mid - glyph / 2.0, hit, Align::Center, ink)?;
        self.text(rt, &label, font, label_l, mid - font.style.line_height / 2.0, label_w + 1.0, Align::Start, ink)?;
        self.text(rt, "\u{E76C}", &self.fonts.icon_pager, next_l, mid - glyph / 2.0, hit, Align::Center, ink)?;
        hits.push(Hit { pager: true, ..Hit::new(prev_l, top, prev_l + hit, bottom, &pager.prev) });
        hits.push(Hit { pager: true, ..Hit::new(next_l, top, right, bottom, &pager.next) });
        hits.push(Hit { pager: true, ..Hit::new(prev_l, top, right, bottom, "") });
        Ok(prev_l)
    }
}

/// (fill, ink)
fn button_colors(role: ButtonRole, accent: Color) -> (Option<Color>, Color) {
    match role {
        ButtonRole::Primary => (Some(color::label::PRIMARY), color::label::ON_ACCENT),
        ButtonRole::Secondary => (Some(color::fill::SECONDARY), color::label::PRIMARY),
        ButtonRole::Accent => (Some(accent.with_alpha(opacity::TINT_FILL)), accent),
        ButtonRole::Destructive => (Some(color::system::RED.with_alpha(opacity::TINT_FILL)), color::system::RED),
        ButtonRole::Plain => (None, color::label::PRIMARY),
    }
}
