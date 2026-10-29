//! The mixer (each app's own level, under the volume card's slider): one line per app,
//! its icon (the app's mute) in the spot the volume's mute button has above, its name over
//! its bar. At most `mixer.max-height` tall: a longer list scrolls inside the card, and the
//! wheel then scrolls it rather than moving a level. Where things go is
//! `reui_design::layout::rows::{mixer_line, mixer_list}`.

use reui_design::layout::rows;
use reui_design::tokens::{component, opacity};
use reui_design::Color;
use reui_protocol::Channel;
use windows::core::Result;
use windows::Win32::Graphics::Direct2D::*;

use crate::features::music;
use crate::render::images::Decoded;
use crate::render::{Align, Hit, Interaction, Renderer};

const GLYPH_MUTED: char = '\u{E74F}';

impl Renderer {
    /// The list at `y`, scrolled by `scroll` (clamped here). Returns its height and how far
    /// it can scroll. Only what shows is drawn and hit-tested.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::render) fn mixer(&self, rt: &ID2D1RenderTarget, channels: &[Channel], y: f32, scroll: f32, with: &Interaction, accent: Color, hits: &mut Vec<Hit>) -> Result<(f32, f32)> {
        let w = self.width.get();
        let line = rows::mixer_line(&self.look, w);
        let list = rows::mixer_list(&self.look, channels.len());
        let scroll = scroll.clamp(0.0, list.max_scroll);
        let scrolls = list.max_scroll > 0.0;
        let (clip_top, clip_bottom) = (y, y + list.shown);
        let (left, right) = (self.look.inset(), w - self.look.inset());
        let h = component::mixer::HEIGHT;
        unsafe { rt.PushAxisAlignedClip(&self.rect(0.0, clip_top, w, clip_bottom), D2D1_ANTIALIAS_MODE_ALIASED) };
        let drawn = (|| -> Result<()> {
            for (i, ch) in channels.iter().enumerate() {
                let top = y + i as f32 * list.step - scroll;
                if top + h <= clip_top || top >= clip_bottom {
                    continue;
                }
                self.channel_icon(rt, ch, line.icon.0, top + line.icon.1, line.size)?;
                // The level as the user is setting it, over the module's.
                let set = with.slider.filter(|(a, ..)| *a == ch.action);
                let value = set.map_or(ch.value, |(_, v, _)| v).clamp(0.0, 1.0);
                let dragging = set.is_some_and(|(.., d)| d);
                let (l, r) = line.bar;
                let font = &self.fonts.caption;
                let reading = dragging.then(|| music::percent(value));
                let reading_w = match &reading {
                    Some(t) => self.text_width(t, font)?,
                    None => 0.0,
                };
                self.text(rt, &ch.label, font, l, top + line.label_top, (r - l - reading_w).max(0.0), Align::Start, self.pal().label_secondary)?;
                if let Some(t) = &reading {
                    self.text(rt, t, font, r - reading_w, top + line.label_top, reading_w + 1.0, Align::End, self.pal().label_secondary)?;
                }
                let (bar, bar_top) = (component::progress::HEIGHT, top + line.bar_top);
                let end = l + (r - l) * value;
                let fill = if ch.muted { accent.with_alpha(accent.a * opacity::LEVEL_DIM) } else { accent };
                unsafe {
                    self.round_rect(rt, l, bar_top, r, bar_top + bar, bar / 2.0, self.pal().fill_track)?;
                    self.round_rect(rt, l, bar_top, end, bar_top + bar, bar / 2.0, fill)?;
                }
                if dragging {
                    self.thumb(rt, end, bar_top + bar / 2.0)?;
                }
                let (row_top, row_bottom) = (top.max(clip_top), (top + h).min(clip_bottom));
                // The icon's whole spot mutes; the rest of the line is the bar (as a
                // slider's, a little past its ends so the extremes are easy to hit).
                let spot = component::slider::HEIGHT;
                hits.push(Hit { block: scrolls, ..Hit::new(left, row_top, left + spot, row_bottom, &ch.mute) });
                let reach = component::thumb::SIZE / 2.0;
                hits.push(Hit { slider: Some((l, r)), block: scrolls, ..Hit::new(l - reach, row_top, (r + reach).min(right + reach), row_bottom, &ch.action) });
            }
            Ok(())
        })();
        unsafe { rt.PopAxisAlignedClip() };
        drawn?;
        if scrolls {
            // The gaps between apps scroll the list too.
            hits.push(Hit { block: true, ..Hit::new(left, clip_top, right, clip_bottom, "") });
        }
        Ok((list.shown, list.max_scroll))
    }

    /// The app's icon (a quiet placeholder until it arrives); muted, it's dimmed under the
    /// muted speaker, as the volume's own mute reads.
    fn channel_icon(&self, rt: &ID2D1RenderTarget, ch: &Channel, x: f32, y: f32, size: f32) -> Result<()> {
        let corner = size * 0.25;
        let shape = D2D1_ROUNDED_RECT { rect: self.rect(x, y, x + size, y + size), radiusX: corner * self.scale, radiusY: corner * self.scale };
        match ch.icon.filter(|id| matches!(self.images.get(id), Some(Decoded::Bitmap(_)))) {
            Some(id) => self.cover(rt, id, &shape)?,
            None => unsafe { self.round_rect(rt, x, y, x + size, y + size, corner, self.pal().fill_tertiary)? },
        }
        if ch.muted {
            unsafe { self.round_rect(rt, x, y, x + size, y + size, corner, self.pal().background.with_alpha(1.0 - opacity::LEVEL_DIM))? };
            let font = &self.fonts.icon_tile_compact;
            let g = font.style.size;
            self.text(rt, &GLYPH_MUTED.to_string(), font, x, y + (size - g) / 2.0, size, Align::Center, self.pal().label_primary)?;
        }
        Ok(())
    }
}
