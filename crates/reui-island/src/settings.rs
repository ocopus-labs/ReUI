//! User settings for the island, stored as JSON in `%APPDATA%\ReUI\settings.json`
//! (`REUI_CONFIG_DIR\roaming` when that's set, for testing). Written by the Settings window
//! and the tray (pause); the island watches the folder ([`watch`]) and applies changes as
//! they land. Unknown or missing fields fall back to defaults; keys this version doesn't
//! know are kept when saving, and a file that isn't a JSON object is never overwritten.
//!
//! Keys: `visibility` ("always" | "when-active" | "when-needed"), `paused_until` (Unix ms),
//! `hide_in_fullscreen` (bool, default true), `show_on_desktop` (bool, default true), `monitor` ("primary" | "cursor" | a display's
//! device name such as `\\.\DISPLAY2`).

use std::path::{Path, PathBuf};

use reui_protocol::{Activity, Priority};

use crate::place::MonitorChoice;
use serde_json::{Map, Value};
use windows::core::HSTRING;
use windows::Win32::Foundation::{HWND, LPARAM, WAIT_OBJECT_0, WPARAM};
use windows::Win32::Storage::FileSystem::{FindFirstChangeNotificationW, FindNextChangeNotification, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE};
use windows::Win32::System::Threading::{WaitForSingleObject, INFINITE};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

/// Posted to the island window when something in the settings folder changed.
pub const WM_SETTINGS_CHANGED: u32 = WM_APP + 4;

/// How long "Pause ReUI" lasts.
pub const PAUSE_MS: u64 = 3_600_000;

/// When the island shows at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Visibility {
    /// The resting pill is always there.
    #[default]
    Always,
    /// Hidden while nothing is happening.
    WhenActive,
    /// Hidden unless something needs you: a decision, a question, a finished or failed task,
    /// or a brief confirmation of something you just did (a volume key, a copy).
    WhenNeeded,
}

impl Visibility {
    fn as_str(self) -> &'static str {
        match self {
            Visibility::Always => "always",
            Visibility::WhenActive => "when-active",
            Visibility::WhenNeeded => "when-needed",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "always" => Some(Visibility::Always),
            "when-active" => Some(Visibility::WhenActive),
            "when-needed" => Some(Visibility::WhenNeeded),
            _ => None,
        }
    }

    /// Whether the island shows for these activities (before reveal and fullscreen, which
    /// the pill applies). A Brief counts as needed: a change the user just made, like the
    /// volume, should answer them even here (PRD D5).
    pub fn shows(self, activities: &[Activity]) -> bool {
        match self {
            Visibility::Always => true,
            Visibility::WhenActive => !activities.is_empty(),
            Visibility::WhenNeeded => activities.iter().any(|a| a.priority >= Priority::Alert || a.priority == Priority::Brief),
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct Settings {
    pub visibility: Visibility,
    /// "Pause ReUI": the island stays hidden (and clipboard recording is paused) until this
    /// Unix time in ms. A time in the past means not paused.
    pub paused_until: Option<u64>,
    /// Hide while a game, video or presentation is fullscreen.
    pub hide_in_fullscreen: bool,
    /// While the desktop is in front, show as if `visibility` were Always: in apps the island
    /// keeps out of the way, on the desktop there's nothing for it to cover.
    pub show_on_desktop: bool,
    /// Which display the island sits on.
    pub monitor: MonitorChoice,
}

impl Default for Settings {
    fn default() -> Self {
        Self { visibility: Visibility::default(), paused_until: None, hide_in_fullscreen: true, show_on_desktop: true, monitor: MonitorChoice::default() }
    }
}

impl Settings {
    pub fn dir() -> Option<PathBuf> {
        let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
        match var("REUI_CONFIG_DIR") {
            Some(root) => Some(root.join("roaming")),
            None => var("APPDATA").map(|dir| dir.join("ReUI")),
        }
    }

    fn path() -> Option<PathBuf> {
        Self::dir().map(|dir| dir.join("settings.json"))
    }

    pub fn load() -> Self {
        Self::path().and_then(|p| std::fs::read_to_string(p).ok()).map(|text| Self::parse(&text)).unwrap_or_default()
    }

    pub fn parse(text: &str) -> Self {
        let Ok(Value::Object(map)) = serde_json::from_str::<Value>(text) else { return Self::default() };
        Self {
            visibility: map.get("visibility").and_then(Value::as_str).and_then(Visibility::parse).unwrap_or_default(),
            paused_until: map.get("paused_until").and_then(Value::as_u64),
            hide_in_fullscreen: map.get("hide_in_fullscreen").and_then(Value::as_bool).unwrap_or(true),
            show_on_desktop: map.get("show_on_desktop").and_then(Value::as_bool).unwrap_or(true),
            monitor: map.get("monitor").and_then(Value::as_str).map(MonitorChoice::parse).unwrap_or_default(),
        }
    }

    /// Paused at `now` (Unix ms): how long is left.
    pub fn pause_left(&self, now: u64) -> Option<u64> {
        self.paused_until.filter(|&until| until > now).map(|until| until - now)
    }

    pub fn save(&self) {
        let Some(path) = Self::path() else { return };
        if let Err(e) = self.save_to(&path) {
            eprintln!("island: can't save settings to {}: {e}", path.display());
        }
    }

    /// Writes our keys over the file's, keeping the rest, atomically (temp file + rename).
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        let mut map = match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<Value>(&text) {
                Ok(Value::Object(map)) => map,
                _ => return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "not a JSON object; left as it is")),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Map::new(),
            Err(e) => return Err(e),
        };
        map.insert("visibility".into(), Value::String(self.visibility.as_str().into()));
        match self.paused_until {
            Some(until) => map.insert("paused_until".into(), Value::from(until)),
            None => map.remove("paused_until"),
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, serde_json::to_string_pretty(&Value::Object(map)).unwrap_or_default() + "\n")?;
        std::fs::rename(&temp, path)
    }
}

