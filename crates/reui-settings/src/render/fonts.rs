//! DirectWrite text formats, one per typographic role, from the tokens; and measuring.

use reui_design::tokens::{font, settings};
use reui_design::TextStyle;
use windows::core::{w, Result, HSTRING};
use windows::Win32::Graphics::DirectWrite::*;

use super::Renderer;
use crate::ui::layout::{Font, Measure};

pub struct Fonts {
    formats: Vec<(Font, TextStyle, IDWriteTextFormat)>,
    icons: Vec<(f32, IDWriteTextFormat)>,
}

const ROLES: [Font; 10] = [Font::Title, Font::Intro, Font::Label, Font::RowTitle, Font::RowDetail, Font::Nav, Font::Control, Font::Field, Font::Mono, Font::Figure];

fn style(font: Font) -> TextStyle {
    use settings::typography as t;
    match font {
        Font::Title => t::TITLE,
        Font::Intro => t::INTRO,
        Font::Label => t::LABEL,
        Font::RowTitle => t::ROW_TITLE,
        Font::RowDetail => t::ROW_DETAIL,
        Font::Nav => t::NAV,
        Font::Control => t::CONTROL,
        Font::Field => t::FIELD,
        Font::Mono => t::MONO,
        Font::Figure => t::FIGURE,
    }
}

fn format(dwrite: &IDWriteFactory, style: TextStyle) -> Result<IDWriteTextFormat> {
    unsafe {
        let format = dwrite.CreateTextFormat(&HSTRING::from(style.family), None, DWRITE_FONT_WEIGHT(style.weight as i32), DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, style.size, w!("en-us"))?;
        format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        let sign = dwrite.CreateEllipsisTrimmingSign(&format)?;
        let trimming = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
        format.SetTrimming(&trimming, &sign)?;
        Ok(format)
    }
}

impl Fonts {
    pub fn new(dwrite: &IDWriteFactory) -> Result<Self> {
        let mut formats = Vec::new();
        for role in ROLES {
            let s = style(role);
            formats.push((role, s, format(dwrite, s)?));
        }
        let mut icons = Vec::new();
        for size in [settings::icon::CAPTION, settings::icon::CONTROL, 16.0] {
            icons.push((size, format(dwrite, TextStyle { family: font::ICONS, weight: 400, size, line_height: size })?));
        }
        Ok(Self { formats, icons })
    }

    pub fn format(&self, font: Font) -> &IDWriteTextFormat {
        &self.formats.iter().find(|(f, ..)| *f == font).expect("every role has a format").2
    }

    pub fn style(&self, font: Font) -> TextStyle {
        style(font)
    }

    /// The icon format closest to `size`.
    pub fn icon(&self, size: f32) -> &IDWriteTextFormat {
        &self.icons.iter().min_by(|a, b| (a.0 - size).abs().total_cmp(&(b.0 - size).abs())).expect("icon formats").1
    }
}

pub fn width(dwrite: &IDWriteFactory, fonts: &Fonts, tabular: &IDWriteTypography, text: &str, font: Font) -> f32 {
    let wide: Vec<u16> = text.encode_utf16().collect();
    unsafe {
        let Ok(layout) = dwrite.CreateTextLayout(&wide, fonts.format(font), 10_000.0, 100.0) else { return 0.0 };
        // Numbers may be drawn tabular, which is wider: measure them so.
        _ = layout.SetTypography(tabular, DWRITE_TEXT_RANGE { startPosition: 0, length: wide.len() as u32 });
        let mut metrics = DWRITE_TEXT_METRICS::default();
        _ = layout.GetMetrics(&mut metrics);
        metrics.widthIncludingTrailingWhitespace.ceil()
    }
}

impl Measure for Renderer {
    fn width(&self, text: &str, font: Font) -> f32 {
        width(self.dwrite(), &self.fonts, self.tabular(), text, font)
    }

    fn height(&self, text: &str, font: Font, max_width: f32) -> f32 {
        let wide: Vec<u16> = text.encode_utf16().collect();
        let line = style(font).line_height;
        unsafe {
            let Ok(layout) = self.dwrite().CreateTextLayout(&wide, self.fonts.format(font), max_width.max(1.0), 10_000.0) else { return line };
            _ = layout.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP);
            _ = layout.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, line, line * 0.8);
            let mut metrics = DWRITE_TEXT_METRICS::default();
            _ = layout.GetMetrics(&mut metrics);
            (metrics.lineCount.max(1) as f32) * line
        }
    }
}
