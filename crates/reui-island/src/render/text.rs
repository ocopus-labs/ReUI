//! Text drawing and measuring: single lines, wrapped blocks, widths, and the formatting
//! helpers (UTF-16 truncation, clocks) they rely on.


use reui_design::{Color, TextStyle};
use windows::core::HSTRING;
use windows::core::Result;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;

use crate::render::{Align, Renderer};
use crate::render::fonts::Font;

impl Renderer {
    fn layout(&self, text: &str, font: &Font, max_width: f32) -> Result<IDWriteTextLayout> {
        let wide: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            let layout = self.dwrite.CreateTextLayout(&wide, &font.format, max_width.max(0.0) * self.scale, font.style.line_height * self.scale)?;
            if font.tabular {
                layout.SetTypography(&self.tabular, DWRITE_TEXT_RANGE { startPosition: 0, length: wide.len() as u32 })?;
            }
            Ok(layout)
        }
    }

    pub(super) fn text_width(&self, text: &str, font: &Font) -> Result<f32> {
        let layout = self.layout(text, font, 10_000.0)?;
        let mut metrics = DWRITE_TEXT_METRICS::default();
        unsafe { layout.GetMetrics(&mut metrics)? };
        Ok((metrics.widthIncludingTrailingWhitespace / self.scale).ceil())
    }

    /// How many lines `text` wraps to in a `width`-wide column (`font` must wrap).
    pub(super) fn wrapped_lines(&self, text: &str, font: &Font, width: f32) -> Result<u32> {
        let layout = self.layout(text, font, width)?;
        let mut metrics = DWRITE_TEXT_METRICS::default();
        unsafe { layout.GetMetrics(&mut metrics)? };
        Ok(metrics.lineCount)
    }

    /// One line of text in a `width`-wide box whose top is `top`; ellipsised if it doesn't fit.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn text(&self, rt: &ID2D1RenderTarget, text: &str, font: &Font, left: f32, top: f32, width: f32, align: Align, color: Color) -> Result<()> {
        let layout = self.layout(text, font, width)?;
        unsafe {
            layout.SetTextAlignment(match align {
                Align::Start => DWRITE_TEXT_ALIGNMENT_LEADING,
                Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
                Align::End => DWRITE_TEXT_ALIGNMENT_TRAILING,
            })?;
            rt.DrawTextLayout(self.point(left, top), &layout, &self.brush(rt, color)?, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT);
        }
        Ok(())
    }

    /// Numerals in `font` with a smaller unit after them on the same baseline ("32" "%"),
    /// the unit in `unit` type and `unit_color`. One layout with the unit's range resized,
    /// so DirectWrite lines up the baselines exactly.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn value_with_unit(&self, rt: &ID2D1RenderTarget, value: &str, unit: &str, font: &Font, unit_style: TextStyle, left: f32, top: f32, width: f32, color: Color, unit_color: Color) -> Result<()> {
        let text = format!("{value}{unit}");
        let layout = self.layout(&text, font, width)?;
        let range = DWRITE_TEXT_RANGE { startPosition: value.encode_utf16().count() as u32, length: unit.encode_utf16().count() as u32 };
        unsafe {
            if range.length > 0 {
                layout.SetFontFamilyName(&HSTRING::from(unit_style.family), range)?;
                layout.SetFontWeight(DWRITE_FONT_WEIGHT(unit_style.weight as i32), range)?;
                layout.SetFontSize(unit_style.size * self.scale, range)?;
                layout.SetDrawingEffect(&self.brush(rt, unit_color)?, range)?;
            }
            rt.DrawTextLayout(self.point(left, top), &layout, &self.brush(rt, color)?, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT);
        }
        Ok(())
    }

    /// Wrapped text in whole lines only, the last one ellipsised if it doesn't all fit.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn text_block(&self, rt: &ID2D1RenderTarget, text: &str, font: &Font, left: f32, top: f32, width: f32, height: f32, color: Color) -> Result<()> {
        let line = font.style.line_height;
        let height = (height / line).floor() * line;
        if height < line {
            return Ok(());
        }
        let wide = utf16_prefix(text, 600);
        unsafe {
            let layout = self.dwrite.CreateTextLayout(&wide, &font.format, width * self.scale, height * self.scale)?;
            layout.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, font.style.line_height * self.scale, font.style.line_height * 0.8 * self.scale)?;
            rt.DrawTextLayout(self.point(left, top), &layout, &self.brush(rt, color)?, D2D1_DRAW_TEXT_OPTIONS_CLIP | D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT);
        }
        Ok(())
    }
}

/// At most `max` UTF-16 units of `text`, never ending in half a surrogate pair (an emoji
/// cut in two draws as a box).
pub(super) fn utf16_prefix(text: &str, max: usize) -> Vec<u16> {
    let mut wide: Vec<u16> = text.encode_utf16().take(max + 1).collect();
    if wide.len() > max {
        wide.truncate(max);
        if wide.last().is_some_and(|u| (0xD800..0xDC00).contains(u)) {
            wide.pop();
        }
    }
    wide
}

/// 83_000 → "1:23", 3_723_000 → "1:02:03".
pub(super) fn clock(ms: u64) -> String {
    let s = ms / 1000;
    match s / 3600 {
        0 => format!("{}:{:02}", s / 60, s % 60),
        h => format!("{h}:{:02}:{:02}", s / 60 % 60, s % 60),
    }
}

/// Time left, rounded up so elapsed + left add up to the length (0:10 and -3:10 of 3:20).
pub(super) fn clock_left(ms: u64) -> String {
    clock(ms.div_ceil(1000) * 1000)
}

#[cfg(test)]
mod tests {
    #[test]
    fn truncation_never_splits_a_surrogate_pair() {
        let text = "ab\u{1F600}c";
        assert_eq!(super::utf16_prefix(text, 3), vec![0x61, 0x62]);
        assert_eq!(super::utf16_prefix(text, 4).len(), 4);
        assert_eq!(super::utf16_prefix(text, 99).len(), 5);
    }

    #[test]
    fn clocks_read_like_players_show_them() {
        assert_eq!(super::clock(83_400), "1:23");
        assert_eq!(super::clock(5_000), "0:05");
        assert_eq!(super::clock(3_723_000), "1:02:03");
    }
}
