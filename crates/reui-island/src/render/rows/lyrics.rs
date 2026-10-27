//! Synced lyrics: the current line and its neighbours, sliding up as lines change.


use reui_design::tokens::{color, component, island};
use reui_design::Color;
use reui_protocol::LyricLine;
use windows::core::Result;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;

use crate::features::music;
use crate::render::{mix, Renderer};
use crate::render::text::utf16_prefix;

impl Renderer {
    /// The line being sung (primary) with what comes next under it (secondary), centred, as
    /// a column: a long line wraps (up to the row's height) and pushes the next one down or
    /// out. When a line starts, the column slides up from the previous one, which fades out
    /// above while the new one brightens into place. An empty line (an instrumental gap)
    /// shows nothing. Always the same height, so lines changing never resize the card.
    pub(in crate::render) fn lyrics(&self, rt: &ID2D1RenderTarget, lines: &[LyricLine], position: Option<u64>, y: f32, motion: Option<(usize, f32)>) -> Result<f32> {
        let line = self.fonts.lyric_wrap.style.line_height;
        let rows = component::lyrics::LINES.max(1.0) as u32;
        let height = line * rows as f32;
        let inset = island::expanded::INSET;
        let width = self.width.get() - 2.0 * inset;
        let Some(position) = position else { return Ok(height) };
        let current = music::current_line(lines, position);
        let first = current.unwrap_or(0);
        let text = |i: usize| lines.get(i).map_or("", |l| l.text.trim());
        unsafe { rt.PushAxisAlignedClip(&self.rect(inset, y, inset + width, y + height), D2D1_ANTIALIAS_MODE_ALIASED) };
        let drawn = (|| -> Result<()> {
            let mut top = y;
            // Mid-change: start one (previous) block higher and slide up to the current one.
            let leaving = motion.filter(|(prev, _)| current == Some(prev + 1));
            if let Some((prev, t)) = leaving {
                let h = self.lyric_height(text(prev), width, rows)?;
                top = y - t * h;
                self.lyric_block(rt, text(prev), inset, top, width, rows, color::label::PRIMARY.with_alpha(1.0 - t))?;
                top += h;
            }
            let mut index = first;
            while top < y + height && index < lines.len() + 1 {
                let ink = match (current == Some(index), leaving) {
                    (true, Some((_, t))) => mix(color::label::SECONDARY, color::label::PRIMARY, t),
                    (true, None) => color::label::PRIMARY,
                    _ => color::label::SECONDARY,
                };
                top += self.lyric_block(rt, text(index), inset, top, width, rows, ink)?;
                index += 1;
            }
            Ok(())
        })();
        unsafe { rt.PopAxisAlignedClip() };
        drawn?;
        Ok(height)
    }

    /// Height of a lyric line wrapped to `width`, at most `max_lines` lines (an empty line
    /// still takes one line: the pause between verses).
    fn lyric_height(&self, text: &str, width: f32, max_lines: u32) -> Result<f32> {
        let line = self.fonts.lyric_wrap.style.line_height;
        if text.is_empty() {
            return Ok(line);
        }
        let wide = utf16_prefix(text, 600);
        let lines = unsafe {
            let layout = self.dwrite.CreateTextLayout(&wide, &self.fonts.lyric_wrap.format, width * self.scale, f32::MAX)?;
            let mut metrics = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut metrics)?;
            metrics.lineCount
        };
        Ok(line * lines.clamp(1, max_lines.max(1)) as f32)
    }

    /// Draws one lyric line centred and wrapped (ellipsised past `max_lines`); returns its
    /// height.
    #[allow(clippy::too_many_arguments)]
    fn lyric_block(&self, rt: &ID2D1RenderTarget, text: &str, left: f32, top: f32, width: f32, max_lines: u32, ink: Color) -> Result<f32> {
        let h = self.lyric_height(text, width, max_lines)?;
        if text.is_empty() || ink.a <= 0.0 {
            return Ok(h);
        }
        let font = &self.fonts.lyric_wrap;
        let wide = utf16_prefix(text, 600);
        unsafe {
            let layout = self.dwrite.CreateTextLayout(&wide, &font.format, width * self.scale, h * self.scale)?;
            layout.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            layout.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, font.style.line_height * self.scale, font.style.line_height * 0.8 * self.scale)?;
            rt.DrawTextLayout(self.point(left, top), &layout, &self.brush(rt, ink)?, D2D1_DRAW_TEXT_OPTIONS_CLIP | D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT);
        }
        Ok(h)
    }
}
