//! When a use shows and when it goes (PRD 5.3). Pure, with the clock passed in.
//!
//! - **Screen settles first.** A screenshot (Snipping Tool, a browser's tab picker preview)
//!   is a capture that starts and stops within milliseconds; only a capture still running
//!   after [`SCREEN_SETTLE`] is a screen being shared or recorded.
//! - **Camera, microphone and location show at once,** and a use that ends straight away
//!   stays for [`MIN_SHOWN`], so a single grabbed frame or a short recording is still seen.
//! - **Already running at start** (the hub started mid-call) shows at once, as nothing new.
//! - **A new app or kind while the indicator is up** is reported as started, for a brief
//!   moment; the first use needs none (the indicator itself is the news).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use super::consent::{App, Kind};

pub const SCREEN_SETTLE: Duration = Duration::from_secs(1);
pub const MIN_SHOWN: Duration = Duration::from_secs(3);

/// One live (or lingering) use as the module shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    pub kind: Kind,
    pub app: App,
    /// FILETIME the use started.
    pub since: u64,
    /// An exe to take the icon from.
    pub exe: Option<String>,
}

/// A use as reported by the watcher (see `alive.rs`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub kind: Kind,
    pub app: App,
    pub since: u64,
    pub exe: Option<String>,
}

#[derive(Debug, Clone)]
struct Seen {
    since: u64,
    exe: Option<String>,
    first_seen: Instant,
    shown_at: Option<Instant>,
    /// Gone from the store, kept until `shown_at + MIN_SHOWN`.
    ended: bool,
}

#[derive(Debug, Default)]
pub struct Tracker {
    seen: BTreeMap<(Kind, App), Seen>,
    started: bool,
}

impl Tracker {
    /// The watcher's latest view. Returns the uses that just started showing while others
    /// already were (a brief "started" each).
    pub fn update(&mut self, reports: &[Report], now: Instant) -> Vec<(Kind, App)> {
        let initial = !self.started;
        self.started = true;
        for (key, seen) in self.seen.iter_mut() {
            if !reports.iter().any(|r| (r.kind, &r.app) == (key.0, &key.1)) {
                seen.ended = true;
            }
        }
        for r in reports {
            let seen = self.seen.entry((r.kind, r.app.clone())).or_insert_with(|| Seen { since: r.since, exe: r.exe.clone(), first_seen: now, shown_at: initial.then_some(now), ended: false });
            // A new session of the same app (it stopped and started again) is still one dot.
            seen.ended = false;
            seen.since = r.since;
            if r.exe.is_some() {
                seen.exe = r.exe.clone();
            }
        }
        self.advance(now)
    }

    /// Time passed: settled screens show, lingering uses go. Same return as [`update`].
    pub fn tick(&mut self, now: Instant) -> Vec<(Kind, App)> {
        self.advance(now)
    }

    fn advance(&mut self, now: Instant) -> Vec<(Kind, App)> {
        self.seen.retain(|_, s| !(s.ended && s.shown_at.is_none_or(|at| now >= at + MIN_SHOWN)));
        // Uses already running when the hub started were shown on insert, so they're
        // never "started".
        let was_showing = self.seen.values().any(|s| s.shown_at.is_some());
        let mut started = Vec::new();
        for (key, s) in self.seen.iter_mut() {
            let settle = if key.0 == Kind::Screen { SCREEN_SETTLE } else { Duration::ZERO };
            if s.shown_at.is_none() && now >= s.first_seen + settle {
                s.shown_at = Some(now);
                started.push(key.clone());
            }
        }
        if was_showing {
            started
        } else {
            // The first use(s): the indicator appearing says it.
            Vec::new()
        }
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.seen
            .iter()
            .filter_map(|(key, s)| match s.shown_at {
                None => Some(s.first_seen + if key.0 == Kind::Screen { SCREEN_SETTLE } else { Duration::ZERO }),
                Some(at) if s.ended => Some(at + MIN_SHOWN),
                Some(_) => None,
            })
            .min()
    }

    /// What the indicator shows: most important kind first, then the most recent use.
    pub fn shown(&self) -> Vec<Shown> {
        let mut out: Vec<Shown> = self.seen.iter().filter(|(_, s)| s.shown_at.is_some()).map(|(k, s)| Shown { kind: k.0, app: k.1.clone(), since: s.since, exe: s.exe.clone() }).collect();
        out.sort_by(|a, b| a.kind.cmp(&b.kind).then(b.since.cmp(&a.since)));
        out
    }

    /// Forget a kind (switched off in Settings).
    pub fn drop_kind(&mut self, kind: Kind) {
        self.seen.retain(|k, _| k.0 != kind);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(kind: Kind, exe: &str, since: u64) -> Report {
        Report { kind, app: App::Exe(exe.into()), since, exe: Some(exe.into()) }
    }

    fn s(n: u64) -> Duration {
        Duration::from_millis(n * 1000)
    }

    #[test]
    fn privacy_running_at_start_shows_at_once_without_a_brief() {
        let (mut t, t0) = (Tracker::default(), Instant::now());
        let started = t.update(&[report(Kind::Screen, "obs64.exe", 5), report(Kind::Microphone, "zoom.exe", 9)], t0);
        assert!(started.is_empty());
        assert_eq!(t.shown().iter().map(|u| u.kind).collect::<Vec<_>>(), vec![Kind::Screen, Kind::Microphone]);
        assert_eq!(t.next_deadline(), None);
    }

    #[test]
    fn privacy_a_screenshot_never_shows_a_recording_does() {
        let (mut t, t0) = (Tracker::default(), Instant::now());
        t.update(&[], t0);
        t.update(&[report(Kind::Screen, "SnippingTool.exe", 1)], t0 + s(10));
        assert!(t.shown().is_empty());
        assert_eq!(t.next_deadline(), Some(t0 + s(10) + SCREEN_SETTLE));
        t.update(&[], t0 + s(10) + Duration::from_millis(30));
        assert!(t.shown().is_empty() && t.next_deadline().is_none(), "gone before it settled");

        t.update(&[report(Kind::Screen, "Teams.exe", 2)], t0 + s(20));
        assert!(t.tick(t0 + s(21)).is_empty(), "the first use is the indicator, not a brief");
        assert_eq!(t.shown().len(), 1);
    }

    #[test]
    fn privacy_short_uses_linger_and_new_apps_are_briefs() {
        let (mut t, t0) = (Tracker::default(), Instant::now());
        t.update(&[], t0);
        assert!(t.update(&[report(Kind::Microphone, "zoom.exe", 1)], t0 + s(1)).is_empty());
        let started = t.update(&[report(Kind::Microphone, "zoom.exe", 1), report(Kind::Camera, "zoom.exe", 2)], t0 + s(5));
        assert_eq!(started, vec![(Kind::Camera, App::Exe("zoom.exe".into()))]);
        assert_eq!(t.shown()[0].kind, Kind::Camera, "camera before microphone");

        // The camera goes off at once: still seen for MIN_SHOWN.
        t.update(&[report(Kind::Microphone, "zoom.exe", 1)], t0 + s(6));
        assert_eq!(t.shown().len(), 2);
        assert_eq!(t.next_deadline(), Some(t0 + s(5) + MIN_SHOWN));
        t.tick(t0 + s(5) + MIN_SHOWN);
        assert_eq!(t.shown().len(), 1);

        // A stopped-and-restarted session in between is the same dot.
        t.update(&[report(Kind::Microphone, "zoom.exe", 7)], t0 + s(9));
        assert_eq!(t.shown()[0].since, 7);
        t.drop_kind(Kind::Microphone);
        assert!(t.shown().is_empty());
    }
}
