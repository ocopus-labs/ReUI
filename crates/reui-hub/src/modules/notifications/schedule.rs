//! When the listener thread lists the notification centre. Pure, with the clock passed in.
//!
//! Listing is what costs: `GetNotificationsAsync` serialises every toast in the
//! notification centre across processes, measured on a Windows 11 26200 PC (release build)
//! at ~3 ms of CPU with one toast and ~28 ms with twenty, plus ~2 ms in the notification
//! service. So the module lists only when something says a toast came or went:
//!
//! - **`Wnf`** (the normal case): the shell's notification-count WNF state changes within
//!   ~35 ms of every toast and every removal (`wnf.rs`), with no change at all in a quiet
//!   minute. A change is debounced by `DEBOUNCE` (one burst, one list). It's undocumented,
//!   so a slow safety list every `SAFETY` checks it: a new toast found there that no change
//!   announced means the count stopped working on this build, and the schedule drops to
//!   `Poll` for good.
//! - **`Poll`** (no WNF, or it stopped working): `POLL` (5 s), `POLL_BUSY` (2 s) for a
//!   minute after something new (messages come in bursts), backing off to `POLL_QUIET`
//!   (15 s) after ten quiet minutes. The listener skips a poll while the session is locked.
//!   5 s rather than 1–2 s because the cost grows with the notification centre: polling
//!   at 1 s with twenty toasts waiting would cost ~3 % of a core all day.

use std::time::{Duration, Instant};

pub const DEBOUNCE: Duration = Duration::from_millis(150);
pub const SAFETY: Duration = Duration::from_secs(300);
pub const POLL: Duration = Duration::from_secs(5);
pub const POLL_BUSY: Duration = Duration::from_secs(2);
pub const POLL_QUIET: Duration = Duration::from_secs(15);
const BUSY_FOR: Duration = Duration::from_secs(60);
const QUIET_AFTER: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    Wnf,
    Poll,
}

#[derive(Debug)]
pub struct Schedule {
    trigger: Trigger,
    last_list: Instant,
    started: Instant,
    last_new: Option<Instant>,
    /// A WNF change waiting out its debounce.
    pending: Option<Instant>,
}

impl Schedule {
    /// `listed` is the first list (the baseline), made just now.
    pub fn new(trigger: Trigger, listed: Instant) -> Self {
        Self { trigger, last_list: listed, started: listed, last_new: None, pending: None }
    }

    pub fn trigger(&self) -> Trigger {
        self.trigger
    }

    /// The count changed: list after the debounce (a burst moves the count several times).
    pub fn signal(&mut self, now: Instant) {
        self.pending.get_or_insert(now + DEBOUNCE);
    }

    fn interval(&self, now: Instant) -> Duration {
        match self.trigger {
            Trigger::Wnf => SAFETY,
            Trigger::Poll => {
                let since = now.saturating_duration_since(self.last_new.unwrap_or(self.started));
                if self.last_new.is_some() && since < BUSY_FOR {
                    POLL_BUSY
                } else if since < QUIET_AFTER {
                    POLL
                } else {
                    POLL_QUIET
                }
            }
        }
    }

    /// When to list next.
    pub fn next(&self, now: Instant) -> Instant {
        let timed = self.last_list + self.interval(now);
        self.pending.map_or(timed, |p| p.min(timed))
    }

    /// Whether the list due at `now` was asked for by a WNF change (rather than a timer).
    pub fn signalled(&self, now: Instant) -> bool {
        self.pending.is_some_and(|p| p <= now)
    }

    /// A list was made at `now`; `found_new` if it had toasts the last one didn't.
    /// `true` if that just dropped the schedule to polling (the count missed a toast).
    pub fn listed(&mut self, now: Instant, found_new: bool) -> bool {
        let signalled = self.signalled(now);
        self.pending = None;
        self.last_list = now;
        if found_new {
            self.last_new = Some(now);
        }
        if self.trigger == Trigger::Wnf && found_new && !signalled {
            self.trigger = Trigger::Poll;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: Duration = Duration::from_secs(1);

    #[test]
    fn notifications_wnf_lists_once_per_burst_and_rarely_otherwise() {
        let t0 = Instant::now();
        let mut s = Schedule::new(Trigger::Wnf, t0);
        assert_eq!(s.next(t0), t0 + SAFETY, "nothing happening: only the safety list");
        s.signal(t0 + S);
        s.signal(t0 + S + Duration::from_millis(40));
        assert_eq!(s.next(t0 + S), t0 + S + DEBOUNCE, "the first change of a burst sets the time");
        assert!(s.signalled(t0 + S + DEBOUNCE));
        assert!(!s.listed(t0 + S + DEBOUNCE, true));
        assert_eq!(s.next(t0 + 2 * S), t0 + S + DEBOUNCE + SAFETY);
        assert_eq!(s.trigger(), Trigger::Wnf);
    }

    #[test]
    fn notifications_a_toast_the_count_missed_drops_to_polling() {
        let t0 = Instant::now();
        let mut s = Schedule::new(Trigger::Wnf, t0);
        assert!(!s.listed(t0 + SAFETY, false), "a quiet safety list proves nothing");
        assert!(s.listed(t0 + 2 * SAFETY, true), "new toasts nobody announced");
        assert_eq!(s.trigger(), Trigger::Poll);
        assert_eq!(s.next(t0 + 2 * SAFETY), t0 + 2 * SAFETY + POLL_BUSY);
    }

    #[test]
    fn notifications_polling_backs_off_when_quiet() {
        let t0 = Instant::now();
        let mut s = Schedule::new(Trigger::Poll, t0);
        assert_eq!(s.next(t0), t0 + POLL);
        s.listed(t0 + 5 * S, true);
        assert_eq!(s.next(t0 + 5 * S), t0 + 5 * S + POLL_BUSY, "busy right after something new");
        let later = t0 + 5 * S + BUSY_FOR;
        s.listed(later, false);
        assert_eq!(s.next(later), later + POLL);
        let quiet = t0 + 5 * S + QUIET_AFTER;
        s.listed(quiet, false);
        assert_eq!(s.next(quiet), quiet + POLL_QUIET);
        // Never anything new: quiet after ten minutes of lists too.
        let mut s = Schedule::new(Trigger::Poll, t0);
        s.listed(t0 + QUIET_AFTER - S, false);
        assert_eq!(s.next(t0 + QUIET_AFTER - S), t0 + QUIET_AFTER - S + POLL);
        s.listed(t0 + QUIET_AFTER, false);
        assert_eq!(s.next(t0 + QUIET_AFTER), t0 + QUIET_AFTER + POLL_QUIET);
    }
}
