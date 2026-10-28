//! The stopwatch: elapsed time on the module's monotonic clock, and laps.

/// Laps kept; the stopwatch keeps running past it, it just stops counting laps.
pub const MAX_LAPS: usize = 99;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Stopwatch {
    /// Elapsed at `since`, or while paused.
    base_ms: u64,
    since: Option<u64>,
    /// Each lap's own length, oldest first.
    pub laps: Vec<u64>,
    /// Elapsed when the last lap was taken.
    lapped_at: u64,
}

impl Stopwatch {
    pub fn start(now: u64) -> Self {
        Self { since: Some(now), ..Self::default() }
    }

    /// Put back from disk.
    pub fn restore(elapsed_ms: u64, since: Option<u64>, laps: Vec<u64>) -> Self {
        let lapped_at = laps.iter().sum::<u64>().min(elapsed_ms);
        Self { base_ms: elapsed_ms, since, laps, lapped_at }
    }

    pub fn elapsed(&self, now: u64) -> u64 {
        self.base_ms + self.since.map_or(0, |since| now.saturating_sub(since))
    }

    pub fn running(&self) -> bool {
        self.since.is_some()
    }

    pub fn toggle(&mut self, now: u64) {
        self.base_ms = self.elapsed(now);
        self.since = if self.since.is_some() { None } else { Some(now) };
    }

    /// Takes a lap; `None` past [`MAX_LAPS`] or with nothing elapsed since the last.
    pub fn lap(&mut self, now: u64) -> Option<u64> {
        let elapsed = self.elapsed(now);
        let lap = elapsed - self.lapped_at;
        if self.laps.len() >= MAX_LAPS || lap == 0 {
            return None;
        }
        self.laps.push(lap);
        self.lapped_at = elapsed;
        Some(lap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_pauses_and_laps() {
        let mut s = Stopwatch::start(1_000);
        assert_eq!(s.elapsed(4_000), 3_000);
        assert_eq!(s.lap(4_000), Some(3_000));
        assert_eq!(s.lap(4_000), None);
        s.toggle(5_000);
        assert_eq!(s.elapsed(60_000), 4_000);
        s.toggle(60_000);
        assert_eq!(s.lap(62_500), Some(3_500));
        assert_eq!(s.laps, vec![3_000, 3_500]);

        let back = Stopwatch::restore(s.elapsed(62_500), None, s.laps.clone());
        assert_eq!(back.elapsed(99_999), 6_500);
        assert!(!back.running());
    }
}
