//! The slider row (volume and the like): toggle, bar, value while dragging, menu button.


use reui_design::tokens::{color, component, island, space};
use reui_design::Color;
use reui_protocol::{Button, ButtonRole, Menu};
use windows::core::Result;
use windows::Win32::Graphics::Direct2D::*;

use crate::features::music;
use crate::render::{Align, Hit, Renderer};

impl Renderer {
    /// Volume and the like: the leading toggle (mute), the bar, and the trailing menu button
    /// (output device). While `dragging`, the value shows as a percentage over the thumb, in
    /// the gap above the row, so nothing in the row moves. Returns the row's height.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::render) fn slider(&self, rt: &ID2D1RenderTarget, leading: &Button, value: f32, action: &str, trailing: Option<&Menu>, y: f32, thumb: bool, dragging: bool, accent: Color, hits: &mut Vec<Hit>) -> Result<f32> {
        let h = component::slider::HEIGHT;
        let gap = component::slider::GAP;
        let inset = island::expanded::INSET;
        let (mut l, mut r) = (inset, self.width.get() - inset);
        self.button(rt, leading, l, y, h, h, accent, hits)?;
        l += h + gap;
        if let Some(menu) = trailing {
            let button = Button { action: String::new(), content: menu.content.clone(), role: ButtonRole::Plain };
            self.button(rt, &button, r - h, y, h, h, accent, hits)?;
            if let Some(hit) = hits.last_mut() {
                hit.menu = true;
            }
            r -= h + gap;
        }
        let bar = component::progress::HEIGHT;
        let top = y + (h - bar) / 2.0;
        let end = l + (r - l) * value.clamp(0.0, 1.0);
        unsafe {
            self.round_rect(rt, l, top, r, top + bar, bar / 2.0, color::fill::TRACK)?;
            self.round_rect(rt, l, top, end, top + bar, bar / 2.0, accent)?;
        }
        if thumb {
            self.thumb(rt, end, y + h / 2.0)?;
        }
        if dragging {
            let text = music::percent(value);
            let font = &self.fonts.caption;
            let tw = self.text_width(&text, font)?;
            let reach = component::thumb::SIZE / 2.0;
            let x = (end - tw / 2.0).min(r + reach - tw).max(l - reach);
            let bottom = y + (h - component::thumb::SIZE) / 2.0 - space::XXS;
            self.text(rt, &text, font, x, bottom - font.style.line_height, tw + 1.0, Align::Start, color::label::SECONDARY)?;
        }
        // The whole row height, and a little past the ends so the extremes are easy to hit.
        let reach = component::thumb::SIZE / 2.0;
        hits.push(Hit { slider: Some((l, r)), ..Hit::new(l - reach, y, r + reach, y + h, action) });
        Ok(h)
    }
}
