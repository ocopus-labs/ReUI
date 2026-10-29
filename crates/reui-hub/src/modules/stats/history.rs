//! The last minute of each gauge's readings, kept only while the card is open: the
//! sampler runs only then, so there is nothing to keep otherwise, and a new opening
//! starts empty rather than joining two minutes with a gap between them. Pure.
//!
//! It also decides when a number has *stayed* high (the card's one use of colour): a
//! single busy second is normal, three in a row is worth noticing.

use std::collections::VecDeque;

use reui_protocol::{Tone, GAUGE_HISTORY};

/// A share at or over this is high.
const HIGH: f32 = 0.9;
/// This many high readings in a row turn the number to `Warning`.
const SUSTAINED: usize = 3;

/// One measure's readings, oldest first, at most a minute of them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Series(VecDeque<f32>);

impl Series {
    /// A reading, 0..=1; the oldest goes once there's a minute of them.
    pub fn push(&mut self, value: f32) {
        if self.0.len() == GAUGE_HISTORY {
            self.0.pop_front();
        }
        self.0.push_back(value.clamp(0.0, 1.0));
    }

    pub fn values(&self) -> Vec<f32> {
        self.0.iter().copied().collect()
    }

    /// `Warning` once the last [`SUSTAINED`] readings were all high, else `Primary`.
    pub fn tone(&self) -> Tone {
        if self.0.len() >= SUSTAINED && self.0.iter().rev().take(SUSTAINED).all(|v| *v >= HIGH) {
            Tone::Warning
        } else {
            Tone::Primary
        }
    }
}

/// Every measure the card colours or draws a line for.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct History {
    pub cpu: Series,
    pub memory: Series,
    pub gpu: Series,
    pub disk: Series,
}

impl History {
    /// One reading's shares. A part that wasn't read this second (unknown, or the first
    /// sample of a rate) adds nothing rather than a made-up value.
    pub fn push(&mut self, cpu: Option<f32>, memory: Option<f32>, gpu: Option<f32>, disk: Option<f32>) {
        for (series, value) in [(&mut self.cpu, cpu), (&mut self.memory, memory), (&mut self.gpu, gpu), (&mut self.disk, disk)] {
            if let Some(v) = value {
                series.push(v);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_history_keeps_the_last_minute() {
        let mut s = Series::default();
        assert!(s.values().is_empty(), "nothing before the first reading");
        for i in 0..75 {
            s.push(i as f32 / 100.0);
        }
        let v = s.values();
        assert_eq!(v.len(), GAUGE_HISTORY);
        assert_eq!((v[0], v[59]), (0.15, 0.74), "oldest first, the newest last");
        s.push(3.0);
        assert_eq!(s.values().last(), Some(&1.0), "clamped");
    }

    #[test]
    fn stats_numbers_warn_only_when_high_three_seconds_running() {
        let mut s = Series::default();
        s.push(0.95);
        s.push(0.97);
        assert_eq!(s.tone(), Tone::Primary, "two isn't sustained");
        s.push(0.9);
        assert_eq!(s.tone(), Tone::Warning);
        s.push(0.5);
        assert_eq!(s.tone(), Tone::Primary, "one quiet second ends it");
        for _ in 0..3 {
            s.push(1.0);
        }
        assert_eq!(s.tone(), Tone::Warning);
    }

    #[test]
    fn stats_history_skips_what_was_not_read() {
        let mut h = History::default();
        h.push(Some(0.2), Some(0.5), None, Some(0.0));
        h.push(Some(0.3), Some(0.5), None, None);
        assert_eq!((h.cpu.values(), h.gpu.values(), h.disk.values()), (vec![0.2, 0.3], vec![], vec![0.0]));
    }
}
