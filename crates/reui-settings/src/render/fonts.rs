//! DirectWrite text formats, one per typographic role (from the tokens), and icon formats
//! made once per size.

use std::cell::RefCell;
use std::collections::HashMap;

use reui_design::tokens::{font, prefs::typography as t, typography as island};
use reui_design::TextStyle;
use windows::core::{w, Result, HSTRING};
use windows::Win32::Graphics::DirectWrite::*;

use crate::ui::layout::{Font, IslandFont};

pub struct Fonts {
    formats: HashMap<Font, IDWriteTextFormat>,
    icons: RefCell<HashMap<u32, IDWriteTextFormat>>,
    dwrite: IDWriteFactory,
}

const ROLES: [Font; 14] = [
    Font::Toolbar,
    Font::Hero,
    Font::HeroBody,
    Font::Label,
    Font::RowTitle,
    Font::RowDetail,
    Font::Footnote,
    Font::Nav,
    Font::Control,
    Font::Field,
    Font::Mono,
    Font::Figure,
    Font::Account,
    Font::AccountDetail,
];

const ISLAND: [IslandFont; 14] = [
    IslandFont::Eyebrow,
    IslandFont::Headline,
    IslandFont::Detail,
    IslandFont::Compact,
    IslandFont::Caption,
    IslandFont::Button,
    IslandFont::Code,
    IslandFont::Tab,
    IslandFont::CardText,
    IslandFont::CardLabel,
    IslandFont::Value,
    IslandFont::Context,
    IslandFont::CardTitle,
    IslandFont::CardCode,
];

pub fn style(font: Font) -> TextStyle {
    match font {
        Font::Toolbar => t::TOOLBAR,
        Font::Hero => t::HERO,
        Font::HeroBody => t::HERO_BODY,
        Font::Label => t::LABEL,
        Font::RowTitle => t::ROW_TITLE,
        Font::RowDetail => t::ROW_DETAIL,
        Font::Footnote => t::FOOTNOTE,
        Font::Nav => t::NAV,
        Font::Control => t::CONTROL,
        Font::Field => t::FIELD,
        Font::Mono => t::MONO,
        Font::Figure => t::FIGURE,
        Font::Account => t::ACCOUNT,
        Font::AccountDetail => t::ACCOUNT_DETAIL,
        Font::Island(f) => match f {
            IslandFont::Eyebrow => island::EYEBROW,
            IslandFont::Headline => island::HEADLINE,
            IslandFont::Detail => island::DETAIL,
            IslandFont::Compact => island::COMPACT,
            IslandFont::Caption => island::CAPTION,
            IslandFont::Button => island::BUTTON,
            IslandFont::Code => island::CODE,
            IslandFont::Tab => island::TAB,
            IslandFont::CardText => island::CARD_TEXT,
            IslandFont::CardLabel => island::CARD_LABEL,
            IslandFont::Value => island::VALUE,
            IslandFont::Context => island::CONTEXT,
            IslandFont::CardTitle => island::CARD_TITLE,
            IslandFont::CardCode => island::CARD_CODE,
        },
    }
}

fn format(dwrite: &IDWriteFactory, style: TextStyle) -> Result<IDWriteTextFormat> {
    unsafe {
        let format = dwrite.CreateTextFormat(&HSTRING::from(style.family), None, DWRITE_FONT_WEIGHT(i32::from(style.weight)), DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, style.size, w!("en-us"))?;
        format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        let sign = dwrite.CreateEllipsisTrimmingSign(&format)?;
        let trimming = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
        format.SetTrimming(&trimming, &sign)?;
        Ok(format)
    }
}

impl Fonts {
    pub fn new(dwrite: &IDWriteFactory) -> Result<Self> {
        let mut formats = HashMap::new();
        for role in ROLES.into_iter().chain(ISLAND.map(Font::Island)) {
            formats.insert(role, format(dwrite, style(role))?);
        }
        Ok(Self { formats, icons: RefCell::default(), dwrite: dwrite.clone() })
    }

    pub fn format(&self, font: Font) -> &IDWriteTextFormat {
        &self.formats[&font]
    }

    /// Segoe Fluent Icons at `size`.
    pub fn icon(&self, size: f32) -> Result<IDWriteTextFormat> {
        let key = size.to_bits();
        if let Some(format) = self.icons.borrow().get(&key) {
            return Ok(format.clone());
        }
        let made = format(&self.dwrite, TextStyle { family: font::ICONS, weight: 400, size, line_height: size })?;
        self.icons.borrow_mut().insert(key, made.clone());
        Ok(made)
    }
}
