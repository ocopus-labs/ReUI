//! The renderer's DirectWrite text formats: one per typographic role, built once from
//! the design tokens, and which of them an element slot uses.


use reui_design::TextStyle;
use windows::core::{w, Result, HSTRING};
use windows::Win32::Graphics::DirectWrite::*;

use crate::render::{Renderer, Slot};

pub(super) struct Font {
    pub(super) format: IDWriteTextFormat,
    pub(super) style: TextStyle,
    pub(super) tabular: bool,
}

pub(super) struct Fonts {
    pub(super) eyebrow: Font,
    pub(super) value: Font,
    pub(super) headline: Font,
    /// The hero when it may wrap (a roomy card's question or title).
    pub(super) headline_wrap: Font,
    pub(super) code: Font,
    pub(super) detail: Font,
    /// The detail when it may take two lines (roomy cards).
    pub(super) detail_wrap: Font,
    pub(super) compact: Font,
    pub(super) button: Font,
    pub(super) caption: Font,
    pub(super) icon_compact: Font,
    pub(super) icon_button: Font,
    pub(super) icon_tile: Font,
    pub(super) icon_tile_compact: Font,
    pub(super) icon_trailing: Font,
    pub(super) card_text: Font,
    pub(super) card_code: Font,
    pub(super) card_label: Font,
    pub(super) card_title: Font,
    pub(super) tab: Font,
    pub(super) tab_count: Font,
    pub(super) search: Font,
    pub(super) note: Font,
    pub(super) icon_search: Font,
    pub(super) icon_star: Font,
    pub(super) icon_kind: Font,
    pub(super) icon_pager: Font,
    /// Wrapping, for lyric lines too long for one line.
    pub(super) lyric_wrap: Font,
    pub(super) pager: Font,
    /// The session strip above an agent moment's header.
    pub(super) context: Font,
    pub(super) choice: Font,
    pub(super) choice_detail: Font,
    /// A description too long for one line wraps to a second (E-A6).
    pub(super) choice_detail_wrap: Font,
    pub(super) icon_check: Font,
    /// Code and diff block lines (cut, not wrapped); prose blocks wrap.
    pub(super) block_code: Font,
    pub(super) block_prose: Font,
    /// System stats: a bare glyph button at the header's end, the glyph in a gauge's ring,
    /// what a gauge measures, and a fact's label and value.
    pub(super) icon_header: Font,
    pub(super) icon_gauge: Font,
    pub(super) gauge_label: Font,
    pub(super) fact_label: Font,
    pub(super) fact: Font,
}

impl Renderer {
    pub(super) fn text_font(&self, slot: Slot) -> &Font {
        match slot {
            Slot::Compact | Slot::Minimal => &self.fonts.compact,
            Slot::Leading | Slot::Trailing => &self.fonts.detail,
        }
    }

    pub(super) fn glyph_font(&self, slot: Slot) -> &Font {
        match slot {
            Slot::Compact | Slot::Minimal => &self.fonts.icon_compact,
            Slot::Leading => &self.fonts.icon_tile,
            Slot::Trailing => &self.fonts.icon_trailing,
        }
    }
}

/// `wrap`: multi-line text, top-aligned, ellipsised on the last line that fits.
pub(super) fn make_font(dwrite: &IDWriteFactory, style: TextStyle, tabular: bool, wrap: bool, scale: f32) -> Result<Font> {
    unsafe {
        let format = dwrite.CreateTextFormat(
            &HSTRING::from(style.family),
            None,
            DWRITE_FONT_WEIGHT(style.weight as i32),
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            style.size * scale,
            w!("en-us"),
        )?;
        if wrap {
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
            format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)?;
        } else {
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        }
        let sign = dwrite.CreateEllipsisTrimmingSign(&format)?;
        let trimming = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
        format.SetTrimming(&trimming, &sign)?;
        Ok(Font { format, style, tabular })
    }
}
