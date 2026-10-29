//! The view switcher: a capsule of glyph tabs under an open place's card
//! (docs/PRD-view-switcher.md). As tall as the island, like the minimal bubble, so it reads
//! as the island's own; its tabs are circles one inset in, concentric with its ends.
//! Positions are from the switcher's own top-left; the island centres it under the card.

use crate::look::{concentric, Look};
use crate::tokens::island;

/// Most tabs the switcher is drawn for (its surface's width). The hub offers five places;
/// a longer list shows its first nine, which is also as far as anyone counts at a glance.
pub const MAX_TABS: usize = 9;

/// Its height: the island's.
pub const HEIGHT: f32 = island::HEIGHT;

/// A tab's circle across.
pub const TAB: f32 = HEIGHT - 2.0 * island::switcher::INSET;

/// The surface the island draws it into: wide enough for [`MAX_TABS`].
pub const SURFACE: (f32, f32) = (width(MAX_TABS), HEIGHT);

/// Room it takes under the tallest card (the island's window is that much taller).
pub const ROOM: f32 = island::switcher::GAP + HEIGHT;

/// Width for `count` tabs.
pub const fn width(count: usize) -> f32 {
    let n = if count > MAX_TABS { MAX_TABS } else { count };
    if n == 0 {
        return 0.0;
    }
    n as f32 * TAB + (n - 1) as f32 * island::switcher::TAB_GAP + 2.0 * island::switcher::INSET
}

/// Where the switcher and its tabs go, for `count` tabs in `look`.
#[derive(Clone, Debug, PartialEq)]
pub struct Switcher {
    pub width: f32,
    pub height: f32,
    /// Its corner radius: a capsule at full roundness, squarer with the island.
    pub radius: f32,
    /// Each tab's left edge (its top is the inset).
    pub tabs: Vec<f32>,
    /// A tab's corner radius, concentric with the switcher's.
    pub tab_radius: f32,
}

impl Switcher {
    pub fn new(look: &Look, count: usize) -> Self {
        let inset = island::switcher::INSET;
        let n = count.min(MAX_TABS);
        let tabs = (0..n).map(|i| inset + i as f32 * (TAB + island::switcher::TAB_GAP)).collect();
        let radius = look.rounded(HEIGHT);
        Switcher { width: width(n), height: HEIGHT, radius, tabs, tab_radius: concentric(radius, inset).min(TAB / 2.0) }
    }

    /// The tab a point (switcher DIPs) is on: the whole height, and each tab's share of the
    /// gaps beside it, so the target is bigger than the circle drawn.
    pub fn tab_at(&self, x: f32, y: f32) -> Option<usize> {
        if !(0.0..self.height).contains(&y) || !(0.0..self.width).contains(&x) {
            return None;
        }
        let half_gap = island::switcher::TAB_GAP / 2.0;
        self.tabs.iter().position(|&left| x >= left - half_gap && x < left + TAB + half_gap).or_else(|| {
            // The end insets belong to the end tabs.
            if x < island::switcher::INSET { Some(0) } else { Some(self.tabs.len().saturating_sub(1)) }
        })
    }

    /// The switcher's top, below a card `card_height` tall whose top is `card_top`.
    pub fn top(card_top: f32, card_height: f32) -> f32 {
        card_top + card_height + island::switcher::GAP
    }
}

/// A decision waiting behind the open place (docs/PRD-view-switcher.md M2): a bubble the
/// switcher's height beside its end, holding the decision's accent dot. Detached rather
/// than a tab, so the tabs never shift under the pointer as it comes and goes, and a
/// moment never reads as one of the places.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Waiting {
    /// Its left edge, from the switcher's left.
    pub left: f32,
    pub size: f32,
    /// Its corner radius: the switcher's, so the two square off together.
    pub radius: f32,
    /// The hover fill inside it, one inset in, and its radius.
    pub inner: f32,
    pub inner_radius: f32,
}

impl Switcher {
    /// The waiting bubble beside this switcher.
    pub fn waiting(&self) -> Waiting {
        let inset = island::switcher::INSET;
        Waiting { left: self.width + island::switcher::WAITING_GAP, size: HEIGHT, radius: self.radius, inner: TAB, inner_radius: concentric(self.radius, inset).min(TAB / 2.0) }
    }

