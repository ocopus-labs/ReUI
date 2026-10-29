//! Privacy indicator settings, in `%APPDATA%\ReUI\privacy.json`, written by the Settings
//! window too (which then sends the module `reload`). Same rules as Devices': created with
//! the defaults when missing, a missing or mistyped field falls back to its default, keys
//! this version doesn't know are kept, and a file that isn't JSON is never overwritten.
//! `enabled` is read by `main.rs` at start.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use super::calls::Family;
use super::consent::Kind;

/// Call apps by family, their keys and defaults. Browsers are off by default (PRD-calls
/// D2): a browser holding the microphone isn't necessarily a call.
const CALL_KEYS: [(Family, &str, bool); 7] = [
    (Family::Teams, "call_teams", true),
    (Family::Zoom, "call_zoom", true),
    (Family::Discord, "call_discord", true),
    (Family::Slack, "call_slack", true),
    (Family::Webex, "call_webex", true),
    (Family::Messengers, "call_messengers", true),
    (Family::Browsers, "call_browsers", false),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub microphone: bool,
    pub camera: bool,
    pub screen: bool,
    /// Off by default: Windows' own widgets and Weather hold location for long stretches,
    /// which would keep the island busy for something rarely surprising (D2).
    pub location: bool,
    /// An ongoing call from a known call app (docs/PRD-calls.md).
    pub calls: bool,
    /// Each family in [`CALL_KEYS`] order.
    pub call_apps: [bool; 7],
}

impl Default for Settings {
    fn default() -> Self {
        Self { microphone: true, camera: true, screen: true, location: false, calls: true, call_apps: CALL_KEYS.map(|(_, _, on)| on) }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        crate::paths::config().join("privacy.json")
    }

    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    pub fn shows(&self, kind: Kind) -> bool {
        match kind {
            Kind::Microphone => self.microphone,
            Kind::Camera => self.camera,
            Kind::Screen => self.screen,
            Kind::Location => self.location,
        }
    }

    /// Whether a call app of `family` counts as a call.
    pub fn calls_from(&self, family: Family) -> bool {
        self.calls && CALL_KEYS.iter().zip(self.call_apps).any(|((f, _, _), on)| *f == family && on)
    }

    pub fn load_from(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(text.trim_start_matches('\u{feff}')).unwrap_or_else(|| {
                eprintln!("privacy: {} isn't valid JSON; using the defaults", path.display());
                Self::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let defaults = Self::default();
                let mut map = defaults.to_map();
                map.insert("enabled".into(), Value::Bool(true));
                write(path, &serde_json::to_string_pretty(&Value::Object(map)).unwrap_or_default());
                defaults
            }
            Err(e) => {
                eprintln!("privacy: can't read {}: {e}", path.display());
                Self::default()
            }
        }
    }

    /// `None` if the text isn't a JSON object.
    pub fn parse(text: &str) -> Option<Self> {
        let Value::Object(map) = serde_json::from_str(text).ok()? else { return None };
        let d = Self::default();
        let flag = |key: &str, default: bool| map.get(key).and_then(Value::as_bool).unwrap_or(default);
        let mut call_apps = d.call_apps;
        for (on, (_, key, _)) in call_apps.iter_mut().zip(CALL_KEYS) {
            *on = flag(key, *on);
        }
        Some(Self {
            microphone: flag("microphone", d.microphone),
            camera: flag("camera", d.camera),
            screen: flag("screen", d.screen),
            location: flag("location", d.location),
            calls: flag("calls", d.calls),
            call_apps,
        })
    }

    fn to_map(&self) -> Map<String, Value> {
        let Value::Object(mut map) = json!({ "microphone": self.microphone, "camera": self.camera, "screen": self.screen, "location": self.location, "calls": self.calls }) else { unreachable!() };
        for ((_, key, _), on) in CALL_KEYS.iter().zip(self.call_apps) {
            map.insert((*key).into(), Value::Bool(on));
        }
        map
    }

