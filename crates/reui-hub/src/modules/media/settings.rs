//! Now Playing settings, in `%APPDATA%\ReUI\media.json`, written by the Settings window
//! too (which then sends the module `reload`).
//! Created with the defaults when missing and read again whenever the card opens, so edits
//! apply without a restart. A missing or mistyped field falls back to its default; a file
//! that isn't JSON is left alone (never overwritten, not even by the tray) and ignored.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Visualizer {
    /// Bars driven by the player's own audio (the island captures it).
    #[default]
    Real,
    /// Synthetic bars.
    Simple,
    /// Bars at rest, always.
    Off,
}

impl Visualizer {
    fn as_str(self) -> &'static str {
        match self {
            Self::Real => "real",
            Self::Simple => "simple",
            Self::Off => "off",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "real" => Some(Self::Real),
            "simple" => Some(Self::Simple),
            "off" => Some(Self::Off),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub visualizer: Visualizer,
    /// Synced lyrics from lrclib.net; off by default because every title goes there.
    pub lyrics: bool,
    /// A player paused this long steps away; 0 keeps it forever.
    pub retract_after_pause_min: u32,
    pub swipe_to_skip: bool,
    /// Video sessions (a film in Movies & TV) are shown at all.
    pub show_video: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { visualizer: Visualizer::Real, lyrics: false, retract_after_pause_min: 10, swipe_to_skip: true, show_video: true }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("ReUI")).unwrap_or_else(|| PathBuf::from(".")).join("media.json")
    }

    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).unwrap_or_else(|| {
                eprintln!("media: {} isn't valid JSON; using the defaults", path.display());
                Self::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let defaults = Self::default();
                write(path, &defaults.to_json());
                defaults
            }
            Err(e) => {
                eprintln!("media: can't read {}: {e}", path.display());
                Self::default()
            }
        }
    }

    /// `None` if the text isn't a JSON object.
    pub fn parse(text: &str) -> Option<Self> {
        let Value::Object(map) = serde_json::from_str(text).ok()? else { return None };
        let d = Self::default();
        let flag = |key: &str, default: bool| map.get(key).and_then(Value::as_bool).unwrap_or(default);
        Some(Self {
            visualizer: map.get("visualizer").and_then(Value::as_str).and_then(Visualizer::parse).unwrap_or(d.visualizer),
            lyrics: flag("lyrics", d.lyrics),
            retract_after_pause_min: map.get("retract_after_pause_min").and_then(Value::as_u64).map_or(d.retract_after_pause_min, |n| n.min(u64::from(u32::MAX)) as u32),
            swipe_to_skip: flag("swipe_to_skip", d.swipe_to_skip),
            show_video: flag("show_video", d.show_video),
        })
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(&self.to_map()).unwrap_or_default()
    }

    fn to_map(&self) -> Map<String, Value> {
        let Value::Object(map) = json!({
            "visualizer": self.visualizer.as_str(),
            "lyrics": self.lyrics,
            "retract_after_pause_min": self.retract_after_pause_min,
            "swipe_to_skip": self.swipe_to_skip,
            "show_video": self.show_video,
        }) else {
            unreachable!()
        };
        map
    }

    /// Applies a tray command (`lyrics:on`, `visualizer:off`, ...). `false` if it isn't one.
    pub fn apply(&mut self, command: &str) -> bool {
        match command.split_once(':') {
            Some(("lyrics", "on")) => self.lyrics = true,
            Some(("lyrics", "off")) => self.lyrics = false,
            Some(("visualizer", mode)) => match Visualizer::parse(mode) {
                Some(v) => self.visualizer = v,
                None => return false,
            },
            _ => return false,
        }
        true
    }

    /// Saves over `path`, keeping keys this version doesn't know. A file that isn't JSON is
    /// the user's to fix, so it's left as it is (the change then lasts until the hub restarts).
    pub fn save_to(&self, path: &Path) {
        let mut map = match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(Value::Object(map)) => map,
                _ => {
                    eprintln!("media: {} isn't valid JSON; not saving", path.display());
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
        eprintln!("media: can't write {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("reui-media-settings-{name}-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&dir);
        dir.join("media.json")
    }

    #[test]
    fn defaults_round_trip() {
        let d = Settings::default();
        assert_eq!((d.visualizer, d.lyrics, d.retract_after_pause_min, d.swipe_to_skip, d.show_video), (Visualizer::Real, false, 10, true, true));
        assert_eq!(Settings::parse(&d.to_json()), Some(d));
    }

    #[test]
    fn partial_and_wrong_fields_fall_back() {
        let s = Settings::parse(r#"{ "visualizer": "Simple", "lyrics": "yes", "retract_after_pause_min": 0, "show_video": false }"#).unwrap();
        assert_eq!(s.visualizer, Visualizer::Simple);
        assert!(!s.lyrics && s.swipe_to_skip && !s.show_video);
        assert_eq!(s.retract_after_pause_min, 0);
        assert_eq!(Settings::parse(r#"{ "visualizer": "loud" }"#).unwrap().visualizer, Visualizer::Real);
        assert_eq!(Settings::parse("not json"), None);
        assert_eq!(Settings::parse("{}"), Some(Settings::default()));
    }

    #[test]
    fn tray_commands() {
        let mut s = Settings::default();
        assert!(s.apply("lyrics:on") && s.lyrics);
        assert!(s.apply("visualizer:off") && s.visualizer == Visualizer::Off);
        assert!(!s.apply("visualizer:loud") && s.visualizer == Visualizer::Off);
        assert!(!s.apply("play"));
    }

    #[test]
    fn files_are_created_saved_and_never_clobbered() {
        let path = temp("files");
        assert_eq!(Settings::load_from(&path), Settings::default());
        assert_eq!(Settings::parse(&std::fs::read_to_string(&path).unwrap()), Some(Settings::default()));

        // Saving keeps keys it doesn't know.
        std::fs::write(&path, r#"{ "future": 1, "lyrics": false }"#).unwrap();
        let s = Settings { lyrics: true, ..Settings::default() };
        s.save_to(&path);
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!((saved["future"].as_u64(), saved["lyrics"].as_bool()), (Some(1), Some(true)));

        // A broken file is neither replaced on load nor on save.
        std::fs::write(&path, "{ broken").unwrap();
        assert_eq!(Settings::load_from(&path), Settings::default());
        s.save_to(&path);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ broken");
        _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
