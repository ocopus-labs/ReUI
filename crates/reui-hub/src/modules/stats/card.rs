//! When the card is open, and so when the sampler runs. Pure (the clock is passed in).
//!
//! "Open" means the island said so: it sends `expanded` when the card opens and `collapsed`
//! when it closes for any reason (dismissed, the hotkey again, another app in front, a
//! fullscreen game hiding the island). Two things would otherwise leave the sampler running
//! with nothing on screen, so both have a deadline: the island not opening the card at all
//! (it's hidden, or ignored the request), and the island going away while it's open (a
//! crash, so no `collapsed` ever comes).

use std::time::{Duration, Instant};

/// The sampler's first reading must come this soon, or it isn't working.
const FIRST_READING: Duration = Duration::from_secs(2);
/// The island must open the card this soon after it's asked to.
const TO_OPEN: Duration = Duration::from_secs(3);
/// An open card closes by itself after this, in case its `collapsed` never comes. Longer
/// than anyone watches the numbers; the island closes an untouched card after a minute.
pub const LONGEST: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Closed,
    /// The sampler was started; the card shows with its first reading.
    Starting,
    /// Published and asked for; waiting for the island to open it.
    Asked,
    Open,
}

/// What the module should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Do {
    Nothing,
    /// Start a sampler for this run.
    Start(u32),
    /// Show the reading.
    Publish,
    /// Show the reading and ask the island to open the card.
    PublishAndOpen,
    /// Ask the island again: an open card asked again closes (the hotkey toggles).
    Toggle,
    /// Stop the sampler and take the card away.
    Close,
}

#[derive(Debug)]
pub struct Card {
    phase: Phase,
    run: u32,
    until: Option<Instant>,
}

impl Default for Card {
    fn default() -> Self {
        Self { phase: Phase::Closed, run: 0, until: None }
    }
}

impl Card {
    /// The hotkey, the tray or a command.
    pub fn open(&mut self, now: Instant) -> Do {
        match self.phase {
            Phase::Closed => {
                self.run = self.run.wrapping_add(1);
                self.set(Phase::Starting, now + FIRST_READING);
                Do::Start(self.run)
            }
            Phase::Starting => Do::Nothing,
            Phase::Asked | Phase::Open => Do::Toggle,
        }
    }

    /// A reading from sampler run `run`.
    pub fn reading(&mut self, run: u32, now: Instant) -> Do {
        if run != self.run {
            return Do::Nothing;
        }
        match self.phase {
            Phase::Starting => {
                self.set(Phase::Asked, now + TO_OPEN);
                Do::PublishAndOpen
            }
            Phase::Asked | Phase::Open => Do::Publish,
            Phase::Closed => Do::Nothing,
        }
    }

    pub fn expanded(&mut self, now: Instant) {
        if matches!(self.phase, Phase::Asked | Phase::Open) {
            self.set(Phase::Open, now + LONGEST);
        }
    }

    pub fn collapsed(&mut self) -> Do {
        match self.phase {
            Phase::Asked | Phase::Open => self.close(),
            _ => Do::Nothing,
        }
    }

    /// When [`Card::due`] should be called next.
    pub fn deadline(&self) -> Option<Instant> {
        self.until
    }

    /// A deadline passed: whatever was waiting didn't happen.
    pub fn due(&mut self, now: Instant) -> Do {
        match self.until {
            Some(until) if now >= until => self.close(),
            _ => Do::Nothing,
        }
    }

    /// Closed by the module itself (turned off in Settings).
    pub fn close(&mut self) -> Do {
        if self.phase == Phase::Closed {
            return Do::Nothing;
        }
        self.phase = Phase::Closed;
        self.until = None;
        Do::Close
    }

    #[cfg(test)]
    pub fn is_open(&self) -> bool {
        self.phase != Phase::Closed
    }

    fn set(&mut self, phase: Phase, until: Instant) {
        self.phase = phase;
        self.until = Some(until);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_card_samples_only_between_open_and_collapsed() {
        let t = Instant::now();
        let mut card = Card::default();
        assert_eq!(card.open(t), Do::Start(1));
        assert_eq!(card.open(t), Do::Nothing, "already starting");
        assert_eq!(card.reading(1, t), Do::PublishAndOpen);
        card.expanded(t);
        assert_eq!(card.reading(1, t), Do::Publish);
        // The hotkey again: the island closes it, then says so.
        assert_eq!(card.open(t), Do::Toggle);
        assert_eq!(card.collapsed(), Do::Close);
        assert!(!card.is_open() && card.deadline().is_none());
        // A reading still queued from that run is dropped.
        assert_eq!(card.reading(1, t), Do::Nothing);
        assert_eq!(card.collapsed(), Do::Nothing);
        // The next run is a new one.
        assert_eq!(card.open(t), Do::Start(2));
        assert_eq!(card.reading(1, t), Do::Nothing);
    }

    #[test]
    fn stats_card_gives_up_when_nothing_shows_it() {
        let t = Instant::now();
        let mut card = Card::default();
        // The sampler never reads.
        card.open(t);
        assert_eq!(card.due(t + Duration::from_secs(1)), Do::Nothing);
        assert_eq!(card.due(t + FIRST_READING), Do::Close);

        // The island is hidden (fullscreen) and never opens it.
        card.open(t);
        card.reading(2, t);
        assert_eq!(card.deadline(), Some(t + TO_OPEN));
        assert_eq!(card.due(t + TO_OPEN), Do::Close);

        // The island goes away while it's open: closed after the longest.
        card.open(t);
        card.reading(3, t);
        card.expanded(t);
        assert_eq!(card.due(t + TO_OPEN), Do::Nothing);
        assert_eq!(card.due(t + LONGEST), Do::Close);
    }
}
