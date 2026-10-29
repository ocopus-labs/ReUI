//! When a microphone use is a call (docs/PRD-calls.md 5.3). Pure, with the clock passed
//! in (Unix milliseconds, the clock the store's start times are in).
//!
//! No call app has a local API left to ask (Teams retired its in 2026), so a call is what
//! the system sees: a known call app holding the microphone.
//! - **It settles first.** Only a use that has lasted [`CALL_SETTLE`] by the store's own
//!   start time is a call, so a voice note's first seconds stay the privacy dot.
//! - **It ends with a grace.** Apps close and reopen the microphone while joining or
//!   switching devices; a release shorter than [`CALL_END`] is the same call, same start.
//! - **The camera and the screen are facts about a call**, never a call by themselves (a
//!   camera preview before joining isn't one).

use std::collections::BTreeMap;

use super::consent::{unix_ms, App, Kind};
use super::tracker::Report;

pub const CALL_SETTLE: u64 = 5_000;
pub const CALL_END: u64 = 3_000;

/// The call apps, grouped as Settings offers them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Family {
    Teams,
    Zoom,
    Discord,
    Slack,
    Webex,
    /// WhatsApp, Telegram, Signal.
    Messengers,
    /// Off by default: a browser holding the microphone may be a Meet call or a dictation
    /// site, and its tabs aren't read (PRD D2).
    Browsers,
}

/// Lowercased exe file names, then package family names (before the publisher id).
const EXES: &[(&str, Family)] = &[
    ("ms-teams.exe", Family::Teams),
    ("teams.exe", Family::Teams),
    ("zoom.exe", Family::Zoom),
    ("discord.exe", Family::Discord),
    ("discordptb.exe", Family::Discord),
    ("discordcanary.exe", Family::Discord),
    ("slack.exe", Family::Slack),
    ("ciscocollabhost.exe", Family::Webex),
    ("webexmta.exe", Family::Webex),
    ("atmgr.exe", Family::Webex),
    ("ciscowebexstart.exe", Family::Webex),
    ("whatsapp.exe", Family::Messengers),
    ("whatsapp.root.exe", Family::Messengers),
    ("telegram.exe", Family::Messengers),
    ("signal.exe", Family::Messengers),
    ("chrome.exe", Family::Browsers),
    ("msedge.exe", Family::Browsers),
    ("firefox.exe", Family::Browsers),
    ("brave.exe", Family::Browsers),
    ("opera.exe", Family::Browsers),
    ("vivaldi.exe", Family::Browsers),
    ("arc.exe", Family::Browsers),
];
const PACKAGES: &[(&str, Family)] = &[
    ("MSTeams", Family::Teams),
    ("5319275A.WhatsAppDesktop", Family::Messengers),
    ("TelegramMessengerLLP.TelegramDesktop", Family::Messengers),
    ("Zoom.Zoom", Family::Zoom),
];

/// Which call app this is, if any.
pub fn family(app: &App) -> Option<Family> {
    match app {
        App::Exe(_) => {
            let file = app.file_name()?;
            EXES.iter().find(|(exe, _)| *exe == file).map(|(_, f)| *f)
        }
        App::Packaged(family) => {
            let name = family.rsplit_once('_').map_or(family.as_str(), |(name, _)| name);
            PACKAGES.iter().find(|(p, _)| p.eq_ignore_ascii_case(name)).map(|(_, f)| *f)
        }
    }
}

/// A call as the card shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub app: App,
    pub family: Family,
    /// When the microphone first went on (Unix ms): the call's time counts from here.
    pub started_ms: u64,
    pub camera: bool,
    pub screen: bool,
    pub exe: Option<String>,
}

#[derive(Debug, Clone)]
struct Seen {
    call: Call,
    settled: bool,
    /// The microphone went away then; the call ends `CALL_END` later unless it's back.
    lost_at: Option<u64>,
}

#[derive(Debug, Default)]
pub struct Calls {
    seen: BTreeMap<App, Seen>,
}

