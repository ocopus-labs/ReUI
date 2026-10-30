//! The script activities being shown, as plain state on the module's millisecond clock (the
//! caller passes it in), so every rule is tested without waiting: how an update merges
//! into what's shown, how many each source may have, when a finished one goes, when one
//! nobody updates any more ends by itself, and how often the island may be told.

use super::request::{Glyph, Phase, Update};

/// A finished one stays this long unless the script says otherwise.
pub const DONE_LINGER_MS: u64 = 6_000;
/// A failure stays longer: it's the one worth reading.
pub const FAILED_LINGER_MS: u64 = 20_000;
/// At most this many at once, from every source together.
pub const MAX_TOTAL: usize = 8;
/// The island hears of updates at most this often (a script may send hundreds a second).
pub const MIN_GAP_MS: u64 = 250;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub per_source: usize,
    /// No update for this long ends a running one (unless it's kept).
    pub stale_ms: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: String,
    pub source: String,
    pub title: Option<String>,
    pub detail: Option<String>,
    pub progress: Option<f32>,
    pub phase: Phase,
    pub glyph: Glyph,
    pub keep: bool,
    /// When it was first shown (for its count-up).
    pub started: u64,
    /// When it was last heard from.
    pub heard: u64,
    /// When it finished, on the module clock (for "took 3 min").
    pub finished: Option<u64>,
    /// When a finished one goes.
    pub gone_at: Option<u64>,
}

/// What an update did, for the module's log.
#[derive(Debug, PartialEq)]
pub struct Applied {
    /// It finished just now (shown as a Brief from here).
    pub finished: bool,
    /// Ones that made way for it (a source or the board was full).
    pub evicted: Vec<String>,
}

#[derive(Debug, Default)]
pub struct Board {
    entries: Vec<Entry>,
}

impl Board {
    pub fn all(&self) -> &[Entry] {
        &self.entries
    }

    /// Merges an update into what's shown, or shows a new one. A full source (or board)
    /// lets its oldest go: the newest thing a script says wins.
    pub fn apply(&mut self, u: Update, limits: Limits, now: u64) -> Applied {
        let mut evicted = Vec::new();
        let at = match self.entries.iter().position(|e| e.id == u.id) {
            Some(at) => at,
            None => {
                while self.entries.iter().filter(|e| e.source == u.source).count() >= limits.per_source.max(1) {
                    evicted.extend(self.evict(|e| e.source == u.source));
                }
                while self.entries.len() >= MAX_TOTAL {
                    evicted.extend(self.evict(|_| true));
                }
                self.entries.push(Entry {
                    id: u.id.clone(),
                    source: u.source.clone(),
                    title: None,
                    detail: None,
                    progress: None,
                    phase: Phase::Running,
                    glyph: Glyph::Script,
                    keep: false,
                    started: now,
                    heard: now,
                    finished: None,
                    gone_at: None,
                });
                self.entries.len() - 1
            }
        };
        let e = &mut self.entries[at];
        // A finished one that's started again is a new run.
        if e.phase != Phase::Running && u.phase == Phase::Running {
            (e.started, e.finished, e.gone_at, e.progress) = (now, None, None, None);
        }
        let finished = e.phase == Phase::Running && u.phase != Phase::Running;
        e.title = u.title.or(e.title.take());
        e.detail = u.detail.or(e.detail.take());
        e.progress = u.progress.or(e.progress);
        e.glyph = u.glyph.unwrap_or(e.glyph);
        e.keep = u.keep;
        e.heard = now;
        e.phase = u.phase;
        if u.phase != Phase::Running {
            if finished {
                e.finished = Some(now);
            }
            let linger = u.linger_ms.unwrap_or(if u.phase == Phase::Done { DONE_LINGER_MS } else { FAILED_LINGER_MS });
            e.gone_at = Some(now + linger);
        }
        Applied { finished, evicted }
    }

    /// The oldest matching entry goes (running ones before finished ones keep their Brief).
    fn evict(&mut self, matching: impl Fn(&Entry) -> bool) -> Option<String> {
        let at = self.entries.iter().enumerate().filter(|(_, e)| matching(e)).min_by_key(|(_, e)| (e.phase == Phase::Running, e.heard)).map(|(i, _)| i)?;
        Some(self.entries.remove(at).id)
    }

    pub fn end(&mut self, id: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.id != id);
        self.entries.len() != before
    }

    pub fn clear(&mut self) -> Vec<String> {
        self.entries.drain(..).map(|e| e.id).collect()
    }

    /// Takes away finished ones whose time is up and running ones nobody has updated for
    /// `stale_ms` (unless kept). Returns their ids.
    pub fn expire(&mut self, limits: Limits, now: u64) -> Vec<String> {
        let gone = |e: &Entry| e.gone_at.is_some_and(|at| at <= now) || (e.phase == Phase::Running && !e.keep && now.saturating_sub(e.heard) >= limits.stale_ms);
        let ids: Vec<String> = self.entries.iter().filter(|e| gone(e)).map(|e| e.id.clone()).collect();
        self.entries.retain(|e| !gone(e));
        ids
    }

    /// When something next needs taking away.
    pub fn next_due(&self, limits: Limits) -> Option<u64> {
        self.entries.iter().filter_map(|e| e.gone_at.or((e.phase == Phase::Running && !e.keep).then(|| e.heard + limits.stale_ms))).min()
    }
}

