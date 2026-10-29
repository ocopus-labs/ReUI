//! What the island shows where: which activity is in the pill and which in the minimal
//! bubble. Pure, so the rules are testable without a window.

use reui_protocol::{Activity, ActivityId, Priority};

/// Orders activities for display, front first, from the hub's priority order:
///
/// 1. The activity the user brought forward (`pinned`) goes first, unless something more
///    urgent (above both it and "ongoing") is waiting. A `summoned` pin (opened with a
///    hotkey, so the user asked for exactly this) goes first regardless.
/// 2. A card the user has open (`open`) stays in front of anything but a decision: an
///    alert from another activity waits in the bubble until the card closes. A card just
///    summoned over it (another hotkey, a view switcher tab) replaces it instead.
/// 3. The bubble (second place) shows a different kind of activity from the pill when there
///    is one, so two Claude sessions don't push the music out of sight, and vice versa.
/// 4. A brief confirmation (a copy) never takes the pill from something else: it shows in
///    the bubble for its moment, unless the user brought it forward themselves, or
///    `briefs_lead`: in "When needed" mode with nothing urgent, the rest isn't on screen
///    anyway, so the confirmation (volume, a copy, a device) takes the pill.
///
/// Returns the order and whether the pin still refers to a live activity.
pub fn arrange(mut activities: Vec<Activity>, pinned: Option<&ActivityId>, summoned: bool, open: Option<&ActivityId>, briefs_lead: bool) -> (Vec<Activity>, bool) {
    let mut pin_alive = false;
    if let Some(pin) = pinned {
        if let Some(i) = activities.iter().position(|a| &a.id == pin) {
            pin_alive = true;
            let level = activities[i].priority.max(Priority::Ongoing);
            if summoned || !activities.iter().any(|a| a.priority > level) {
                let front = activities.remove(i);
                activities.insert(0, front);
            }
        }
    }
    if let (Some(open), Some(front)) = (open, activities.first()) {
        let asked_for = summoned && pinned == Some(&front.id);
        if &front.id != open && front.priority < Priority::Decision && !asked_for {
            if let Some(i) = activities.iter().position(|a| &a.id == open) {
                let kept = activities.remove(i);
                activities.insert(0, kept);
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
    let chosen = |a: &Activity| pinned == Some(&a.id) || open == Some(&a.id);
    if activities.len() > 1 && !briefs_lead {
        if let Some(i) = activities.iter().position(|a| a.priority == Priority::Brief && !chosen(a)) {
            if i != 1 {
                let brief = activities.remove(i);
                activities.insert(1, brief);
            }
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
            audio: None,
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
        let (shown, _) = arrange(hub_order, None, false, None, false);
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
        let (shown, alive) = arrange(hub_order, Some(&pin), false, None, false);
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
        let (shown, _) = arrange(with_request, Some(&pin), false, None, false);
        assert_eq!(keys(&shown), ["permission-1", "now-playing", "session-a"]);

        let answered = vec![activity("claude", "session-a", Priority::Ongoing), activity("media", "now-playing", Priority::Ongoing)];
        let (shown, _) = arrange(answered, Some(&pin), false, None, false);
        assert_eq!(keys(&shown), ["now-playing", "session-a"]);
    }

    #[test]
    fn a_pin_to_something_gone_is_dropped() {
        let pin = ActivityId { module: "media".into(), key: "now-playing".into() };
        let (shown, alive) = arrange(vec![activity("claude", "session-a", Priority::Ongoing)], Some(&pin), false, None, false);
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
        assert_eq!(keys(&arrange(hub_order, None, false, None, false).0), ["a", "b", "c"]);
    }

    #[test]
    fn an_alert_waits_while_another_card_is_open_but_a_decision_does_not() {
        let open = activity("media", "now-playing", Priority::Ongoing).id;
        let alert = vec![activity("claude", "done", Priority::Alert), activity("media", "now-playing", Priority::Ongoing)];
        assert_eq!(keys(&arrange(alert.clone(), None, false, Some(&open), false).0), ["now-playing", "done"]);
        assert_eq!(keys(&arrange(alert, None, false, None, false).0), ["done", "now-playing"]);
        let decision = vec![activity("claude", "ask", Priority::Decision), activity("media", "now-playing", Priority::Ongoing)];
        assert_eq!(keys(&arrange(decision, None, false, Some(&open), false).0), ["ask", "now-playing"]);
    }

    #[test]
    fn a_summoned_card_beats_an_alert() {
        let pin = activity("clipboard", "clipboard", Priority::Passive).id;
        let hub_order = vec![activity("claude", "session-a", Priority::Decision), activity("clipboard", "clipboard", Priority::Passive)];
        assert_eq!(keys(&arrange(hub_order.clone(), Some(&pin), true, None, false).0), ["clipboard", "session-a"]);
        assert_eq!(keys(&arrange(hub_order, Some(&pin), false, None, false).0), ["session-a", "clipboard"]);
    }

    #[test]
    fn a_card_summoned_over_an_open_one_replaces_it() {
        let open = activity("stats", "system", Priority::Passive).id;
        let launcher = activity("timers", "launcher", Priority::Passive);
        let hub_order = vec![launcher.clone(), activity("stats", "system", Priority::Passive)];
        assert_eq!(keys(&arrange(hub_order.clone(), Some(&launcher.id), true, Some(&open), false).0), ["launcher", "system"]);
        // Brought forward without being asked for (the wheel), the open card stays.
        assert_eq!(keys(&arrange(hub_order, Some(&launcher.id), false, Some(&open), false).0), ["system", "launcher"]);
    }

    #[test]
    fn in_when_needed_mode_a_brief_takes_the_pill() {
        let music = activity("media", "now-playing", Priority::Ongoing);
        let volume = activity("osd", "volume", Priority::Brief);
        let (shown, _) = arrange(vec![volume.clone(), music.clone()], None, false, None, true);
        assert_eq!(keys(&shown), ["volume", "now-playing"]);
    }

    #[test]
    fn a_brief_confirmation_waits_in_the_bubble() {
        let music = activity("media", "now-playing", Priority::Ongoing);
        let claude = activity("claude", "session-a", Priority::Ongoing);
        let copied = activity("clipboard", "clipboard", Priority::Brief);
        // The hub ranks it first; the island keeps the music in the pill.
        let (shown, _) = arrange(vec![copied.clone(), music.clone(), claude.clone()], None, false, None, false);
        assert_eq!(keys(&shown), ["now-playing", "clipboard", "session-a"]);
        // Alone, it takes the pill.
        assert_eq!(keys(&arrange(vec![copied.clone()], None, false, None, false).0), ["clipboard"]);
        // Brought forward by the user, it stays in front.
        let (shown, _) = arrange(vec![copied.clone(), music.clone()], Some(&copied.id), false, None, false);
        assert_eq!(keys(&shown), ["clipboard", "now-playing"]);
    }
}