impl Calls {
    /// The watcher's latest uses (every kind, before the indicator's own toggles). `allowed`
    /// says which families Settings counts.
    pub fn update(&mut self, reports: &[Report], allowed: impl Fn(Family) -> bool, now_ms: u64) {
        for r in reports.iter().filter(|r| r.kind == Kind::Microphone) {
            let Some(family) = family(&r.app).filter(|f| allowed(*f)) else { continue };
            let started = unix_ms(r.since);
            let seen = self.seen.entry(r.app.clone()).or_insert_with(|| Seen {
                call: Call { app: r.app.clone(), family, started_ms: started, camera: false, screen: false, exe: None },
                settled: false,
                lost_at: None,
            });
            // A new session of the same call keeps the call's start.
            seen.lost_at = None;
            seen.call.started_ms = seen.call.started_ms.min(started);
        }
        for (app, seen) in self.seen.iter_mut() {
            let mine = |kind: Kind| reports.iter().any(|r| r.kind == kind && &r.app == app);
            if !mine(Kind::Microphone) && seen.lost_at.is_none() {
                seen.lost_at = Some(now_ms);
            }
            seen.call.camera = mine(Kind::Camera);
            seen.call.screen = mine(Kind::Screen);
            if let Some(exe) = reports.iter().filter(|r| &r.app == app).find_map(|r| r.exe.clone()) {
                seen.call.exe = Some(exe);
            }
        }
        // A family switched off in Settings stops counting at once.
        self.seen.retain(|_, s| allowed(s.call.family));
        self.tick(now_ms);
    }

    /// Time passed: uses settle into calls, released ones end.
    pub fn tick(&mut self, now_ms: u64) {
        self.seen.retain(|_, s| match s.lost_at {
            // A use that never settled was never a call: gone at once.
            Some(_) if !s.settled => false,
            Some(at) => now_ms < at + CALL_END,
            None => true,
        });
        for s in self.seen.values_mut().filter(|s| s.lost_at.is_none()) {
            if now_ms >= s.call.started_ms + CALL_SETTLE {
                s.settled = true;
            }
        }
    }

    pub fn next_deadline(&self) -> Option<u64> {
        self.seen
            .values()
            .filter_map(|s| match s.lost_at {
                Some(at) => Some(at + CALL_END),
                None if !s.settled => Some(s.call.started_ms + CALL_SETTLE),
                None => None,
            })
            .min()
    }

    /// The calls now, the earliest first.
    pub fn calls(&self) -> Vec<Call> {
        let mut out: Vec<Call> = self.seen.values().filter(|s| s.settled).map(|s| s.call.clone()).collect();
        out.sort_by(|a, b| (a.started_ms, &a.app).cmp(&(b.started_ms, &b.app)));
        out
    }

