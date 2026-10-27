//! A card that works like a panel (clipboard history): its scrolling row of cards, the
//! selection, the search text being typed, and the keyboard map. Pure, so the rules are
//! testable without a window; `pill` drives it and `render` draws from it.

use std::collections::HashSet;

use reui_design::tokens::{component, island, motion};
use reui_protocol::{ActivityId, ExpandedView, ImageId, ItemKey, Row, Snapshot};

/// Layout of a row of `count` cards in a view `width` wide. Card `i`'s left edge is
/// `inset + i * step - scroll`; the row runs to the panel's right edge and is clipped there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CardRow {
    pub width: f32,
    pub count: usize,
}

impl CardRow {
    pub fn new(width: f32, count: usize) -> Self {
        Self { width, count }
    }

    /// Four across when they all fit; otherwise four and a half, the half card at the edge
    /// saying there's more.
    pub fn card_width(&self) -> f32 {
        let inset = island::expanded::INSET;
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
        island::expanded::INSET + i as f32 * self.step() - scroll
    }

    /// Scrolled all the way, the last card ends one inset from the edge.
    pub fn max_scroll(&self) -> f32 {
        let inset = island::expanded::INSET;
        let content = inset + self.count as f32 * self.step() - component::card::GAP + inset;
        (content - self.width).max(0.0)
    }

    pub fn clamp(&self, scroll: f32) -> f32 {
        scroll.clamp(0.0, self.max_scroll())
    }