    /// Whether a point (switcher DIPs) is on the waiting bubble.
    pub fn on_waiting(&self, x: f32, y: f32) -> bool {
        let w = self.waiting();
        (0.0..self.height).contains(&y) && (w.left..w.left + w.size).contains(&x)
    }
}

/// The next place `step` tabs on from `current` (Ctrl+Tab, the wheel), wrapping; from no
/// current place, the first or the last.
pub fn step(count: usize, current: Option<usize>, step: i32) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let n = count as i32;
    Some(match current {
        Some(at) => (at as i32 + step).rem_euclid(n) as usize,
        None if step >= 0 => 0,
        None => count - 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::Appearance;

    fn look(roundness: f32) -> Look {
        Look::new(&Appearance { roundness, ..Appearance::default() }, false, None)
    }

    #[test]
    fn tabs_sit_one_inset_in_and_concentric_with_the_capsule() {
        let s = Switcher::new(&look(1.0), 4);
        let inset = island::switcher::INSET;
        assert_eq!(s.height, island::HEIGHT);
        assert!((s.radius - island::HEIGHT / 2.0).abs() < 1e-4, "a capsule at full roundness");
        assert!((s.tab_radius - TAB / 2.0).abs() < 1e-4, "circles inside it");
        assert_eq!(s.tabs[0], inset);
        assert!((s.width - (s.tabs[3] + TAB + inset)).abs() < 1e-4, "the last tab ends one inset from the edge");
        assert!((s.width - width(4)).abs() < 1e-4);
        // Square-ish looks square both off together, never under the minimum.
        let square = Switcher::new(&look(0.0), 4);
        assert!(square.radius < s.radius && square.tab_radius < TAB / 2.0);
        assert!((square.tab_radius - concentric(square.radius, inset)).abs() < 1e-4);
    }

    #[test]
    fn a_tab_takes_clicks_beyond_its_circle() {
        let s = Switcher::new(&look(1.0), 3);
        let mid = |i: usize| s.tabs[i] + TAB / 2.0;
        assert_eq!(s.tab_at(mid(1), 1.0), Some(1));
        assert_eq!(s.tab_at(s.tabs[1] - 1.0, s.height / 2.0), Some(1), "half the gap before it");
        assert_eq!(s.tab_at(1.0, s.height / 2.0), Some(0), "the end inset is the first tab's");
        assert_eq!(s.tab_at(s.width - 1.0, s.height / 2.0), Some(2));
        assert_eq!(s.tab_at(mid(0), s.height + 1.0), None);
        assert_eq!(s.tab_at(s.width + 1.0, 4.0), None);
    }

    #[test]
    fn more_than_nine_places_show_the_first_nine() {
        assert_eq!(Switcher::new(&look(1.0), 12).tabs.len(), MAX_TABS);
        assert!((width(12) - SURFACE.0).abs() < 1e-4);
        assert_eq!(width(0), 0.0);
    }

    #[test]
    fn a_waiting_decision_sits_beside_the_switcher_not_in_it() {
        let s = Switcher::new(&look(1.0), 4);
        let w = s.waiting();
        assert_eq!(w.left, s.width + island::switcher::WAITING_GAP);
        assert_eq!(w.size, s.height);
        assert_eq!(w.radius, s.radius, "the same shape as the switcher's ends");
        assert!((w.inner_radius - s.tab_radius).abs() < 1e-4);
        assert!(s.on_waiting(w.left + 1.0, 1.0) && !s.on_waiting(s.width - 1.0, 1.0));
        assert_eq!(s.tab_at(w.left + 1.0, 1.0), None, "never a tab");
    }

    #[test]
    fn stepping_wraps_and_starts_from_an_end() {
        assert_eq!(step(4, Some(3), 1), Some(0));
        assert_eq!(step(4, Some(0), -1), Some(3));
        assert_eq!(step(4, None, 1), Some(0));
        assert_eq!(step(4, None, -1), Some(3));
        assert_eq!(step(0, None, 1), None);
        assert!(Switcher::top(11.0, 84.0) > 95.0);
    }
}
