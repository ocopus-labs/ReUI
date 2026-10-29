//! The island's geometry: where every piece of every presentation goes, as pure functions
//! over a [`Look`] and what was measured. One copy, shared: the island's renderer
//! (`reui-island/src/render`) lays out with it and draws there, and Settings' previews
//! (`reui-settings/src/render/preview`) lay out with it and draw their own way, so a
//! preview can't drift from the island.
//!
//! No text engine here: a width that depends on text (a label, a time, a description)
//! comes in as a number the caller measured. Line heights are the tokens' (each font's
//! `line_height`), which is what both programs lay text out on. Everything is in DIPs at
//! the look's size 1 (the surfaces scale it).
//!
//! - [`compact`]: the compact pill (sides, margins, a label with its mark, the meter) and
//!   the minimal bubble beside it.
//! - [`header`]: the expanded header (columns, eyebrow → hero → detail, centring), the
//!   session strip and the source switcher.
//! - [`rows`]: the full-width rows (buttons, bars and their times, the slider, search,
//!   tabs, choices, text blocks, lyrics).
//! - [`cards`]: the clipboard's scrolling row of cards and its tabs' scroll.
//! - Here: a card's width, corners and height, and the gaps between its rows.

pub mod cards;
pub mod compact;
pub mod header;
pub mod rows;

use crate::look::Look;
use crate::tokens::{component, island};

/// How wide an open card is (the protocol's `CardSize`, which `reui-design` doesn't see).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Card {
    #[default]
    Regular,
    /// Agent moments: wider, and its buttons and detail sit like an alert's.
    Roomy,
    /// A panel (clipboard history).
    Wide,
}

impl Card {
    pub fn width(self) -> f32 {
        match self {
            Card::Regular => island::expanded::WIDTH,
            Card::Roomy => island::expanded::ROOMY_WIDTH,
            Card::Wide => island::expanded::WIDE_WIDTH,
        }
    }

    pub fn roomy(self) -> bool {
        self == Card::Roomy
    }

    /// Its corner radius: the wide panel's is calmer than the card's.
    pub fn radius(self, look: &Look) -> f32 {
        match self {
            Card::Wide => look.panel_radius(),
            _ => look.radius(),
        }
    }

    /// Its tallest: a roomy card may grow further, and roomier padding grows both.
    pub fn max_height(self, look: &Look) -> f32 {
        match self {
            Card::Roomy => look.max_height(island::expanded::ROOMY_MAX_HEIGHT),
            _ => look.max_height(island::expanded::MAX_HEIGHT),
        }
    }

    /// The card's height for content that ends at `natural` (its bottom padding
    /// included): never under the minimum nor over its tallest.
    pub fn height(self, look: &Look, natural: f32) -> f32 {
        natural.clamp(island::expanded::MIN_HEIGHT, self.max_height(look))
    }

    /// Padding under the last row. Alert-style buttons at the bottom of a roomy card keep
    /// the same distance from the bottom edge as from the sides.
    pub fn bottom(self, look: &Look, text_buttons_last: bool) -> f32 {
        if self.roomy() && text_buttons_last {
            look.padded(component::button::ROOMY_INSET)
        } else {
            look.inset()
        }
    }
}

/// The expanded surface: wide enough for the wide card and tall enough for the roomy one
/// at the look's padding; narrower cards are drawn centred in it.
pub fn expanded_surface(look: &Look) -> (f32, f32) {
    (island::expanded::WIDE_WIDTH, look.max_height(island::expanded::ROOMY_MAX_HEIGHT))
}

/// The gap above full-width row `index` (0 is the first under the header): a section gap
/// under the header, none when there is no header, a row gap between rows.
pub fn row_gap(look: &Look, index: usize, headless: bool) -> f32 {
    match index {
        0 if headless => 0.0,
        0 => look.section_gap(),
        _ => look.row_gap(),
    }
}

#[cfg(test)]
mod tests;
