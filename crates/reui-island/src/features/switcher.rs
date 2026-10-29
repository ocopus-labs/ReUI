//! When the view switcher shows and which tab is selected (docs/PRD-view-switcher.md §5.2).
//! Pure, so the rules are testable without a window; the pill draws and places it.

use reui_protocol::{Activity, Destination, Priority};

/// What the switcher is deciding about: the island's state and the card that's open.
#[derive(Clone, Copy, Debug)]
pub struct Open<'a> {
    /// The Settings toggle (`view_switcher`).
    pub enabled: bool,
    /// The island is expanded and on screen.
    pub expanded: bool,
    /// The open card's module and priority.
    pub module: &'a str,
    pub priority: Priority,
    /// It was opened on request (a hotkey, the tray, a tab).
    pub summoned: bool,
}

/// The selected tab when it shows, or `None` when it doesn't: an open card of a place
/// (Passive or Ongoing, or summoned; never a moment, which comes to you), with at least two
/// places to switch between.
pub fn selected(places: &[Destination], open: Option<Open<'_>>) -> Option<usize> {
    let open = open?;
    if !open.enabled || !open.expanded || places.len() < 2 {
        return None;
    }
    if !open.summoned && open.priority > Priority::Ongoing {
        return None;
    }
    places.iter().take(reui_design::layout::switcher::MAX_TABS).position(|p| p.module == open.module)
}

/// The decision waiting behind the open place while the switcher shows (PRD M2): the
/// first one after the front in display order. Only decisions: they're what can't go on
/// without you (an alert says something happened and keeps), and they're what a summoned
/// place holds back (`order::arrange`).
pub fn waiting(shown: &[Activity], switcher: bool) -> Option<&Activity> {
    if !switcher {
        return None;
    }
    shown.iter().skip(1).find(|a| a.priority == Priority::Decision)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reui_protocol::{ActivityId, CompactView, Element};

    fn activity(module: &str, priority: Priority) -> Activity {
        Activity { id: ActivityId { module: module.into(), key: "k".into() }, priority, accent: None, compact: CompactView::default(), minimal: Element::Empty, audio: None, expanded: None, quiet: None }
    }

    #[test]
    fn a_decision_behind_the_place_waits_on_the_switcher() {
        let shown = [activity("clipboard", Priority::Passive), activity("media", Priority::Ongoing), activity("claude", Priority::Decision)];
        assert_eq!(waiting(&shown, true).map(|a| a.id.module.as_str()), Some("claude"));
        assert!(waiting(&shown, false).is_none(), "only while the switcher shows");
        let alert = [activity("clipboard", Priority::Passive), activity("timers", Priority::Alert)];
        assert!(waiting(&alert, true).is_none(), "alerts keep");
        let front = [activity("claude", Priority::Decision)];
        assert!(waiting(&front, true).is_none(), "the one in front isn't waiting");
    }

    fn places(modules: &[&str]) -> Vec<Destination> {
        modules.iter().map(|m| Destination { module: (*m).into(), glyph: 'x', name: (*m).into(), open: "open".into() }).collect()
    }

    fn open(module: &str, priority: Priority) -> Option<Open<'_>> {
        Some(Open { enabled: true, expanded: true, module, priority, summoned: false })
    }

    #[test]
    fn a_place_shows_its_tab_selected() {
        let list = places(&["clipboard", "timers", "stats"]);
        assert_eq!(selected(&list, open("timers", Priority::Ongoing)), Some(1), "a running timer is Timers");
        assert_eq!(selected(&list, open("stats", Priority::Passive)), Some(2));
    }

    #[test]
    fn moments_and_other_cards_show_none() {
        let list = places(&["clipboard", "timers", "media"]);
        assert_eq!(selected(&list, open("timers", Priority::Decision)), None, "a finished timer is a moment");
        assert_eq!(selected(&list, open("claude", Priority::Decision)), None);
        assert_eq!(selected(&list, open("devices", Priority::Passive)), None, "not a place on this list");
        assert_eq!(selected(&list, open("clipboard", Priority::Brief)), None);
        let summoned = Some(Open { summoned: true, ..open("clipboard", Priority::Brief).unwrap() });
        assert_eq!(selected(&list, summoned), Some(0), "asked for, it's a place");
    }

    #[test]
    fn never_compact_off_or_alone() {
        let list = places(&["clipboard", "timers"]);
        assert_eq!(selected(&list, None), None);
        assert_eq!(selected(&list, Some(Open { expanded: false, ..open("clipboard", Priority::Passive).unwrap() })), None);
        assert_eq!(selected(&list, Some(Open { enabled: false, ..open("clipboard", Priority::Passive).unwrap() })), None);
        assert_eq!(selected(&places(&["clipboard"]), open("clipboard", Priority::Passive)), None, "one tab switches nothing");
    }
}
