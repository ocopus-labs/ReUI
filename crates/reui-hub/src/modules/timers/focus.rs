//! A focus session: work and breaks in turn (25 / 5, and 15 after every 4th work phase by
//! default), on the module's monotonic clock. When a phase ends the session waits for the
//! user to start the next one, unless auto-start is on: the app reminds, it doesn't decide.

/// How long an unanswered phase end waits before the session ends by itself.
pub const UNANSWERED_MS: u64 = 10 * 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Work,
    ShortBreak,
    LongBreak,
}

impl Phase {
    pub fn is_break(self) -> bool {
        self != Phase::Work
    }
}

/// Phase lengths and the long-break rhythm, from settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    pub work_ms: u64,
    pub short_break_ms: u64,
    pub long_break_ms: u64,
    /// Work phases per long break.
    pub rounds: u32,
}

impl Plan {
    fn length(&self, phase: Phase) -> u64 {
        match phase {
            Phase::Work => self.work_ms,
            Phase::ShortBreak => self.short_break_ms,
            Phase::LongBreak => self.long_break_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Focus {
    pub plan: Plan,
    pub phase: Phase,
    /// Work phases finished so far.
    pub done_work: u32,
    pub length_ms: u64,
    left_ms: u64,
    /// When it last started running; `None` while paused or waiting.
    since: Option<u64>,
    /// The phase ended at this time and the next one waits to be started.
    waiting_since: Option<u64>,
}

impl Focus {
    pub fn start(plan: Plan, now: u64) -> Self {
        Self { plan, phase: Phase::Work, done_work: 0, length_ms: plan.work_ms, left_ms: plan.work_ms, since: Some(now), waiting_since: None }
    }

    /// Put back from disk: `left_ms` to go in `phase`, running from `since` or paused.
    pub fn restore(plan: Plan, phase: Phase, done_work: u32, left_ms: u64, since: Option<u64>) -> Self {
        let length_ms = plan.length(phase).max(left_ms);
        Self { plan, phase, done_work, length_ms, left_ms, since, waiting_since: None }
    }

    pub fn left(&self, now: u64) -> u64 {
        match self.since {
            Some(since) => self.left_ms.saturating_sub(now.saturating_sub(since)),
            None => self.left_ms,
        }
    }

    pub fn running(&self) -> bool {
        self.since.is_some()
    }

    pub fn waiting(&self) -> bool {
        self.waiting_since.is_some()
    }

    /// Which round of work this is, from 1 to `plan.rounds`.
    pub fn round(&self) -> u32 {
        let rounds = self.plan.rounds.max(1);
        match self.phase {
            Phase::Work => self.done_work % rounds + 1,
            // A break belongs to the work phase just finished.
            _ => (self.done_work + rounds - 1) % rounds + 1,
        }
    }

    /// When something is due: the phase's end, or an unanswered wait running out.
    pub fn next_due(&self) -> Option<u64> {
        match (self.since, self.waiting_since) {
            (_, Some(at)) => Some(at + UNANSWERED_MS),
            (Some(since), None) => Some(since + self.left_ms),
            (None, None) => None,
        }
    }

    pub fn toggle(&mut self, now: u64) {
        if self.waiting() {
            return;
        }
        self.left_ms = self.left(now);
        self.since = if self.since.is_some() { None } else { Some(now) };
    }

    /// The phase reached zero: counts the work, and waits (or, with `auto`, goes straight
    /// on). `true` if it ended just now.
    pub fn end_phase(&mut self, now: u64, auto: bool) -> bool {
        if self.waiting() || self.since.is_none() || self.left(now) > 0 {
            return false;
        }
        if self.phase == Phase::Work {
            self.done_work += 1;
        }
        self.since = None;
        self.left_ms = 0;
        self.waiting_since = Some(now);
        if auto {
            self.begin(self.next_phase(), now);
        }
        true
    }

    /// What follows the phase that just ended (`done_work` already counts it): the phase
    /// a waiting session starts next.
    pub fn next_phase(&self) -> Phase {
        match self.phase {
            Phase::Work if self.done_work % self.plan.rounds.max(1) == 0 => Phase::LongBreak,
            Phase::Work => Phase::ShortBreak,
            _ => Phase::Work,
        }
    }

    /// Starts the phase the session is waiting on.
    pub fn start_next(&mut self, now: u64) {
        if self.waiting() {
            self.begin(self.next_phase(), now);
        }
    }

    /// Skips the break: straight to work, now (from a waiting break, or during one).
    pub fn skip_break(&mut self, now: u64) {
        let on_break = if self.waiting() { self.next_phase().is_break() } else { self.phase.is_break() };
        if on_break {
            self.begin(Phase::Work, now);
        }
    }

    /// The unanswered wait ran out.
    pub fn gave_up(&self, now: u64) -> bool {
        self.waiting_since.is_some_and(|at| now >= at + UNANSWERED_MS)
    }

    fn begin(&mut self, phase: Phase, now: u64) {
        self.phase = phase;
        self.length_ms = self.plan.length(phase);
        self.left_ms = self.length_ms;
        self.since = Some(now);
        self.waiting_since = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60_000;
    const PLAN: Plan = Plan { work_ms: 25 * MIN, short_break_ms: 5 * MIN, long_break_ms: 15 * MIN, rounds: 4 };

    /// Runs the current phase to its end and starts the next.
    fn finish(f: &mut Focus, now: &mut u64) {
        *now += f.left(*now);
        assert!(f.end_phase(*now, false));
        f.start_next(*now);
    }

    #[test]
    fn work_and_breaks_with_a_long_one_every_fourth() {
        let mut now = 0;
        let mut f = Focus::start(PLAN, now);
        let mut phases = vec![(f.phase, f.round())];
        for _ in 0..8 {
            finish(&mut f, &mut now);
            phases.push((f.phase, f.round()));
        }
        use Phase::*;
        assert_eq!(
            phases,
            vec![(Work, 1), (ShortBreak, 1), (Work, 2), (ShortBreak, 2), (Work, 3), (ShortBreak, 3), (Work, 4), (LongBreak, 4), (Work, 1)]
        );
        assert_eq!(f.done_work, 4);
    }

    #[test]
    fn waits_to_be_asked_unless_auto() {
        let mut f = Focus::start(PLAN, 0);
        assert_eq!(f.next_due(), Some(25 * MIN));
        assert!(!f.end_phase(25 * MIN - 1, false));
        assert!(f.end_phase(25 * MIN, false));
        assert!(f.waiting() && !f.running());
        assert_eq!(f.next_due(), Some(25 * MIN + UNANSWERED_MS));
        assert!(!f.gave_up(25 * MIN + UNANSWERED_MS - 1));
        assert!(f.gave_up(25 * MIN + UNANSWERED_MS));

        let mut auto = Focus::start(PLAN, 0);
        assert!(auto.end_phase(25 * MIN + 500, true));
        assert_eq!((auto.phase, auto.running(), auto.waiting()), (Phase::ShortBreak, true, false));
    }

    #[test]
    fn pause_and_skip_the_break() {
        let mut f = Focus::start(PLAN, 0);
        f.toggle(MIN);
        assert_eq!(f.left(30 * MIN), 24 * MIN);
        assert_eq!(f.next_due(), None);
        f.toggle(30 * MIN);
        assert!(f.end_phase(54 * MIN, false));
        f.skip_break(54 * MIN);
        assert_eq!((f.phase, f.round(), f.left(54 * MIN)), (Phase::Work, 2, 25 * MIN));
        // Skipping during work does nothing.
        f.skip_break(55 * MIN);
        assert_eq!(f.left(55 * MIN), 24 * MIN);
    }
}
