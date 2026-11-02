//! When to say a restart is needed (docs/PRD-update-restart.md §4.1). Pure: what the
//! watcher saw, what was said, and the time in; what to do and when to look again out.
//!
//! Once per set of updates: Windows lists the updates waiting for a restart as value
//! names under `Auto Update\RebootRequired`, so a new update waiting says it again and
//! the same ones never do. The note shows for `NOTE`, longer while it's open.

use std::time::Duration;

/// What Windows Update says, as the watcher reads it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    /// The updates waiting for a restart (empty: none, no restart needed).
    pub waiting: Vec<String>,
    /// Active hours, start and end (0..=23): Windows restarts by itself outside them.
    pub active: Option<(u32, u32)>,
}

impl State {
    pub fn needed(&self) -> bool {
        !self.waiting.is_empty()
    }
}

/// How long the note stays when nobody opens it.
pub const NOTE: Duration = Duration::from_secs(12);
/// How long "Restart · sure?" waits for the second press.
pub const CONFIRM: Duration = Duration::from_secs(3);

/// What the module does next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Show (or update) the note.
    Show,
    /// Take it away.
    Retract,
    Nothing,
}

#[derive(Debug, Default)]
pub struct Plan {
    /// The updates last said (sorted), so the same ones aren't said twice.
    said: Vec<String>,
    /// The note is up.
    pub showing: bool,
    /// The note is open: it stays until it's closed.
    pub open: bool,
    /// When the note goes if it isn't opened (ms, the module's clock).
    pub until: Option<u64>,
    /// When the first press of Restart now stops counting (ms).
    pub confirm_until: Option<u64>,
}

impl Plan {
    /// A new reading from the watcher at `now` (ms).
    pub fn saw(&mut self, state: &State, now: u64) -> Step {
        if !state.needed() {
            self.said.clear();
            return self.hide();
        }
        let mut waiting = state.waiting.clone();
        waiting.sort();
        // Only new updates are news; the same set again (any write to the key) isn't.
        if waiting.iter().all(|w| self.said.contains(w)) {
            return Step::Nothing;
        }
        self.said = waiting;
        self.showing = true;
        self.until = Some(now + NOTE.as_millis() as u64);
        Step::Show
    }

    pub fn opened(&mut self) {
        if self.showing {
            self.open = true;
            self.until = None;
        }
    }

    /// Closed again: it's been read, so it goes.
    pub fn closed(&mut self) -> Step {
        if self.open { self.hide() } else { Step::Nothing }
    }

    /// The first press of Restart now asks; a second within `CONFIRM` restarts. Returns
    /// whether to restart now.
    pub fn restart_pressed(&mut self, now: u64) -> bool {
        match self.confirm_until {
            Some(until) if now <= until => {
                self.confirm_until = None;
                true
            }
            _ => {
                self.confirm_until = Some(now + CONFIRM.as_millis() as u64);
                false
            }
        }
    }

    /// Schedule was pressed: Windows' page takes over, the note goes.
    pub fn scheduled(&mut self) -> Step {
        self.hide()
    }

    /// When the module should look again (ms): the note's end or the confirm's.
    pub fn next(&self) -> Option<u64> {
        [self.until, self.confirm_until].into_iter().flatten().min()
    }

    /// The time came at `now`.
    pub fn tick(&mut self, now: u64) -> Step {
        let mut step = Step::Nothing;
        if self.confirm_until.is_some_and(|t| now >= t) {
            self.confirm_until = None;
            step = Step::Show;
        }
        if self.until.is_some_and(|t| now >= t) {
            return self.hide();
        }
        step
    }

    pub fn confirming(&self) -> bool {
        self.confirm_until.is_some()
    }

    fn hide(&mut self) -> Step {
        let was = self.showing;
        self.showing = false;
        self.open = false;
        self.until = None;
        self.confirm_until = None;
        if was { Step::Retract } else { Step::Nothing }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn waiting(ids: &[&str]) -> State {
        State { waiting: ids.iter().map(|s| s.to_string()).collect(), active: Some((8, 17)) }
    }

    #[test]
    fn a_waiting_update_is_said_once() {
        let mut p = Plan::default();
        assert_eq!(p.saw(&State::default(), 0), Step::Nothing);
        assert_eq!(p.saw(&waiting(&["b", "a"]), 1_000), Step::Show);
        assert_eq!(p.next(), Some(1_000 + NOTE.as_millis() as u64));
        // Windows writes the key again: same updates, nothing new.
        assert_eq!(p.saw(&waiting(&["a", "b"]), 2_000), Step::Nothing);
        // Unopened, it goes on its own.
        assert_eq!(p.tick(1_000 + NOTE.as_millis() as u64), Step::Retract);
        assert_eq!(p.saw(&waiting(&["a", "b"]), 60_000), Step::Nothing);
        // Another update joins: news again.
        assert_eq!(p.saw(&waiting(&["a", "b", "c"]), 70_000), Step::Show);
    }

    #[test]
    fn opened_it_stays_until_closed() {
        let mut p = Plan::default();
        p.saw(&waiting(&["a"]), 0);
        p.opened();
        assert_eq!(p.next(), None);
        assert_eq!(p.tick(NOTE.as_millis() as u64 * 10), Step::Nothing);
        assert_eq!(p.closed(), Step::Retract);
        assert!(!p.showing);
    }

    #[test]
    fn restart_asks_once_then_restarts() {
        let mut p = Plan::default();
        p.saw(&waiting(&["a"]), 0);
        p.opened();
        assert!(!p.restart_pressed(1_000));
        assert!(p.confirming());
        assert!(p.restart_pressed(2_500));
        // Too late: it asks again.
        assert!(!p.restart_pressed(10_000));
        assert_eq!(p.tick(10_000 + CONFIRM.as_millis() as u64), Step::Show);
        assert!(!p.confirming());
        assert!(!p.restart_pressed(20_000));
    }

    #[test]
    fn done_or_scheduled_it_goes() {
        let mut p = Plan::default();
        p.saw(&waiting(&["a"]), 0);
        assert_eq!(p.saw(&State::default(), 1), Step::Retract);
        // The restart happened; the same id waiting again (a retry) is news.
        assert_eq!(p.saw(&waiting(&["a"]), 2), Step::Show);
        assert_eq!(p.scheduled(), Step::Retract);
        assert_eq!(p.scheduled(), Step::Nothing);
    }
}
