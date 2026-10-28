//! Countdown timers as plain state on a monotonic millisecond clock the caller passes in
//! (the module's `Instant`, which keeps counting through sleep), so every rule here is
//! tested without waiting.

/// At most this many timers at once (E-T8).
pub const MAX_TIMERS: usize = 10;
/// A finish this late (sleep, ReUI closed) is shown quietly, without the sound.
pub const LATE_MS: u64 = 60_000;

#[derive(Debug, Clone, PartialEq)]
pub struct Timer {
    pub id: u32,
    pub label: Option<String>,
    /// The length it was started with (+1 minute adds to it), for the ring.
    pub length_ms: u64,
    /// Time left at `since`, or while paused.
    left_ms: u64,
    /// When it last started running; `None` while paused.
    since: Option<u64>,
}

impl Timer {
    pub fn left(&self, now: u64) -> u64 {
        match self.since {
            Some(since) => self.left_ms.saturating_sub(now.saturating_sub(since)),
            None => self.left_ms,
        }
    }

    pub fn running(&self) -> bool {
        self.since.is_some()
    }

    /// When it reaches zero, if it's running.
    pub fn ends_at(&self) -> Option<u64> {
        self.since.map(|since| since + self.left_ms)
    }
}

/// A timer that reached zero.
#[derive(Debug, Clone, PartialEq)]
pub struct Finished {
    pub label: Option<String>,
    pub length_ms: u64,
    /// How long ago it reached zero when it was noticed (sleep, a restart).
    pub late_ms: u64,
}

#[derive(Debug, Default)]
pub struct Timers {
    list: Vec<Timer>,
    next_id: u32,
}

impl Timers {
    /// Starts a timer; `None` when [`MAX_TIMERS`] are already going.
    pub fn start(&mut self, label: Option<String>, length_ms: u64, now: u64) -> Option<u32> {
        self.add(label, length_ms, length_ms, Some(now))
    }

    /// Puts back a timer (from disk, or an undone cancel) with `left_ms` to go.
    pub fn add(&mut self, label: Option<String>, length_ms: u64, left_ms: u64, since: Option<u64>) -> Option<u32> {
        if self.list.len() >= MAX_TIMERS {
            return None;
        }
        self.next_id += 1;
        let id = self.next_id;
        self.list.push(Timer { id, label, length_ms: length_ms.max(left_ms), left_ms, since });
        Some(id)
    }

    #[cfg(test)]
    pub fn get(&self, id: u32) -> Option<&Timer> {
        self.list.iter().find(|t| t.id == id)
    }

    pub fn all(&self) -> &[Timer] {
        &self.list
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.list.len()
    }

    /// Pauses a running timer, resumes a paused one.
    pub fn toggle(&mut self, id: u32, now: u64) {
        if let Some(t) = self.list.iter_mut().find(|t| t.id == id) {
            t.left_ms = t.left(now);
            t.since = if t.since.is_some() { None } else { Some(now) };
        }
    }

    pub fn cancel(&mut self, id: u32, now: u64) -> Option<Cancelled> {
        let i = self.list.iter().position(|t| t.id == id)?;
        let t = self.list.remove(i);
        Some(Cancelled { left_ms: t.left(now), running: t.running(), label: t.label, length_ms: t.length_ms })
    }

    /// Takes out every timer that has reached zero by `now`.
    pub fn take_finished(&mut self, now: u64) -> Vec<Finished> {
        let mut done = Vec::new();
        self.list.retain(|t| match t.ends_at() {
            Some(end) if end <= now => {
                done.push(Finished { label: t.label.clone(), length_ms: t.length_ms, late_ms: now - end });
                false
            }
            _ => true,
        });
        done
    }

    /// When the next one reaches zero.
    pub fn next_end(&self) -> Option<u64> {
        self.list.iter().filter_map(Timer::ends_at).min()
    }

    /// Soonest to end first: running ones by time left, then paused ones by time left.
    pub fn ordered(&self, now: u64) -> Vec<&Timer> {
        let mut list: Vec<&Timer> = self.list.iter().collect();
        list.sort_by_key(|t| (!t.running(), t.left(now), t.id));
        list
    }
}

/// What a cancel removed, kept for Ctrl+Z.
#[derive(Debug, Clone, PartialEq)]
pub struct Cancelled {
    pub label: Option<String>,
    pub length_ms: u64,
    pub left_ms: u64,
    pub running: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60_000;

    #[test]
    fn counts_down_pauses_and_resumes() {
        let mut t = Timers::default();
        let id = t.start(None, 4 * MIN, 1_000).unwrap();
        assert_eq!(t.get(id).unwrap().left(1_000 + MIN), 3 * MIN);
        t.toggle(id, 1_000 + MIN);
        assert!(!t.get(id).unwrap().running());
        assert_eq!(t.get(id).unwrap().left(1_000 + 10 * MIN), 3 * MIN);
        assert_eq!(t.next_end(), None);
        t.toggle(id, 1_000 + 10 * MIN);
        assert_eq!(t.next_end(), Some(1_000 + 13 * MIN));
        assert!(t.take_finished(1_000 + 13 * MIN - 1).is_empty());
        let done = t.take_finished(1_000 + 13 * MIN + 5);
        assert_eq!(done, vec![Finished { label: None, length_ms: 4 * MIN, late_ms: 5 }]);
        assert!(t.is_empty());
    }

    #[test]
    fn soonest_first_paused_last() {
        let mut t = Timers::default();
        let bread = t.start(Some("Bread".into()), 40 * MIN, 0).unwrap();
        let pasta = t.start(Some("Pasta".into()), 9 * MIN, 0).unwrap();
        let tea = t.start(Some("Tea".into()), 2 * MIN, 0).unwrap();
        t.toggle(tea, 0);
        let order: Vec<u32> = t.ordered(MIN).iter().map(|t| t.id).collect();
        assert_eq!(order, vec![pasta, bread, tea]);
    }

    #[test]
    fn cancel_and_the_limit() {
        let mut t = Timers::default();
        let id = t.start(None, MIN, 0).unwrap();
        let gone = t.cancel(id, 30_000).unwrap();
        assert_eq!((gone.left_ms, gone.running), (30_000, true));
        for _ in 0..MAX_TIMERS {
            assert!(t.start(None, MIN, 0).is_some());
        }
        assert_eq!(t.start(None, MIN, 0), None);
    }

    #[test]
    fn several_finish_together() {
        let mut t = Timers::default();
        t.start(Some("A".into()), MIN, 0);
        t.start(Some("B".into()), MIN, 0);
        t.start(Some("C".into()), 5 * MIN, 0);
        let done = t.take_finished(2 * MIN);
        assert_eq!(done.len(), 2);
        assert!(done.iter().all(|d| d.late_ms == MIN));
        assert_eq!(t.len(), 1);
    }
}
