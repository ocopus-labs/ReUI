//! What the island shows where: which activity is in the pill and which in the minimal
//! bubble. Pure, so the rules are testable without a window.

use reui_protocol::{Activity, ActivityId, Priority};

/// Orders activities for display, front first, from the hub's priority order:
///
/// 1. The activity the user brought forward (`pinned`) goes first, unless something more
///    urgent (above both it and "ongoing") is waiting.
/// 2. The bubble (second place) shows a different kind of activity from the pill when there
///    is one, so two Claude sessions don't push the music out of sight, and vice versa.
///
/// Returns the order and whether the pin still refers to a live activity.
pub fn arrange(mut activities: Vec<Activity>, pinned: Option<&ActivityId>) -> (Vec<Activity>, bool) {
    let mut pin_alive = false;
    if let Some(pin) = pinned {
        if let Some(i) = activities.iter().position(|a| &a.id == pin) {
            pin_alive = true;
            let level = activities[i].priority.max(Priority::Ongoing);
            if !activities.iter().any(|a| a.priority > level) {
                let front = activities.remove(i);
                activities.insert(0, front);
            }
        }
    }
    if activities.len() > 2 && activities[1].id.module == activities[0].id.module {
        let front_module = activities[0].id.module.clone();
        if let Some(i) = activities.iter().skip(2).position(|a| a.id.module != front_module) {
            let other = activities.remove(i + 2);
            activities.insert(1, other);
        }
    }
    (activities, pin_alive)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reui_protocol::{CompactView, Element};

    fn activity(module: &str, key: &str, priority: Priority) -> Activity {
        Activity {
            id: ActivityId { module: module.into(), key: key.into() },
            priority,
            accent: None,
            compact: CompactView::default(),
            minimal: Element::Empty,
            expanded: None,
        }
    }

    fn keys(activities: &[Activity]) -> Vec<&str> {
        activities.iter().map(|a| a.id.key.as_str()).collect()
    }

    #[test]
    fn bubble_shows_a_different_kind_of_activity() {
        let hub_order = vec![
            activity("claude", "session-a", Priority::Ongoing),
            activity("claude", "session-b", Priority::Ongoing),
            activity("media", "now-playing", Priority::Ongoing),
        ];
        let (shown, _) = arrange(hub_order, None);
        assert_eq!(keys(&shown), ["session-a", "now-playing", "session-b"]);
    }

    #[test]
    fn music_in_front_puts_claude_in_the_bubble() {
        let hub_order = vec![
            activity("claude", "session-a", Priority::Ongoing),
            activity("claude", "session-b", Priority::Ongoing),
            activity("media", "now-playing", Priority::Ongoing),
        ];
        let pin = ActivityId { module: "media".into(), key: "now-playing".into() };
        let (shown, alive) = arrange(hub_order, Some(&pin));
        assert!(alive);
        assert_eq!(keys(&shown), ["now-playing", "session-a", "session-b"]);
    }

    #[test]
    fn something_urgent_beats_the_pin_and_the_pin_comes_back_after() {
        let pin = ActivityId { module: "media".into(), key: "now-playing".into() };
        let with_request = vec![
            activity("claude", "permission-1", Priority::Decision),
            activity("claude", "session-a", Priority::Ongoing),
            activity("media", "now-playing", Priority::Ongoing),
        ];
        let (shown, _) = arrange(with_request, Some(&pin));
        assert_eq!(keys(&shown), ["permission-1", "now-playing", "session-a"]);

        let answered = vec![activity("claude", "session-a", Priority::Ongoing), activity("media", "now-playing", Priority::Ongoing)];
        let (shown, _) = arrange(answered, Some(&pin));
        assert_eq!(keys(&shown), ["now-playing", "session-a"]);
    }

    #[test]
    fn a_pin_to_something_gone_is_dropped() {
        let pin = ActivityId { module: "media".into(), key: "now-playing".into() };
        let (shown, alive) = arrange(vec![activity("claude", "session-a", Priority::Ongoing)], Some(&pin));
        assert!(!alive);
        assert_eq!(keys(&shown), ["session-a"]);
    }

    #[test]
    fn same_kind_only_keeps_hub_order() {
        let hub_order = vec![
            activity("claude", "a", Priority::Ongoing),
            activity("claude", "b", Priority::Ongoing),
            activity("claude", "c", Priority::Ongoing),
        ];
        assert_eq!(keys(&arrange(hub_order, None).0), ["a", "b", "c"]);
    }
}
