//! The Capability Access Manager's consent store read as "what's in use now". Pure: the
//! registry walk (`store.rs`) hands over raw entries, this decides which are live.
//!
//! Windows records every use of a sensitive capability under
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\<capability>`:
//! a subkey per packaged app (its package family name) and, under `NonPackaged`, one per
//! desktop exe with `\` written as `#`. Each holds `LastUsedTimeStart` and
//! `LastUsedTimeStop` (FILETIMEs). An app is using the capability while its start is set
//! and its stop is 0 or older than the start. That's the same record Settings › Privacy
//! shows as "Currently in use", and it covers apps without package identity.

/// What's being used. Ordered by how much it matters that you know: a shared screen before
/// a camera before a microphone before location (the one shown first, and the accent).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Screen,
    Camera,
    Microphone,
    Location,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Screen, Kind::Camera, Kind::Microphone, Kind::Location];

    /// The consent store's capability keys for this kind. Screen capture is two:
    /// `graphicsCaptureProgrammatic` (Windows.Graphics.Capture, with the border) and
    /// `graphicsCaptureWithoutBorder` (the same capture, border waived).
    pub fn capabilities(self) -> &'static [&'static str] {
        match self {
            Kind::Screen => &["graphicsCaptureProgrammatic", "graphicsCaptureWithoutBorder"],
            Kind::Camera => &["webcam"],
            Kind::Microphone => &["microphone"],
            Kind::Location => &["location"],
        }
    }
}

/// Who used it: a packaged app's family name, or a desktop app's full exe path.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum App {
    Packaged(String),
    Exe(String),
}

impl App {
    /// A subkey under a capability: `NonPackaged` children are exe paths with `#` for `\`.
    pub fn from_key(name: &str, non_packaged: bool) -> App {
        if non_packaged {
            App::Exe(name.replace('#', "\\"))
        } else {
            App::Packaged(name.to_owned())
        }
    }

    /// Lowercased exe file name, for matching processes; `None` for packaged apps.
    pub fn file_name(&self) -> Option<String> {
        match self {
            App::Exe(path) => Some(path.rsplit(['\\', '/']).next().unwrap_or(path).to_lowercase()),
            App::Packaged(_) => None,
        }
    }
}

/// One subkey's record, as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub kind: Kind,
    pub app: App,
    pub start: u64,
    pub stop: u64,
}

/// An app using one kind now, since `since` (a FILETIME).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Use {
    pub kind: Kind,
    pub app: App,
    pub since: u64,
}

/// Started and not stopped since. A stop older than the start is the previous session's.
pub fn in_use(entry: &Entry) -> bool {
    entry.start != 0 && (entry.stop == 0 || entry.stop < entry.start)
}

/// The live entries, one per kind and app (screen capture's two keys merge, earliest start
/// kept), minus ReUI's own programs (`own`): the island's visualizer captures loopback
/// audio and must never show as a microphone.
pub fn uses(entries: &[Entry], own: impl Fn(&App) -> bool) -> Vec<Use> {
    let mut out: Vec<Use> = Vec::new();
    for e in entries.iter().filter(|e| in_use(e) && !own(&e.app)) {
        match out.iter_mut().find(|u| u.kind == e.kind && u.app == e.app) {
            Some(u) => u.since = u.since.min(e.start),
            None => out.push(Use { kind: e.kind, app: e.app.clone(), since: e.start }),
        }
    }
    out.sort_by(|a, b| (a.kind, a.since, &a.app).cmp(&(b.kind, b.since, &b.app)));
    out
}

/// Whether an app is one of ReUI's: an exe in `own_dir` (where the hub runs from) or any
/// `reui-*.exe` (a dev build elsewhere).
pub fn is_own(app: &App, own_dir: &str) -> bool {
    let App::Exe(path) = app else { return false };
    let lower = path.to_lowercase();
    let dir = own_dir.trim_end_matches('\\').to_lowercase();
    let in_dir = !dir.is_empty() && lower.rsplit_once('\\').is_some_and(|(parent, _)| parent == dir);
    in_dir || app.file_name().is_some_and(|f| f.starts_with("reui-"))
}

