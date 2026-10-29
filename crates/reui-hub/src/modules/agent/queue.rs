//! The moments of one agent module in one queue, oldest first, shown as one card with a
//! pager (PRD agent moments S10, E-A4), so sessions of one agent never stack up in the
//! island. Pure.
//!
//! Claude fires both of ReUI's `PreToolUse` entries for a question: the async one (no
//! reply possible) and the blocking one. Whichever comes first, they become one moment:
//! the blocking call upgrades a read-only moment in place, and the async duplicate of one
//! already waiting (or just settled) is ignored. Without the blocking entry (hooks from an
//! older build, E-A13) only the async call comes, and the moment stays read-only: "Answer
//! in terminal".

use std::collections::VecDeque;

use tokio::time::Instant;

use super::moments::{Kind, Moment};

/// Tool calls answered lately, so a late async duplicate doesn't come back read-only.
const SETTLED_KEPT: usize = 32;

/// How an arriving moment was taken.
#[derive(Debug, PartialEq)]
pub(super) enum Arrival {
    Added,
    /// The blocking call for a moment already shown read-only: now answerable.
    Upgraded,
    /// The async duplicate of one already here or just settled.
    Ignored,
}

#[derive(Default)]
pub(super) struct Queue {
    /// Oldest first.
    list: Vec<Moment>,
    /// The one in the card (its id); the oldest when unset or gone.
    shown: Option<u64>,
    settled: VecDeque<String>,
}

impl Queue {
    pub(super) fn arrive(&mut self, moment: Moment) -> Arrival {
        if let Some(tool_use) = &moment.tool_use_id {
            if !moment.waiting && self.settled.contains(tool_use) {
                return Arrival::Ignored;
            }
        }
        let same = |m: &Moment| {
            m.session == moment.session
                && match (&m.tool_use_id, &moment.tool_use_id) {
                    (Some(a), Some(b)) => a == b,
                    _ => m.input == moment.input && std::mem::discriminant(&m.kind) == std::mem::discriminant(&moment.kind),
                }
        };
        if let Some(existing) = self.list.iter_mut().find(|m| same(m) && !matches!(m.kind, Kind::Permission { .. })) {
            if !moment.waiting || existing.waiting {
                return Arrival::Ignored;
            }
            if self.shown == Some(existing.id) {
                self.shown = Some(moment.id);
            }
            // Keep what was already done with it (a step, a check), now with a bridge.
            existing.id = moment.id;
            existing.waiting = true;
            existing.deadline = moment.deadline;
            return Arrival::Upgraded;
        }
        self.list.push(moment);
        Arrival::Added
    }

    pub(super) fn get_mut(&mut self, id: u64) -> Option<&mut Moment> {
        self.list.iter_mut().find(|m| m.id == id)
    }

    pub(super) fn current(&self) -> Option<&Moment> {
        self.shown.and_then(|id| self.list.iter().find(|m| m.id == id)).or(self.list.first())
    }

    pub(super) fn current_mut(&mut self) -> Option<&mut Moment> {
        let id = self.current()?.id;
        self.get_mut(id)
    }

    /// Where the card is: (index, count).
    pub(super) fn position(&self) -> (usize, usize) {
        let at = self.current().and_then(|c| self.list.iter().position(|m| m.id == c.id)).unwrap_or(0);
        (at, self.list.len())
    }

    /// The pager: shows the next (`delta` 1) or previous one, stopping at the ends.
    pub(super) fn page(&mut self, delta: i32) -> bool {
        let (at, count) = self.position();
        let to = (at as i64 + i64::from(delta)).clamp(0, count.saturating_sub(1) as i64) as usize;
        if count == 0 || to == at {
            return false;
        }
        self.shown = Some(self.list[to].id);
        true
    }

    /// Takes a moment out (answered, gone, timed out), remembering its tool call.
    pub(super) fn remove(&mut self, id: u64) -> Option<Moment> {
        let at = self.list.iter().position(|m| m.id == id)?;
        let moment = self.list.remove(at);
        if self.shown == Some(id) {
            // The next one in line, else the one before.
            self.shown = self.list.get(at).or(self.list.last()).map(|m| m.id);
        }
        if let Some(tool_use) = &moment.tool_use_id {
            self.settled.push_back(tool_use.clone());
            if self.settled.len() > SETTLED_KEPT {
                self.settled.pop_front();
            }
        }
        Some(moment)
    }