/// Watches the settings folder on a thread of its own and posts [`WM_SETTINGS_CHANGED`] to
/// `hwnd` on every change there (the clipboard database lives there too, so the island
/// re-reads the small file and ignores what didn't change).
pub fn watch(hwnd: HWND) {
    let Some(dir) = Settings::dir() else { return };
    _ = std::fs::create_dir_all(&dir);
    let hwnd = hwnd.0 as isize;
    _ = std::thread::Builder::new().name("settings-watch".into()).spawn(move || unsafe {
        let Ok(handle) = FindFirstChangeNotificationW(&HSTRING::from(dir.as_os_str()), false, FILE_NOTIFY_CHANGE_LAST_WRITE | FILE_NOTIFY_CHANGE_FILE_NAME) else {
            eprintln!("island: can't watch {} for settings changes", dir.display());
            return;
        };
        while WaitForSingleObject(handle, INFINITE) == WAIT_OBJECT_0 {
            if PostMessageW(Some(HWND(hwnd as *mut _)), WM_SETTINGS_CHANGED, WPARAM(0), LPARAM(0)).is_err() {
                return; // the window is gone
            }
            if FindNextChangeNotification(handle).is_err() {
                return;
            }
        }
    });
}

pub fn now_ms() -> u64 {
    reui_protocol::unix_ms()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_what_it_writes_and_tolerates_junk() {
        let s = Settings { visibility: Visibility::WhenNeeded, paused_until: Some(5), ..Settings::default() };
        assert_eq!(Settings::parse(r#"{"visibility":"when-needed","paused_until":5}"#), s);
        let full = Settings::parse(r#"{"hide_in_fullscreen":false,"monitor":"cursor"}"#);
        assert_eq!((full.hide_in_fullscreen, full.monitor), (false, MonitorChoice::Cursor));
        assert_eq!(Settings::parse(r#"{"monitor":"\\\\.\\DISPLAY2"}"#).monitor, MonitorChoice::Named(r"\\.\DISPLAY2".into()));
        assert!(Settings::default().hide_in_fullscreen);
        // Missing or extra fields fall back to defaults instead of failing.
        assert_eq!(Settings::parse(r#"{"other":1}"#), Settings::default());
        assert_eq!(Settings::parse(r#"{"visibility":"sometimes"}"#), Settings::default());
        assert_eq!(Settings::parse("not json"), Settings::default());
    }

    #[test]
    fn saving_keeps_other_keys_and_never_clobbers_a_broken_file() {
        let dir = std::env::temp_dir().join(format!("reui-island-settings-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("settings.json");
        let s = Settings { visibility: Visibility::WhenActive, paused_until: Some(42), ..Settings::default() };
        s.save_to(&path).unwrap();
        assert_eq!(Settings::parse(&std::fs::read_to_string(&path).unwrap()), s);
        std::fs::write(&path, r#"{ "future": [1], "visibility": "always", "paused_until": 9 }"#).unwrap();
        Settings { visibility: Visibility::WhenNeeded, paused_until: None, ..Settings::default() }.save_to(&path).unwrap();
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!((saved["future"][0].as_u64(), saved["visibility"].as_str(), saved.get("paused_until")), (Some(1), Some("when-needed"), None));
        std::fs::write(&path, "{ broken").unwrap();
        assert!(s.save_to(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ broken");
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pause_counts_down_and_ends() {
        let s = Settings { paused_until: Some(1_000), ..Settings::default() };
        assert_eq!(s.pause_left(400), Some(600));
        assert_eq!(s.pause_left(1_000), None);
        assert_eq!(Settings::default().pause_left(0), None);
    }

    #[test]
    fn when_needed_shows_for_alerts_decisions_and_briefs() {
        let with = |priorities: &[Priority]| -> Vec<Activity> {
            priorities
                .iter()
                .map(|&priority| Activity {
                    id: reui_protocol::ActivityId { module: "m".into(), key: format!("{priority:?}") },
                    priority,
                    accent: None,
                    compact: Default::default(),
                    minimal: reui_protocol::Element::Empty,
                    audio: None,
                    expanded: None,
                })
                .collect()
        };
        let needed = |p: &[Priority]| Visibility::WhenNeeded.shows(&with(p));
        assert!(!needed(&[]));
        assert!(!needed(&[Priority::Passive, Priority::Ongoing]));
        assert!(needed(&[Priority::Ongoing, Priority::Brief]));
        assert!(needed(&[Priority::Alert]));
        assert!(needed(&[Priority::Decision]));
        assert!(Visibility::WhenActive.shows(&with(&[Priority::Passive])));
        assert!(!Visibility::WhenActive.shows(&[]));
        assert!(Visibility::Always.shows(&[]));
    }
}
