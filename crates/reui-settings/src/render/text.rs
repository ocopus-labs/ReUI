//! DirectWrite text layouts, kept between frames. Making a layout (shaping, font fallback)
//! is the expensive part of drawing text, and nearly every frame (hover, scroll, a toggle
//! sliding) draws the same strings in the same boxes. A layout is kept while frames keep
//! drawing it and dropped after one frame without it; widths and heights measured for
//! layout are kept until there are many of them.

use std::collections::HashMap;

use windows::core::Result;
use windows::Win32::Graphics::DirectWrite::*;

use super::fonts::{style, Fonts};
use crate::ui::layout::Font;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Face {
    Text(Font),
    /// Segoe Fluent Icons at this size (f32 bits).
    Icon(u32),
}

/// How a layout is set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// One line, vertically centred, aligned 0 start / 1 centre / 2 end, figures tabular.
    Line { align: u8, tabular: bool },
    /// Wrapped from the top, with the role's line height.
    Paragraph,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    text: String,
    face: Face,
    w: u32,
    h: u32,
    kind: Kind,
}

/// Text layouts kept between frames, at most (a few KB each).
const LAYOUTS: usize = 1500;
/// Measurements stop being kept past this many (a page has a few hundred strings).
const MEASURES: usize = 4096;

pub struct TextCache {
    dwrite: IDWriteFactory,
    fonts: Fonts,
    /// Tabular figures, for numbers that change in place.
    tabular: IDWriteTypography,
    live: HashMap<Key, IDWriteTextLayout>,
    previous: HashMap<Key, IDWriteTextLayout>,
    widths: HashMap<(String, Font), f32>,
    heights: HashMap<(String, Font, u32), f32>,
}

impl TextCache {
    pub fn new() -> Result<Self> {
        let dwrite: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        let tabular = unsafe {
            let t = dwrite.CreateTypography()?;
            t.AddFontFeature(DWRITE_FONT_FEATURE { nameTag: DWRITE_FONT_FEATURE_TAG_TABULAR_FIGURES, parameter: 1 })?;
            t
        };
        let fonts = Fonts::new(&dwrite)?;
        Ok(Self { dwrite, fonts, tabular, live: HashMap::new(), previous: HashMap::new(), widths: HashMap::new(), heights: HashMap::new() })
    }

    /// The layout for `text` in a `w` × `h` box, made only if no recent frame had it.
    pub fn layout(&mut self, text: &str, face: Face, w: f32, h: f32, kind: Kind) -> Result<IDWriteTextLayout> {
        let key = Key { text: text.to_string(), face, w: w.max(0.0).to_bits(), h: h.max(0.0).to_bits(), kind };
        if let Some(layout) = self.live.get(&key) {
            return Ok(layout.clone());
        }
        if let Some(layout) = self.previous.remove(&key) {
            self.live.insert(key, layout.clone());
            return Ok(layout);
        }
        let layout = self.make(text, face, w, h, kind)?;
        self.live.insert(key, layout.clone());
        Ok(layout)
    }

    /// Tabular numerals in `font` with a smaller unit on their baseline ("32" "%"), in a
    /// `w` × `h` box: one layout with the unit's range resized, so DirectWrite lines the
    /// baselines up. Not kept (a preview draws a handful, and only when it changes).
    pub fn with_unit(&self, value: &str, unit: &str, font: Font, unit_style: reui_design::TextStyle, w: f32, h: f32) -> Result<(IDWriteTextLayout, DWRITE_TEXT_RANGE)> {
        let layout = self.make(&format!("{value}{unit}"), Face::Text(font), w, h, Kind::Line { align: 0, tabular: true })?;
        let range = DWRITE_TEXT_RANGE { startPosition: value.encode_utf16().count() as u32, length: unit.encode_utf16().count() as u32 };
        if range.length > 0 {
            unsafe {
                layout.SetFontFamilyName(&windows::core::HSTRING::from(unit_style.family), range)?;
                layout.SetFontWeight(DWRITE_FONT_WEIGHT(i32::from(unit_style.weight)), range)?;
                layout.SetFontSize(unit_style.size, range)?;
            }
        }
        Ok((layout, range))
    }

    /// A frame is done. Layouts drawn lately stay (rows scrolled out and back in, pages
    /// visited again); past [`LAYOUTS`], only the last frame's are kept.
    pub fn end_frame(&mut self) {
        if self.previous.len() + self.live.len() > LAYOUTS {
            self.previous = std::mem::take(&mut self.live);
        } else {
            self.previous.extend(self.live.drain());
        }
    }

    /// One line's width (numbers measured tabular, as they may be drawn).
    pub fn width(&mut self, text: &str, font: Font) -> f32 {
        if let Some(&w) = self.widths.get(&(text.to_string(), font)) {
            return w;
        }
        let w = self
            .make(text, Face::Text(font), 10_000.0, 100.0, Kind::Line { align: 0, tabular: true })
            .map(|layout| {
                let mut metrics = DWRITE_TEXT_METRICS::default();
                unsafe { _ = layout.GetMetrics(&mut metrics) };
                metrics.widthIncludingTrailingWhitespace.ceil()
            })
            .unwrap_or(0.0);
        if self.widths.len() > MEASURES {
            self.widths.clear();
        }
        self.widths.insert((text.to_string(), font), w);
        w
    }

    /// Height of `text` wrapped at `width`.
    pub fn height(&mut self, text: &str, font: Font, width: f32) -> f32 {
        let key = (text.to_string(), font, width.to_bits());
        if let Some(&h) = self.heights.get(&key) {
            return h;
        }
        let line = style(font).line_height;
        let h = self
            .make(text, Face::Text(font), width.max(1.0), 10_000.0, Kind::Paragraph)
            .map(|layout| {
                let mut metrics = DWRITE_TEXT_METRICS::default();
                unsafe { _ = layout.GetMetrics(&mut metrics) };
                metrics.lineCount.max(1) as f32 * line
            })
            .unwrap_or(line);
        if self.heights.len() > MEASURES {
            self.heights.clear();
        }
        self.heights.insert(key, h);
        h
    }
}

impl TextCache {
    fn make(&self, text: &str, face: Face, w: f32, h: f32, kind: Kind) -> Result<IDWriteTextLayout> {
    crate::perf::text_layout();
    let wide: Vec<u16> = text.encode_utf16().collect();
    let format = match face {
        Face::Text(font) => self.fonts.format(font).clone(),
        Face::Icon(size) => self.fonts.icon(f32::from_bits(size))?,
    };
    unsafe {
        let layout = self.dwrite.CreateTextLayout(&wide, &format, w.max(0.0), h.max(0.0))?;
        match kind {
            Kind::Line { align, tabular: tab } => {
                layout.SetTextAlignment(match align {
                    0 => DWRITE_TEXT_ALIGNMENT_LEADING,
                    1 => DWRITE_TEXT_ALIGNMENT_CENTER,
                    _ => DWRITE_TEXT_ALIGNMENT_TRAILING,
                })?;
                layout.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
                if tab {
                    layout.SetTypography(&self.tabular, DWRITE_TEXT_RANGE { startPosition: 0, length: wide.len() as u32 })?;
                }
            }
            Kind::Paragraph => {
                let Face::Text(font) = face else { return Ok(layout) };
                let line = style(font).line_height;
                layout.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
                layout.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)?;
                layout.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, line, line * 0.8)?;
            }
        }
        Ok(layout)
    }
}
}