/// FILETIME (100 ns since 1601) → Unix milliseconds.
pub fn unix_ms(filetime: u64) -> u64 {
    const EPOCH_DIFF_MS: u64 = 11_644_473_600_000;
    (filetime / 10_000).saturating_sub(EPOCH_DIFF_MS)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Recorded from a Windows 11 25H2 (26200) consent store: Steam's mic session long over,
    /// Chrome on the camera now, Discord sharing the screen (both capture keys), the
    /// Camera app with no times (permission only), the island's own loopback.
    pub fn recorded() -> Vec<Entry> {
        let exe = |p: &str| App::from_key(p, true);
        vec![
            Entry { kind: Kind::Microphone, app: exe("C:#Program Files (x86)#Steam#steam.exe"), start: 134318160398015841, stop: 134318160403059668 },
            Entry { kind: Kind::Camera, app: exe("C:#Program Files#Google#Chrome#Application#chrome.exe"), start: 134377500000000000, stop: 134342864837626347 },
            Entry { kind: Kind::Camera, app: App::from_key("Microsoft.WindowsCamera_8wekyb3d8bbwe", false), start: 0, stop: 0 },
            Entry { kind: Kind::Screen, app: exe("C:#Users#me#AppData#Local#Discord#app-1.0.9239#Discord.exe"), start: 134377500100000000, stop: 0 },
            Entry { kind: Kind::Screen, app: exe("C:#Users#me#AppData#Local#Discord#app-1.0.9239#Discord.exe"), start: 134377500050000000, stop: 0 },
            Entry { kind: Kind::Microphone, app: exe("C:#Program Files#ReUI#reui-island.exe"), start: 134377500000000000, stop: 0 },
        ]
    }

    #[test]
    fn privacy_live_entries_are_started_and_not_stopped_since() {
        let e = |start, stop| Entry { kind: Kind::Microphone, app: App::Exe("a.exe".into()), start, stop };
        assert!(in_use(&e(10, 0)));
        assert!(in_use(&e(10, 5)), "the stop is the last session's");
        assert!(!in_use(&e(10, 10)) && !in_use(&e(10, 12)));
        assert!(!in_use(&e(0, 0)), "permission only, never used");
    }

    #[test]
    fn privacy_recorded_store_reads_as_camera_and_screen() {
        let live = uses(&recorded(), |a| is_own(a, r"C:\Program Files\ReUI"));
        assert_eq!(live.len(), 2, "{live:?}");
        assert_eq!(live[0].kind, Kind::Screen);
        assert_eq!(live[0].since, 134377500050000000, "two capture keys merge, earliest start");
        assert_eq!(live[0].app.file_name().as_deref(), Some("discord.exe"));
        assert_eq!((live[1].kind, live[1].app.clone()), (Kind::Camera, App::Exe(r"C:\Program Files\Google\Chrome\Application\chrome.exe".into())));
    }

    #[test]
    fn privacy_reui_never_counts() {
        assert!(is_own(&App::Exe(r"C:\Users\me\AppData\Local\Programs\ReUI\reui-island.exe".into()), r"C:\Users\me\AppData\Local\Programs\ReUI\"));
        assert!(is_own(&App::Exe(r"D:\ReUI\target\debug\reui-island.exe".into()), r"C:\elsewhere"));
        assert!(is_own(&App::Exe(r"C:\Apps\ReUI\helper.exe".into()), r"c:\apps\reui"));
        assert!(!is_own(&App::Exe(r"C:\Apps\ReUI\sub\helper.exe".into()), r"C:\Apps\ReUI"));
        assert!(!is_own(&App::Exe(r"C:\Program Files\Zoom\bin\Zoom.exe".into()), ""));
        assert!(!is_own(&App::Packaged("Microsoft.WindowsCamera_8wekyb3d8bbwe".into()), r"C:\x"));
    }

    #[test]
    fn privacy_kinds_and_times() {
        assert_eq!(Kind::Screen.capabilities(), ["graphicsCaptureProgrammatic", "graphicsCaptureWithoutBorder"]);
        assert!(Kind::ALL.windows(2).all(|w| w[0] < w[1]), "most important first");
        assert_eq!(unix_ms(116_444_736_000_000_000), 0);
        assert_eq!(unix_ms(134_374_853_892_141_459), 1_793_011_789_214);
    }
}