    /// Read-only moments of `session` that are over (its tool ran, the turn ended); all of
    /// them when `tool_use_id` is `None`. Returns whether any went.
    pub(super) fn end_read_only(&mut self, session: &str, tool_use_id: Option<&str>) -> bool {
        let gone: Vec<u64> = self
            .list
            .iter()
            .filter(|m| !m.waiting && m.session == session && tool_use_id.is_none_or(|t| m.tool_use_id.as_deref().is_none_or(|own| own == t)))
            .map(|m| m.id)
            .collect();
        for id in &gone {
            self.remove(*id);
        }
        !gone.is_empty()
    }

    pub(super) fn has_session(&self, session: &str) -> bool {
        self.list.iter().any(|m| m.session == session)
    }

    pub(super) fn deadline(&self) -> Option<Instant> {
        self.list.iter().map(|m| m.deadline).min()
    }

    /// Takes out every moment whose wait is over.
    pub(super) fn expire(&mut self, now: Instant) -> Vec<Moment> {
        let ids: Vec<u64> = self.list.iter().filter(|m| m.deadline <= now).map(|m| m.id).collect();
        ids.into_iter().filter_map(|id| self.remove(id)).collect()
    }

    /// Splits `<id>:<action>` (an island action; `alt:` in front when pressed with Alt).
    pub(super) fn parse(action: &str) -> Option<(u64, String)> {
        let (alt, rest) = match action.strip_prefix(reui_protocol::ACTION_ALT) {
            Some(rest) => (true, rest),
            None => (false, action),
        };
        let (id, verb) = rest.split_once(':')?;
        let id = id.parse().ok()?;
        Some((id, if alt { format!("alt:{verb}") } else { verb.to_string() }))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use super::*;
    use crate::modules::agent::moments::tests::ask;
    use crate::modules::agent::moments::PERMISSION_WAIT;

    #[test]
    fn the_blocking_and_async_calls_become_one_moment() {
        // Async first: read-only, then upgraded in place by the blocking call.
        let mut q = Queue::default();
        assert_eq!(q.arrive(ask(1, false)), Arrival::Added);
        q.get_mut(1).unwrap().act("pick:0");
        assert_eq!(q.arrive(ask(2, true)), Arrival::Upgraded);
        let m = q.current().unwrap();
        assert!(m.id == 2 && m.waiting && matches!(m.kind, Kind::Question { step: 0, .. }));
        // Blocking first: the async duplicate is ignored, also after it's answered.
        let mut q = Queue::default();
        assert_eq!(q.arrive(ask(2, true)), Arrival::Added);
        assert_eq!(q.arrive(ask(1, false)), Arrival::Ignored);
        q.remove(2);
        assert_eq!(q.arrive(ask(3, false)), Arrival::Ignored);
        assert!(q.current().is_none());
    }

    #[test]
    fn several_moments_page_oldest_first() {
        let now = Instant::now();
        let mut q = Queue::default();
        q.arrive(Moment::permission(1, "a", "Bash", json!({}), None, now));
        q.arrive(Moment::permission(2, "b", "Edit", json!({}), None, now + Duration::from_secs(1)));
        q.arrive(Moment::ask(3, "a", "ExitPlanMode", None, json!({}), true, now).unwrap());
        assert_eq!((q.current().unwrap().id, q.position()), (1, (0, 3)));
        assert!(!q.page(-1));
        assert!(q.page(1) && q.page(1) && !q.page(1));
        assert_eq!(q.position(), (2, 3));
        // The one shown goes: the one before it shows.
        q.remove(3);
        assert_eq!(q.current().unwrap().id, 2);
        q.page(-1);
        q.remove(1);
        assert_eq!((q.current().unwrap().id, q.position()), (2, (0, 1)));
        assert_eq!(q.deadline(), Some(now + Duration::from_secs(1) + PERMISSION_WAIT));
        assert_eq!(q.expire(now + Duration::from_secs(1) + PERMISSION_WAIT).len(), 1);
        assert!(q.current().is_none());
    }

    #[test]
    fn read_only_moments_end_with_their_tool() {
        let mut q = Queue::default();
        q.arrive(ask(1, false));
        q.arrive(Moment::permission(2, "s1", "Bash", json!({}), None, Instant::now()));
        assert!(!q.end_read_only("s1", Some("toolu_other")));
        assert!(q.end_read_only("s1", Some("toolu_1")));
        assert!(q.has_session("s1"), "the permission waits on");
        assert!(!q.end_read_only("s1", None));
    }

    #[test]
    fn island_actions_name_their_moment() {
        assert_eq!(Queue::parse("12:pick:3"), Some((12, "pick:3".into())));
        assert_eq!(Queue::parse(&format!("{}12:deny", reui_protocol::ACTION_ALT)), Some((12, "alt:deny".into())));
        assert_eq!(Queue::parse("submit"), None);
    }
}