    /// The nearest scroll that shows card `i` whole, inside the insets.
    pub fn reveal(&self, scroll: f32, i: usize) -> f32 {
        let inset = island::expanded::INSET;
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

/// One frame of the eased scroll: an exponential approach to `target`, snapped once it's
/// within a fraction of a pixel. Returns the new position and whether it's still moving.
pub fn ease(current: f32, target: f32, dt_ms: f32) -> (f32, bool) {
    let k = (-dt_ms / motion::scroll::TIME_CONSTANT as f32).exp();
    let next = target + (current - target) * k;
    if (next - target).abs() < 0.25 {
        (target, false)
    } else {
        (next, true)
    }
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

/// Keys a captured panel handles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Type(char),
    Backspace,
    BackspaceWord,
    Escape,
    Move(i32),
    First,
    Last,
    Activate,
    Item(ItemKey),
    /// Alt+1…9: the nth card in view.
    Nth(usize),
    /// Tab / Shift+Tab.
    Tab(i32),
    Undo,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

pub mod vk {
    pub const BACK: u16 = 0x08;
    pub const TAB: u16 = 0x09;
    pub const RETURN: u16 = 0x0D;
    pub const ESCAPE: u16 = 0x1B;
    pub const END: u16 = 0x23;
    pub const HOME: u16 = 0x24;
    pub const LEFT: u16 = 0x25;
    pub const UP: u16 = 0x26;
    pub const RIGHT: u16 = 0x27;
    pub const DOWN: u16 = 0x28;
    pub const DELETE: u16 = 0x2E;
    pub const KEY_1: u16 = 0x31;
    pub const KEY_9: u16 = 0x39;
    pub const KEY_P: u16 = 0x50;
    pub const KEY_Z: u16 = 0x5A;
    pub const NUMPAD1: u16 = 0x61;
    pub const NUMPAD9: u16 = 0x69;
}

/// The keyboard map. `text` gives the character the key types with the current layout and
/// modifiers (asked only when needed). `None` means "not ours": the key goes on to the app
/// in front. Ctrl+Alt together is AltGr, which types characters on many layouts.
pub fn map_key(key: u16, mods: Mods, text: impl FnOnce() -> Option<char>) -> Option<Key> {
    let Mods { shift, ctrl, alt } = mods;
    let bare = !ctrl && !alt;
    let ctrl_only = ctrl && !alt && !shift;
    Some(match key {
        vk::ESCAPE if bare => Key::Escape,
        vk::BACK if !alt => {
            if ctrl {
                Key::BackspaceWord
            } else {
                Key::Backspace
            }
        }
        vk::LEFT | vk::UP if bare => Key::Move(-1),
        vk::RIGHT | vk::DOWN if bare => Key::Move(1),
        vk::HOME if bare => Key::First,
        vk::END if bare => Key::Last,
        vk::RETURN if !alt => match (shift, ctrl) {
            (false, false) => Key::Activate,
            (true, false) => Key::Item(ItemKey::ShiftEnter),
            (false, true) => Key::Item(ItemKey::CtrlEnter),
            (true, true) => return None,
        },
        vk::DELETE if bare && !shift => Key::Item(ItemKey::Delete),
        vk::TAB if bare => Key::Tab(if shift { -1 } else { 1 }),
        vk::KEY_P if ctrl_only => Key::Item(ItemKey::CtrlP),
        vk::KEY_Z if ctrl_only => Key::Undo,
        vk::KEY_1..=vk::KEY_9 if alt && !ctrl => Key::Nth((key - vk::KEY_1) as usize),
        vk::NUMPAD1..=vk::NUMPAD9 if alt && !ctrl => Key::Nth((key - vk::NUMPAD1) as usize),
        _ if ctrl != alt => return None,
        _ => Key::Type(text().filter(|c| !c.is_control())?),
    })
}

/// Ctrl+Backspace: drops trailing spaces, then the word before them.
pub fn delete_word(text: &mut String) {
    let trimmed = text.trim_end_matches(char::is_whitespace).len();
    text.truncate(trimmed);
    let word = text.trim_end_matches(|c: char| !c.is_whitespace()).len();
    text.truncate(word);
}

/// Every image a snapshot shows. Both ends of the pipe keep exactly these after each
/// snapshot (the hub what it has sent, the island what it has decoded), so an image that
/// comes back later is simply sent again.
pub fn referenced(snapshot: &Snapshot) -> HashSet<ImageId> {
    snapshot.activities.iter().flat_map(|a| a.images()).collect()
}

/// What the front card's rows say, as far as the panel cares.
#[derive(Default, Debug, PartialEq)]
struct Content {
    actions: Vec<String>,
    /// The module's query and selected tab: when they change, the list is a new one.
    filter: (Option<String>, Option<String>),
}

fn content(view: Option<&ExpandedView>) -> Content {
    let mut c = Content::default();
    for row in view.map(|v| v.rows.as_slice()).unwrap_or_default() {
        match row {
            Row::Items(items) => c.actions = items.iter().map(|i| i.action.clone()).collect(),
            Row::Search { query, .. } => c.filter.0 = Some(query.clone()),
            Row::Tabs(tabs) => c.filter.1 = tabs.iter().find(|t| t.selected).map(|t| t.action.clone()),
            _ => {}
        }
    }
    c
}

pub fn view_width(view: &ExpandedView) -> f32 {
    if view.wide {
        island::expanded::WIDE_WIDTH
    } else {
        island::expanded::WIDTH
    }
}

/// The open panel's interaction state. Reset when the panel closes or another activity
/// comes to the front; kept (clamped) across updates of the same activity.
#[derive(Default, Debug)]
pub struct Panel {
    activity: Option<ActivityId>,
    content: Content,
    row: Option<CardRow>,
    pub selected: usize,
    /// Scroll position drawn now, and where it's easing to.
    pub scroll: f32,
    pub target: f32,
    /// The text in the search field while the panel has the keyboard; the module's query
    /// otherwise. Shown at once; the module hears it debounced.
    pub query: Option<String>,
}

impl Panel {
    pub fn reset(&mut self) {
        *self = Panel::default();
    }

    pub fn row(&self) -> Option<CardRow> {
        self.row.filter(|r| r.count > 0)
    }

    pub fn selected_action(&self) -> Option<&str> {
        self.content.actions.get(self.selected).map(String::as_str)
    }

    /// The module's own query, as of the latest snapshot.
    pub fn module_query(&self) -> Option<&str> {
        self.content.filter.0.as_deref()
    }

    /// A new snapshot for the front activity. The selection follows its item (by action)
    /// when the list changes; a new search or tab starts over at the first card; an item
    /// that went away (deleted) leaves the selection on its neighbour. `follow` scrolls the
    /// selection into view (the keyboard is driving).
    pub fn sync(&mut self, id: Option<&ActivityId>, view: Option<&ExpandedView>, follow: bool) {
        if self.activity.as_ref() != id {
            self.reset();
            self.activity = id.cloned();
        }
        let new = content(view);
        let row = view.map(|v| CardRow::new(view_width(v), new.actions.len()));
        let was = self.selected_action().map(str::to_owned);
        let filter_changed = new.filter != self.content.filter;
        self.selected = match was.and_then(|a| new.actions.iter().position(|n| *n == a)) {
            Some(i) => i,
            None if filter_changed => 0,
            None => self.selected.min(new.actions.len().saturating_sub(1)),
        };
        if filter_changed {
            self.scroll = 0.0;
            self.target = 0.0;
        }
        self.content = new;
        self.row = row;
        if let Some(row) = self.row() {
            self.scroll = row.clamp(self.scroll);
            self.target = if follow { row.reveal(row.clamp(self.target), self.selected) } else { row.clamp(self.target) };
        } else {
            (self.scroll, self.target) = (0.0, 0.0);
        }
    }

    /// Moves the selection to `i` (clamped) and eases the row to show it.
    pub fn select(&mut self, i: usize) {
        let Some(row) = self.row() else { return };
        self.selected = i.min(row.count - 1);
        self.target = row.reveal(self.target, self.selected);
    }

    pub fn wheel(&mut self, delta: i32) {
        if let Some(row) = self.row() {
            self.target = row.wheel(self.target, delta);
        }
    }

    /// One animation frame; `true` while still moving.
    pub fn tick(&mut self, dt_ms: f32) -> bool {
        let (scroll, moving) = ease(self.scroll, self.target, dt_ms);
        self.scroll = scroll;
        moving
    }

    pub fn moving(&self) -> bool {
        self.scroll != self.target
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reui_protocol::{CompactView, Element, Headline, Item, Preview, Tab};

    const W: f32 = 560.0;

    #[test]
    fn four_fit_and_more_show_a_half_card() {
        let four = CardRow::new(W, 4);
        assert_eq!(four.max_scroll(), 0.0);
        let right = four.left(3, 0.0) + four.card_width();
        assert!((right - (W - island::expanded::INSET)).abs() < 0.01);

        let many = CardRow::new(W, 10);
        // Four whole cards, then half of the fifth cut by the panel's edge.
        let fifth = many.left(4, 0.0);
        assert!((W - fifth - many.card_width() / 2.0).abs() < 0.01);
        assert_eq!(many.visible(0.0), 0..5);
    }

    #[test]
    fn scrolling_clamps_and_steps_a_card_per_notch() {
        let row = CardRow::new(W, 10);
        assert_eq!(row.wheel(0.0, 120), 0.0);
        assert!((row.wheel(0.0, -120) - row.step()).abs() < 0.01);
        assert_eq!(row.wheel(0.0, -120 * 50), row.max_scroll());
        // At the end, the last card ends one inset from the edge.
        let last = row.left(9, row.max_scroll()) + row.card_width();
        assert!((last - (W - island::expanded::INSET)).abs() < 0.01);
    }

    #[test]
    fn revealing_a_card_scrolls_just_enough() {
        let row = CardRow::new(W, 10);
        assert_eq!(row.reveal(0.0, 2), 0.0);
        let s = row.reveal(0.0, 4);
        assert!((row.left(4, s) + row.card_width() - (W - island::expanded::INSET)).abs() < 0.01);
        assert_eq!(row.reveal(s, 1), s, "already in view");
        assert!((row.left(1, row.reveal(row.max_scroll(), 1)) - island::expanded::INSET).abs() < 0.01);
        assert_eq!(row.first_shown(row.step() * 3.0), 3);
    }

    #[test]
    fn easing_settles_exactly_on_the_target() {
        let (mut x, mut moving) = (0.0, true);
        let mut frames = 0;
        while moving {
            (x, moving) = ease(x, 100.0, 16.0);
            frames += 1;
        }
        assert_eq!(x, 100.0);
        assert!(frames < 30, "{frames}");
    }

    #[test]
    fn tabs_scroll_only_when_too_wide() {
        assert_eq!(tabs_offset(&[50.0, 50.0], 6.0, Some(1), 200.0), 0.0);
        assert_eq!(tabs_offset(&[100.0, 100.0, 100.0], 0.0, Some(2), 200.0), 100.0);
        assert_eq!(tabs_offset(&[100.0, 100.0, 100.0], 0.0, Some(0), 200.0), 0.0);
    }

    #[test]
    fn key_map() {
        let none = Mods::default();
        let shift = Mods { shift: true, ..none };
        let ctrl = Mods { ctrl: true, ..none };
        let alt = Mods { alt: true, ..none };
        let altgr = Mods { ctrl: true, alt: true, ..none };
        let a = || Some('a');
        assert_eq!(map_key(0x41, none, a), Some(Key::Type('a')));
        assert_eq!(map_key(0x41, ctrl, a), None, "Ctrl+A goes to the app");
        assert_eq!(map_key(0x51, altgr, || Some('@')), Some(Key::Type('@')));
        assert_eq!(map_key(0x41, none, || None), None, "dead keys type nothing");
        assert_eq!(map_key(0x41, none, || Some('\u{1}')), None);
        assert_eq!(map_key(vk::BACK, none, a), Some(Key::Backspace));
        assert_eq!(map_key(vk::BACK, ctrl, a), Some(Key::BackspaceWord));
        assert_eq!(map_key(vk::ESCAPE, none, a), Some(Key::Escape));
        assert_eq!(map_key(vk::LEFT, none, a), Some(Key::Move(-1)));
        assert_eq!(map_key(vk::DOWN, none, a), Some(Key::Move(1)));
        assert_eq!(map_key(vk::HOME, none, a), Some(Key::First));
        assert_eq!(map_key(vk::END, none, a), Some(Key::Last));
        assert_eq!(map_key(vk::RETURN, none, a), Some(Key::Activate));
        assert_eq!(map_key(vk::RETURN, shift, a), Some(Key::Item(ItemKey::ShiftEnter)));
        assert_eq!(map_key(vk::RETURN, ctrl, a), Some(Key::Item(ItemKey::CtrlEnter)));
        assert_eq!(map_key(vk::DELETE, none, a), Some(Key::Item(ItemKey::Delete)));
        assert_eq!(map_key(vk::KEY_P, ctrl, a), Some(Key::Item(ItemKey::CtrlP)));
        assert_eq!(map_key(vk::KEY_Z, ctrl, a), Some(Key::Undo));
        assert_eq!(map_key(vk::TAB, none, a), Some(Key::Tab(1)));
        assert_eq!(map_key(vk::TAB, shift, a), Some(Key::Tab(-1)));
        assert_eq!(map_key(0x33, alt, a), Some(Key::Nth(2)));
        assert_eq!(map_key(vk::NUMPAD1, alt, a), Some(Key::Nth(0)));
        assert_eq!(map_key(0x46, alt, a), None, "Alt+F goes to the app");
        assert_eq!(map_key(0x20, none, || Some(' ')), Some(Key::Type(' ')));
    }

    #[test]
    fn ctrl_backspace_deletes_a_word() {
        let mut s = String::from("figma file  ");
        delete_word(&mut s);
        assert_eq!(s, "figma ");
        delete_word(&mut s);
        assert_eq!(s, "");
        delete_word(&mut s);
        assert_eq!(s, "");
    }

    fn item(action: &str) -> Item {
        Item { action: action.into(), preview: Preview::Text(action.into()), icon: None, age: "1m".into(), meta: None, pinned: false, actions: vec![] }
    }

    fn view(query: &str, tab: &str, actions: &[&str]) -> ExpandedView {
        ExpandedView {
            leading: Element::Empty,
            eyebrow: None,
            headline: Headline::Text(String::new()),
            detail: None,
            trailing: Element::Empty,
            rows: vec![
                Row::Search { query: query.into(), buttons: vec![] },
                Row::Tabs(vec![Tab { label: tab.into(), count: 1, selected: true, action: tab.into() }]),
                Row::Items(actions.iter().map(|a| item(a)).collect()),
            ],
            wide: true,
            tint: false,
            pager: None,
            open: None,
        }
    }

    #[test]
    fn selection_follows_its_item_and_resets_on_a_new_filter() {
        let id = ActivityId { module: "clipboard".into(), key: "clipboard".into() };
        let mut p = Panel::default();
        let actions: Vec<String> = (0..10).map(|i| format!("p{i}")).collect();
        let refs: Vec<&str> = actions.iter().map(String::as_str).collect();
        p.sync(Some(&id), Some(&view("", "all", &refs)), true);
        p.select(6);
        assert_eq!(p.selected_action(), Some("p6"));
        assert!(p.target > 0.0);

        // A new copy slides in at the start: the selection stays on the same item.
        let mut more = vec!["new"];
        more.extend(&refs);
        p.sync(Some(&id), Some(&view("", "all", &more)), true);
        assert_eq!((p.selected, p.selected_action()), (7, Some("p6")));

        // The selected item is deleted: its neighbour takes the place.
        let without: Vec<&str> = more.iter().copied().filter(|a| *a != "p6").collect();
        p.sync(Some(&id), Some(&view("", "all", &without)), true);
        assert_eq!(p.selected_action(), Some("p7"));

        // A search that drops it starts over at the first card.
        p.sync(Some(&id), Some(&view("fig", "all", &["p1", "p3"])), true);
        assert_eq!((p.selected, p.target), (0, 0.0));

        // Another activity in front starts from scratch.
        let other = ActivityId { module: "media".into(), key: "x".into() };
        p.sync(Some(&other), None, true);
        assert_eq!((p.selected, p.scroll, p.row()), (0, 0.0, None));
    }

    #[test]
    fn eviction_keeps_exactly_what_the_snapshot_shows() {
        use reui_protocol::{Activity, Art, Priority};
        let with = |ids: &[u64]| Snapshot {
            activities: ids
                .iter()
                .map(|&i| Activity {
                    id: ActivityId { module: "m".into(), key: i.to_string() },
                    priority: Priority::Ongoing,
                    accent: None,
                    compact: CompactView { leading: Element::Art(Art::Image(ImageId(i))), trailing: Element::Empty },
                    minimal: Element::Empty,
                    audio: None,
                    expanded: None,
                })
                .collect(),
        };
        assert_eq!(referenced(&with(&[1, 2])), HashSet::from([ImageId(1), ImageId(2)]));
        assert!(referenced(&with(&[])).is_empty());
    }
}
