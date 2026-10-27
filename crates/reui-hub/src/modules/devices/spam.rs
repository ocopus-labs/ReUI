//! When a connect deserves a card (PRD 5.2): nothing for 15 s after the hub starts, the
//! machine resumes or the session unlocks (what's connected then is the baseline); a
//! reconnect within 10 s of a disconnect is a flap; at most one card per device a minute;
//! and connects within 2 s of the first one merge into it ("3 devices connected"). Pure:
//! time comes in as an argument.

use std::collections::HashMap;
use std::hash::Hash;
use std::time::Duration;

use tokio::time::Instant;

pub const GRACE: Duration = Duration::from_secs(15);
pub const FLAP: Duration = Duration::from_secs(10);
pub const COOLDOWN: Duration = Duration::from_secs(60);
pub const MERGE: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict<K> {
    /// No card.
    Quiet,
    /// A card of its own.
    Show,
    /// Joins the card `into`, which now stands for all of `members` (`into` first).
    Merge { into: K, members: Vec<K> },
}

#[derive(Debug, Default)]
struct Seen {
    connected: bool,
    disconnected_at: Option<Instant>,
    shown_at: Option<Instant>,
}

#[derive(Debug)]
struct Burst<K> {
    started: Instant,
    members: Vec<K>,
}

#[derive(Debug)]
pub struct Gate<K> {
    quiet_until: Instant,
    seen: HashMap<K, Seen>,
    burst: Option<Burst<K>>,
}

impl<K: Clone + Eq + Hash> Gate<K> {
    /// Starts in the startup grace.
    pub fn new(now: Instant) -> Self {
        Self { quiet_until: now + GRACE, seen: HashMap::new(), burst: None }
    }

    /// Resume or unlock: devices reconnecting now are the baseline, not news.
    pub fn quiet(&mut self, now: Instant) {
        self.quiet_until = self.quiet_until.max(now + GRACE);
        self.burst = None;
    }

    pub fn in_grace(&self, now: Instant) -> bool {
        now < self.quiet_until
    }

    pub fn connected(&mut self, key: K, now: Instant) -> Verdict<K> {
        let seen = self.seen.entry(key.clone()).or_default();
        if std::mem::replace(&mut seen.connected, true) {
            return Verdict::Quiet;
        }
        let recent = |at: Option<Instant>, window: Duration| at.is_some_and(|at| now < at + window);
        if now < self.quiet_until || recent(seen.disconnected_at, FLAP) || recent(seen.shown_at, COOLDOWN) {
            return Verdict::Quiet;
        }
        seen.shown_at = Some(now);
        match &mut self.burst {
            Some(burst) if now < burst.started + MERGE && !burst.members.is_empty() => {
                burst.members.push(key);
                Verdict::Merge { into: burst.members[0].clone(), members: burst.members.clone() }
            }
            _ => {
                self.burst = Some(Burst { started: now, members: vec![key] });
                Verdict::Show
            }
        }
    }

    /// Silent; remembered for the flap rule.
    pub fn disconnected(&mut self, key: &K, now: Instant) {
        if let Some(seen) = self.seen.get_mut(key) {
            if std::mem::replace(&mut seen.connected, false) {
                seen.disconnected_at = Some(now);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(base: Instant, s: f32) -> Instant {
        base + Duration::from_secs_f32(s)
    }

    #[test]
    fn devices_grace_makes_a_baseline() {
        let t = Instant::now();
        let mut g = Gate::new(t);
        assert_eq!(g.connected("buds", at(t, 1.0)), Verdict::Quiet);
        assert_eq!(g.connected("mouse", at(t, 14.0)), Verdict::Quiet);
        assert!(g.in_grace(at(t, 14.9)) && !g.in_grace(at(t, 15.0)));
        // Already connected: a repeat is nothing even after the grace.
        assert_eq!(g.connected("buds", at(t, 20.0)), Verdict::Quiet);
        assert_eq!(g.connected("keyboard", at(t, 20.0)), Verdict::Show);

        // Resume: the three that come back are quiet.
        g.quiet(at(t, 100.0));
        for k in ["buds", "mouse", "keyboard"] {
            g.disconnected(&k, at(t, 99.0));
        }
        for k in ["buds", "mouse", "keyboard"] {
            assert_eq!(g.connected(k, at(t, 105.0)), Verdict::Quiet);
        }
    }

    #[test]
    fn devices_flaps_and_cooldown() {
        let t = Instant::now();
        let mut g = Gate::new(t);
        let t = at(t, 20.0);
        assert_eq!(g.connected("buds", t), Verdict::Show);
        g.disconnected(&"buds", at(t, 70.0));
        // Back within 10 s: a flap.
        assert_eq!(g.connected("buds", at(t, 75.0)), Verdict::Quiet);
        g.disconnected(&"buds", at(t, 80.0));
        assert_eq!(g.connected("buds", at(t, 95.0)), Verdict::Show);
        // Within a minute of the last card: nothing, even after a real gap.
        g.disconnected(&"buds", at(t, 100.0));
        assert_eq!(g.connected("buds", at(t, 120.0)), Verdict::Quiet);
        g.disconnected(&"buds", at(t, 130.0));
        assert_eq!(g.connected("buds", at(t, 160.0)), Verdict::Show);
    }

    #[test]
    fn devices_connects_within_two_seconds_merge() {
        let t = Instant::now();
        let mut g = Gate::new(t);
        let t = at(t, 30.0);
        assert_eq!(g.connected("a", t), Verdict::Show);
        assert_eq!(g.connected("b", at(t, 0.5)), Verdict::Merge { into: "a", members: vec!["a", "b"] });
        assert_eq!(g.connected("c", at(t, 1.9)), Verdict::Merge { into: "a", members: vec!["a", "b", "c"] });
        // Measured from the first: a fourth after 2 s is a card of its own.
        assert_eq!(g.connected("d", at(t, 2.1)), Verdict::Show);
        // A resume ends any burst.
        g.quiet(at(t, 3.0));
        assert_eq!(g.connected("e", at(t, 3.5)), Verdict::Quiet);
    }
}