    /// Whether `app` has a call, so the indicator leaves it to the call card.
    pub fn has_call(&self, app: &App) -> bool {
        self.seen.get(app).is_some_and(|s| s.settled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FILETIME of Unix `ms`.
    fn ft(ms: u64) -> u64 {
        (ms + 11_644_473_600_000) * 10_000
    }

    fn report(kind: Kind, exe: &str, since_ms: u64) -> Report {
        Report { kind, app: App::Exe(format!(r"C:\Apps\{exe}")), since: ft(since_ms), exe: Some(format!(r"C:\Apps\{exe}")) }
    }

    const T0: u64 = 1_800_000_000_000;

    #[test]
    fn privacy_call_apps_are_known_by_exe_and_package() {
        assert_eq!(family(&App::Exe(r"C:\Users\me\AppData\Local\Discord\app-1.0.9239\Discord.exe".into())), Some(Family::Discord));
        assert_eq!(family(&App::Exe(r"C:\Program Files\WindowsApps\x\ms-teams.exe".into())), Some(Family::Teams));
        assert_eq!(family(&App::Packaged("MSTeams_8wekyb3d8bbwe".into())), Some(Family::Teams));
        assert_eq!(family(&App::Packaged("5319275A.WhatsAppDesktop_cv1g1gvanyjgm".into())), Some(Family::Messengers));
        assert_eq!(family(&App::Exe(r"C:\Program Files\Google\Chrome\Application\chrome.exe".into())), Some(Family::Browsers));
        assert_eq!(family(&App::Exe(r"C:\Windows\System32\SoundRecorder.exe".into())), None);
        assert_eq!(family(&App::Packaged("Microsoft.WindowsCamera_8wekyb3d8bbwe".into())), None);
    }

    #[test]
    fn privacy_a_call_settles_and_counts_from_the_mic_start() {
        let mut c = Calls::default();
        c.update(&[report(Kind::Microphone, "Teams.exe", T0)], |_| true, T0 + 100);
        assert!(c.calls().is_empty(), "not yet: a voice note starts like this too");
        assert_eq!(c.next_deadline(), Some(T0 + CALL_SETTLE));
        c.tick(T0 + CALL_SETTLE);
        let calls = c.calls();
        assert_eq!((calls.len(), calls[0].started_ms, calls[0].family), (1, T0, Family::Teams));
        assert!(c.has_call(&calls[0].app));
        assert_eq!(c.next_deadline(), None);
    }

    #[test]
    fn privacy_a_short_use_is_never_a_call() {
        let mut c = Calls::default();
        c.update(&[report(Kind::Microphone, "WhatsApp.exe", T0)], |_| true, T0);
        c.update(&[], |_| true, T0 + 4_000);
        assert!(c.calls().is_empty() && c.next_deadline().is_none());
        // Not a call app, or one switched off: never.
        c.update(&[report(Kind::Microphone, "Audacity.exe", T0 - 60_000), report(Kind::Microphone, "chrome.exe", T0 - 60_000)], |f| f != Family::Browsers, T0 + 5_000);
        assert!(c.calls().is_empty());
    }

    #[test]
    fn privacy_running_at_start_is_a_call_at_once() {
        let mut c = Calls::default();
        c.update(&[report(Kind::Microphone, "Zoom.exe", T0 - 600_000), report(Kind::Camera, "Zoom.exe", T0 - 590_000)], |_| true, T0);
        let calls = c.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].camera && !calls[0].screen);
        // A camera alone isn't a call.
        let mut c = Calls::default();
        c.update(&[report(Kind::Camera, "Zoom.exe", T0 - 600_000)], |_| true, T0);
        assert!(c.calls().is_empty());
    }

    #[test]
    fn privacy_a_call_survives_a_short_release_and_ends_after_the_grace() {
        let mut c = Calls::default();
        c.update(&[report(Kind::Microphone, "Discord.exe", T0)], |_| true, T0 + 10_000);
        assert_eq!(c.calls().len(), 1);
        // Released while switching microphones...
        c.update(&[], |_| true, T0 + 20_000);
        assert_eq!(c.calls().len(), 1, "still the call during the grace");
        assert_eq!(c.next_deadline(), Some(T0 + 20_000 + CALL_END));
        // ...and back with a new session: same call, same start.
        c.update(&[report(Kind::Microphone, "Discord.exe", T0 + 21_000), report(Kind::Screen, "Discord.exe", T0 + 21_000)], |_| true, T0 + 21_000);
        let calls = c.calls();
        assert_eq!((calls[0].started_ms, calls[0].screen), (T0, true));
        // Left the call.
        c.update(&[], |_| true, T0 + 30_000);
        c.tick(T0 + 30_000 + CALL_END - 1);
        assert_eq!(c.calls().len(), 1);
        c.tick(T0 + 30_000 + CALL_END);
        assert!(c.calls().is_empty() && c.next_deadline().is_none());
    }

    #[test]
    fn privacy_switching_a_family_off_ends_its_call() {
        let mut c = Calls::default();
        let teams = [report(Kind::Microphone, "Teams.exe", T0)];
        c.update(&teams, |_| true, T0 + 10_000);
        assert_eq!(c.calls().len(), 1);
        c.update(&teams, |f| f != Family::Teams, T0 + 11_000);
        assert!(c.calls().is_empty());
    }
}