/// How often the island may be told: at once when it's been a while, otherwise once at
/// the end of the gap with everything that came in meanwhile.
#[derive(Debug, Default)]
pub struct Pace {
    last: Option<u64>,
    pending: bool,
}

impl Pace {
    /// Whether to publish now; if not, [`Pace::due`] says when.
    pub fn ask(&mut self, now: u64) -> bool {
        if self.last.is_none_or(|last| now >= last + MIN_GAP_MS) {
            self.last = Some(now);
            self.pending = false;
            true
        } else {
            self.pending = true;
            false
        }
    }

    /// Published for some other reason (a finish, an end): the gap starts again.
    pub fn published(&mut self, now: u64) {
        self.last = Some(now);
        self.pending = false;
    }

    pub fn due(&self) -> Option<u64> {
        self.pending.then(|| self.last.unwrap_or(0) + MIN_GAP_MS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: Limits = Limits { per_source: 3, stale_ms: 600_000 };

    fn update(id: &str, source: &str) -> Update {
        Update { id: id.into(), source: source.into(), title: None, detail: None, progress: None, phase: Phase::Running, glyph: None, linger_ms: None, keep: false }
    }

    #[test]
    fn updates_merge_and_finish() {
        let mut b = Board::default();
        b.apply(Update { title: Some("Build".into()), progress: Some(0.3), glyph: Some(Glyph::Build), ..update("build", "pwsh") }, LIMITS, 0);
        b.apply(Update { detail: Some("Linking".into()), ..update("build", "pwsh") }, LIMITS, 5_000);
        let e = &b.all()[0];
        assert_eq!((e.title.as_deref(), e.detail.as_deref(), e.progress, e.glyph, e.heard), (Some("Build"), Some("Linking"), Some(0.3), Glyph::Build, 5_000));
        let done = b.apply(Update { phase: Phase::Done, ..update("build", "pwsh") }, LIMITS, 9_000);
        assert!(done.finished);
        assert_eq!(b.next_due(LIMITS), Some(9_000 + DONE_LINGER_MS));
        assert!(b.expire(LIMITS, 9_000 + DONE_LINGER_MS - 1).is_empty());
        assert_eq!(b.expire(LIMITS, 9_000 + DONE_LINGER_MS), ["build"]);
        assert!(b.all().is_empty());
    }

    #[test]
    fn a_failure_stays_longer_unless_told() {
        let mut b = Board::default();
        b.apply(Update { phase: Phase::Failed, ..update("a", "s") }, LIMITS, 0);
        b.apply(Update { phase: Phase::Done, linger_ms: Some(1_000), ..update("b", "s") }, LIMITS, 0);
        assert_eq!(b.all().iter().map(|e| e.gone_at).collect::<Vec<_>>(), [Some(FAILED_LINGER_MS), Some(1_000)]);
        // Started again: a new run with its own clock.
        b.apply(update("a", "s"), LIMITS, 2_000);
        let a = &b.all()[0];
        assert_eq!((a.phase, a.started, a.gone_at), (Phase::Running, 2_000, None));
    }

    #[test]
    fn a_script_that_vanished_ends_unless_kept() {
        let mut b = Board::default();
        b.apply(update("lost", "s"), LIMITS, 0);
        b.apply(Update { keep: true, ..update("kept", "s") }, LIMITS, 0);
        assert_eq!(b.next_due(LIMITS), Some(LIMITS.stale_ms));
        assert_eq!(b.expire(LIMITS, LIMITS.stale_ms), ["lost"]);
        assert_eq!(b.next_due(LIMITS), None, "a kept one never times out");
        assert!(b.end("kept"));
        assert!(!b.end("kept"));
    }

    #[test]
    fn a_full_source_lets_its_oldest_go() {
        let mut b = Board::default();
        for (i, id) in ["a", "b", "c"].iter().enumerate() {
            b.apply(update(id, "noisy"), LIMITS, i as u64);
        }
        b.apply(update("other", "calm"), LIMITS, 10);
        let applied = b.apply(update("d", "noisy"), LIMITS, 20);
        assert_eq!(applied.evicted, ["a"]);
        assert_eq!(b.all().iter().filter(|e| e.source == "noisy").count(), 3);
        // The whole board has room for MAX_TOTAL.
        for i in 0..20 {
            b.apply(update(&format!("s{i}"), &format!("src{i}")), LIMITS, 100 + i);
        }
        assert_eq!(b.all().len(), MAX_TOTAL);
        assert_eq!(b.clear().len(), MAX_TOTAL);
    }

    #[test]
    fn the_island_hears_four_times_a_second_at_most() {
        let mut p = Pace::default();
        assert!(p.ask(0));
        assert!(!p.ask(10));
        assert!(!p.ask(200));
        assert_eq!(p.due(), Some(MIN_GAP_MS));
        assert!(p.ask(MIN_GAP_MS));
        assert_eq!(p.due(), None);
        p.published(1_000);
        assert!(!p.ask(1_100));
    }
}
