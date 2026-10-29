//! A panel's scrolling row of cards (clipboard history) and its row of filter tabs.

use crate::tokens::component;

/// Layout of a row of `count` cards in a view `width` wide. Card `i`'s left edge is
/// `inset + i * step - scroll`; the row runs to the panel's right edge and is clipped there.
/// `inset` is the panel's (the look's padding).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CardRow {
    pub width: f32,
    pub count: usize,
    pub inset: f32,
}

impl CardRow {
    pub fn new(width: f32, count: usize, inset: f32) -> Self {
        Self { width, count, inset }
    }

    /// Four across when they all fit; otherwise four and a half, the half card at the edge
    /// saying there's more.
    pub fn card_width(&self) -> f32 {
        let inset = self.inset;
        let gap = component::card::GAP;
        if self.count as f32 <= component::card::VISIBLE {
            let n = component::card::VISIBLE;
            (self.width - 2.0 * inset - gap * (n - 1.0)) / n
        } else {
            let n = component::card::VISIBLE_MORE;
            (self.width - inset - gap * n.floor()) / n
        }
    }

    pub fn step(&self) -> f32 {
        self.card_width() + component::card::GAP
    }

    pub fn left(&self, i: usize, scroll: f32) -> f32 {
        self.inset + i as f32 * self.step() - scroll
    }

    /// Scrolled all the way, the last card ends one inset from the edge.
    pub fn max_scroll(&self) -> f32 {
        let inset = self.inset;
        let content = inset + self.count as f32 * self.step() - component::card::GAP + inset;
        (content - self.width).max(0.0)
    }

    pub fn clamp(&self, scroll: f32) -> f32 {
        scroll.clamp(0.0, self.max_scroll())
    }

    /// The nearest scroll that shows card `i` whole, inside the insets.
    pub fn reveal(&self, scroll: f32, i: usize) -> f32 {
        let inset = self.inset;
        let left = self.left(i, 0.0);
        let right = left + self.card_width();
        let scroll = if left - scroll < inset {
            left - inset
        } else if right - scroll > self.width - inset {
            right - (self.width - inset)
        } else {
            scroll
        };
        self.clamp(scroll)
    }

    /// Cards at least partly inside the panel: the only ones drawn and hit-tested.
    pub fn visible(&self, scroll: f32) -> std::ops::Range<usize> {
        let cw = self.card_width();
        let start = (0..self.count).find(|&i| self.left(i, scroll) + cw > 0.0).unwrap_or(self.count);
        let end = (start..self.count).find(|&i| self.left(i, scroll) >= self.width).unwrap_or(self.count);
        start..end
    }

    /// The first card mostly in view; Alt+1 means this one.
    pub fn first_shown(&self, scroll: f32) -> usize {
        let cw = self.card_width();
        (0..self.count).find(|&i| self.left(i, scroll) + cw / 2.0 >= 0.0).unwrap_or(0)
    }

    /// A wheel turn: one notch (120) is one card. Trackpads send smaller deltas and move
    /// proportionally. Positive is backwards (wheel up / tilt left, already negated).
    pub fn wheel(&self, target: f32, delta: i32) -> f32 {
        self.clamp(target - delta as f32 / 120.0 * self.step())
    }
}

/// A filter tab, "Label count", its label and count measured.
pub fn tab_width(label: f32, count: f32) -> f32 {
    let pad = component::tabs::PADDING;
    pad + label + component::tabs::COUNT_GAP + count + pad
}

/// Horizontal offset of a row of tabs `widths` wide (with `gap` between) in `avail`, so the
/// selected one stays visible; 0 when they all fit.
pub fn tabs_offset(widths: &[f32], gap: f32, selected: Option<usize>, avail: f32) -> f32 {
    let total: f32 = widths.iter().sum::<f32>() + gap * widths.len().saturating_sub(1) as f32;
    if total <= avail {
        return 0.0;
    }
    let Some(sel) = selected.filter(|&i| i < widths.len()) else { return 0.0 };
    let right: f32 = widths[..=sel].iter().sum::<f32>() + gap * sel as f32;
    (right - avail).clamp(0.0, total - avail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens::island;

    const W: f32 = 560.0;

    #[test]
    fn four_fit_and_more_show_a_half_card() {
        let four = CardRow::new(W, 4, island::expanded::INSET);
        assert_eq!(four.max_scroll(), 0.0);
        let right = four.left(3, 0.0) + four.card_width();
        assert!((right - (W - island::expanded::INSET)).abs() < 0.01);

        let many = CardRow::new(W, 10, island::expanded::INSET);
        // Four whole cards, then half of the fifth cut by the panel's edge.
        let fifth = many.left(4, 0.0);
        assert!((W - fifth - many.card_width() / 2.0).abs() < 0.01);
        assert_eq!(many.visible(0.0), 0..5);
    }

    #[test]
    fn scrolling_clamps_and_steps_a_card_per_notch() {
        let row = CardRow::new(W, 10, island::expanded::INSET);
        assert_eq!(row.wheel(0.0, 120), 0.0);
        assert!((row.wheel(0.0, -120) - row.step()).abs() < 0.01);
        assert_eq!(row.wheel(0.0, -120 * 50), row.max_scroll());
        // At the end, the last card ends one inset from the edge.
        let last = row.left(9, row.max_scroll()) + row.card_width();
        assert!((last - (W - island::expanded::INSET)).abs() < 0.01);
    }

    #[test]
    fn revealing_a_card_scrolls_just_enough() {
        let row = CardRow::new(W, 10, island::expanded::INSET);
        assert_eq!(row.reveal(0.0, 2), 0.0);
        let s = row.reveal(0.0, 4);
        assert!((row.left(4, s) + row.card_width() - (W - island::expanded::INSET)).abs() < 0.01);
        assert_eq!(row.reveal(s, 1), s, "already in view");
        assert!((row.left(1, row.reveal(row.max_scroll(), 1)) - island::expanded::INSET).abs() < 0.01);
        assert_eq!(row.first_shown(row.step() * 3.0), 3);
    }

    #[test]
    fn tabs_scroll_only_when_too_wide() {
        assert_eq!(tabs_offset(&[50.0, 50.0], 6.0, Some(1), 200.0), 0.0);
        assert_eq!(tabs_offset(&[100.0, 100.0, 100.0], 0.0, Some(2), 200.0), 100.0);
        assert_eq!(tabs_offset(&[100.0, 100.0, 100.0], 0.0, Some(0), 200.0), 0.0);
    }

    #[test]
    fn a_tab_is_its_label_and_count_padded() {
        let pad = component::tabs::PADDING;
        assert_eq!(tab_width(40.0, 10.0), 2.0 * pad + 40.0 + component::tabs::COUNT_GAP + 10.0);
    }
}
