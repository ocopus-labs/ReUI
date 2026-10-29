//! A card that works like a panel (clipboard history): its scrolling row of cards, the
//! selection and the search text being typed (the keys it reads are `keys.rs`). Pure, so
//! the rules are testable without a window; `pill` drives it and `render` draws from it.

use std::collections::HashSet;

use reui_design::tokens::motion;
use reui_protocol::{ActivityId, ExpandedView, ImageId, Row, Snapshot};

/// The row of cards' geometry lives with the rest of the island's layout.
pub use reui_design::layout::cards::CardRow;

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

/// The width a view asks for (its `size`), in DIPs.
pub fn view_width(view: &ExpandedView) -> f32 {
    crate::render::card(view.size).width()
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
    /// selection into view (the keyboard is driving). `inset` is the panel's (the look's).
    pub fn sync(&mut self, id: Option<&ActivityId>, view: Option<&ExpandedView>, follow: bool, inset: f32) {
        if self.activity.as_ref() != id {
            self.reset();
            self.activity = id.cloned();
        }
        let new = content(view);
        let row = view.map(|v| CardRow::new(view_width(v), new.actions.len(), inset));
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
    use reui_design::tokens::island;
    use reui_protocol::{CardSize, CompactView, Element, Headline, Item, Preview, Tab};

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
                Row::Search { query: query.into(), buttons: vec![], placeholder: None, glyph: None },
                Row::Tabs(vec![Tab { label: tab.into(), count: 1, selected: true, action: tab.into() }]),
                Row::Items(actions.iter().map(|a| item(a)).collect()),
            ],
            context: None, size: CardSize::Wide,
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
        p.sync(Some(&id), Some(&view("", "all", &refs)), true, island::expanded::INSET);
        p.select(6);
        assert_eq!(p.selected_action(), Some("p6"));
        assert!(p.target > 0.0);

        // A new copy slides in at the start: the selection stays on the same item.
        let mut more = vec!["new"];
        more.extend(&refs);
        p.sync(Some(&id), Some(&view("", "all", &more)), true, island::expanded::INSET);
        assert_eq!((p.selected, p.selected_action()), (7, Some("p6")));

        // The selected item is deleted: its neighbour takes the place.
        let without: Vec<&str> = more.iter().copied().filter(|a| *a != "p6").collect();
        p.sync(Some(&id), Some(&view("", "all", &without)), true, island::expanded::INSET);
        assert_eq!(p.selected_action(), Some("p7"));

        // A search that drops it starts over at the first card.
        p.sync(Some(&id), Some(&view("fig", "all", &["p1", "p3"])), true, island::expanded::INSET);
        assert_eq!((p.selected, p.target), (0, 0.0));

        // Another activity in front starts from scratch.
        let other = ActivityId { module: "media".into(), key: "x".into() };
        p.sync(Some(&other), None, true, island::expanded::INSET);
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
