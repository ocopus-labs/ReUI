//! The places a surface may offer to switch between (the island's view switcher,
//! docs/PRD-view-switcher.md): views the user opens on purpose, as opposed to moments that
//! come to them. One table, because the order is a product decision across modules (like
//! `main.rs`'s module list) and a tab must never move while it's being used; each place
//! opens through the command its module already takes from the tray, so no module changes
//! to join. Pure, so which places exist when is testable without a hub.

use std::collections::HashSet;

use reui_protocol::{Activity, Destination};

/// When a place can be gone to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Available {
    /// Whenever its module runs: it has something to show at any time.
    Always,
    /// Only while the module has a card out (Now Playing, while a player has one).
    WithCard,
}

struct Place {
    module: &'static str,
    glyph: char,
    name: &'static str,
    open: &'static str,
    available: Available,
}

/// In tab order: the places that are always there first, so the one that comes and goes
/// (Now Playing) never shifts the others.
const PLACES: &[Place] = &[
    Place { module: "clipboard", glyph: '\u{E77F}', name: "Clipboard", open: "open", available: Available::Always },
    Place { module: "timers", glyph: '\u{E916}', name: "Timers", open: "launcher", available: Available::Always },
    Place { module: "devices", glyph: '\u{E772}', name: "Devices", open: "overview", available: Available::Always },
    Place { module: "stats", glyph: '\u{E9D9}', name: "System stats", open: "open", available: Available::Always },
    Place { module: "media", glyph: '\u{E8D6}', name: "Now Playing", open: "show", available: Available::WithCard },
];

/// The places there are now: those whose module is running (`modules`, by name), and of
/// those that need a card, the ones with an expanded card among `activities`.
pub fn available<'a>(modules: impl IntoIterator<Item = &'a str>, activities: &[&Activity]) -> Vec<Destination> {
    let running: HashSet<&str> = modules.into_iter().collect();
    PLACES
        .iter()
        .filter(|p| running.contains(p.module))
        .filter(|p| p.available == Available::Always || activities.iter().any(|a| a.id.module == p.module && a.expanded.is_some()))
        .map(|p| Destination { module: p.module.into(), glyph: p.glyph, name: p.name.into(), open: p.open.into() })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use reui_protocol::{ActivityId, CompactView, Element, ExpandedView, Headline, Priority};

    fn activity(module: &str, card: bool) -> Activity {
        let expanded = card.then(|| ExpandedView {
            context: None,
            leading: Element::Empty,
            eyebrow: None,
            headline: Headline::Text(String::new()),
            detail: None,
            trailing: Element::Empty,
            rows: Vec::new(),
            size: Default::default(),
            tint: false,
            pager: None,
            open: None,
        });
        Activity { id: ActivityId { module: module.into(), key: "k".into() }, priority: Priority::Ongoing, accent: None, compact: CompactView::default(), minimal: Element::Empty, expanded, audio: None, quiet: None }
    }

    fn names(list: &[Destination]) -> Vec<&str> {
        list.iter().map(|d| d.name.as_str()).collect()
    }

    #[test]
    fn only_running_modules_have_a_place_in_a_fixed_order() {
        let all = ["stats", "media", "osd", "clipboard", "claude", "timers", "devices"];
        assert_eq!(names(&available(all, &[])), ["Clipboard", "Timers", "Devices", "System stats"]);
        assert_eq!(names(&available(["stats", "clipboard"], &[])), ["Clipboard", "System stats"], "a module that's off has no tab");
        assert!(available(["osd", "claude", "privacy"], &[]).is_empty(), "moments are not places");
    }

    #[test]
    fn now_playing_is_a_place_only_while_it_has_a_card_and_comes_last() {
        let modules = ["media", "clipboard", "timers"];
        let playing = activity("media", true);
        assert_eq!(names(&available(modules, &[&playing])), ["Clipboard", "Timers", "Now Playing"]);
        let bare = activity("media", false);
        assert_eq!(names(&available(modules, &[&bare])), ["Clipboard", "Timers"]);
        assert_eq!(names(&available(["clipboard"], &[&playing])), ["Clipboard"], "a card from a module that isn't running counts for nothing");
        let open = &available(modules, &[&playing])[2];
        assert_eq!((open.module.as_str(), open.open.as_str()), ("media", "show"));
    }
}