    /// Applies a tray command (`microphone:off`, `camera:on`, `screen:`, `location:`,
    /// `calls:`, `call_teams:` and the other call apps' keys).
    /// `false` if it isn't one.
    pub fn apply(&mut self, command: &str) -> bool {
        let Some((name, value)) = command.split_once(':') else { return false };
        let on = match value {
            "on" => true,
            "off" => false,
            _ => return false,
        };
        let field = match name {
            "microphone" => &mut self.microphone,
            "camera" => &mut self.camera,
            "screen" => &mut self.screen,
            "location" => &mut self.location,
            "calls" => &mut self.calls,
            _ => match CALL_KEYS.iter().position(|(_, key, _)| *key == name) {
                Some(i) => &mut self.call_apps[i],
                None => return false,
            },
        };
        *field = on;
        true
    }

    /// Saves over `path`, keeping keys this version doesn't know (and `enabled`). A file
    /// that isn't JSON is the user's to fix, so it's left as it is.
    pub fn save_to(&self, path: &Path) {
        let mut map = match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str(text.trim_start_matches('\u{feff}')) {
                Ok(Value::Object(map)) => map,
                _ => {
                    eprintln!("privacy: {} isn't valid JSON; not saving", path.display());
                    return;
                }
            },
            Err(_) => Map::new(),
        };
        map.extend(self.to_map());
        write(path, &serde_json::to_string_pretty(&Value::Object(map)).unwrap_or_default());
    }
}

fn write(path: &Path, text: &str) {
    if let Some(dir) = path.parent() {
        _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(path, text) {
        eprintln!("privacy: can't write {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("reui-privacy-settings-{name}-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&dir);
        dir.join("privacy.json")
    }

    #[test]
    fn privacy_settings_defaults_and_fallbacks() {
        let d = Settings::default();
        assert!(d.microphone && d.camera && d.screen && !d.location);
        assert_eq!(Settings::parse(r#"{ "camera": false, "location": "yes" }"#), Some(Settings { camera: false, ..d.clone() }));
        assert_eq!(Settings::parse("[1]"), None);
        assert!(!d.shows(Kind::Location) && d.shows(Kind::Screen));
        assert!(d.calls_from(Family::Teams) && !d.calls_from(Family::Browsers));
        let s = Settings::parse(r#"{ "call_browsers": true, "call_zoom": false }"#).unwrap();
        assert!(s.calls_from(Family::Browsers) && !s.calls_from(Family::Zoom));
        assert!(!Settings::parse(r#"{ "calls": false }"#).unwrap().calls_from(Family::Teams));
    }

    #[test]
    fn privacy_settings_tray_commands() {
        let mut s = Settings::default();
        assert!(s.apply("microphone:off") && !s.microphone);
        assert!(s.apply("location:on") && s.location);
        assert!(!s.apply("camera:maybe") && !s.apply("mute") && !s.apply("webcam:on"));
        assert!(s.apply("call_discord:off") && !s.calls_from(Family::Discord));
        assert!(s.apply("calls:off") && !s.calls && !s.apply("call_skype:off"));
    }

    #[test]
    fn privacy_settings_files_are_created_saved_and_never_clobbered() {
        let path = temp("files");
        assert_eq!(Settings::load_from(&path), Settings::default());
        let created: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!((created["enabled"].as_bool(), created["location"].as_bool()), (Some(true), Some(false)));

        std::fs::write(&path, r#"{ "future": 1, "enabled": false }"#).unwrap();
        Settings { screen: false, ..Settings::default() }.save_to(&path);
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!((saved["future"].as_u64(), saved["enabled"].as_bool(), saved["screen"].as_bool()), (Some(1), Some(false), Some(false)));

        std::fs::write(&path, "{ broken").unwrap();
        assert_eq!(Settings::load_from(&path), Settings::default());
        Settings::default().save_to(&path);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ broken");
        _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
