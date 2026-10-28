//! Timers settings, in `%APPDATA%\ReUI\timers.json`, written by the Settings window too
//! (which then sends the module `reload`). Same rules as the other modules': created with
//! the defaults when missing, a missing or mistyped field falls back to its default, keys
//! this version doesn't know are kept, and a file that isn't JSON is never overwritten.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use super::sound;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// Opens the launcher ("Win+Alt+T").
    pub shortcut: String,
    /// Preset lengths in minutes, in order.
    pub presets: Vec<u32>,
    /// A `C:\Windows\Media` file without `.wav`, or `None` for silence.
    pub sound: Option<String>,
    /// How long the finish sound rings; 0 rings until stopped.
    pub ring_for_s: u32,
    pub focus: Focus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Focus {
    pub work_min: u32,
    pub short_break_min: u32,
    pub long_break_min: u32,
    /// Work phases before a long break.
    pub rounds: u32,
    /// Start the next phase without asking.
    pub auto_start: bool,
    /// Turn on Do Not Disturb while working (T1).
    pub dnd: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            shortcut: "Win+Alt+T".into(),
            presets: vec![1, 5, 10, 25],
            sound: Some(sound::DEFAULT.into()),
            ring_for_s: 60,
            focus: Focus { work_min: 25, short_break_min: 5, long_break_min: 15, rounds: 4, auto_start: false, dnd: false },
        }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        crate::paths::config().join("timers.json")
    }

    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).unwrap_or_else(|| {
                eprintln!("timers: {} isn't valid JSON; using the defaults", path.display());
                Self::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let defaults = Self::default();
                write(path, &serde_json::to_string_pretty(&defaults.to_map()).unwrap_or_default());
                defaults
            }
            Err(e) => {
                eprintln!("timers: can't read {}: {e}", path.display());
                Self::default()
            }
        }
    }

    /// `None` if the text isn't a JSON object.
    pub fn parse(text: &str) -> Option<Self> {
        let Value::Object(map) = serde_json::from_str(text).ok()? else { return None };
        let d = Self::default();
        let flag = |key: &str, default: bool| map.get(key).and_then(Value::as_bool).unwrap_or(default);
        // Minutes from 1 to a day; anything else is the default.
        let minutes = |key: &str, default: u32| map.get(key).and_then(Value::as_u64).filter(|m| (1..=24 * 60).contains(m)).map_or(default, |m| m as u32);
        let presets: Vec<u32> = map
            .get("presets")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_u64).filter(|m| (1..=24 * 60).contains(m)).map(|m| m as u32).take(6).collect())
            .filter(|p: &Vec<u32>| !p.is_empty())
            .unwrap_or(d.presets);
        let sound = match map.get("sound").and_then(Value::as_str) {
            Some("none") => None,
            Some(name) if is_media_name(name) => Some(name.to_string()),
            _ => d.sound,
        };
        Some(Self {
            shortcut: map.get("shortcut").and_then(Value::as_str).map_or(d.shortcut, str::to_string),
            presets,
            sound,
            ring_for_s: map.get("ring_for_s").and_then(Value::as_u64).filter(|s| *s <= 600).map_or(d.ring_for_s, |s| s as u32),
            focus: Focus {
                work_min: minutes("focus_work_min", d.focus.work_min),
                short_break_min: minutes("focus_short_break_min", d.focus.short_break_min),
                long_break_min: minutes("focus_long_break_min", d.focus.long_break_min),
                rounds: map.get("focus_rounds").and_then(Value::as_u64).filter(|r| (1..=12).contains(r)).map_or(d.focus.rounds, |r| r as u32),
                auto_start: flag("focus_auto_start", d.focus.auto_start),
                dnd: flag("focus_dnd", d.focus.dnd),
            },
        })
    }

    fn to_map(&self) -> Map<String, Value> {
        let Value::Object(map) = json!({
            "enabled": true,
            "shortcut": self.shortcut,
            "presets": self.presets,
            "sound": self.sound.as_deref().unwrap_or("none"),
            "ring_for_s": self.ring_for_s,
            "focus_work_min": self.focus.work_min,
            "focus_short_break_min": self.focus.short_break_min,
            "focus_long_break_min": self.focus.long_break_min,
            "focus_rounds": self.focus.rounds,
            "focus_auto_start": self.focus.auto_start,
            "focus_dnd": self.focus.dnd,
        }) else {
            unreachable!()
        };
        map
    }
}

/// A plain file name (no path tricks): only letters and digits.
fn is_media_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 32 && name.chars().all(|c| c.is_ascii_alphanumeric())
}

fn write(path: &Path, text: &str) {
    if let Some(dir) = path.parent() {
        _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(path, text) {
        eprintln!("timers: can't write {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timer_settings_defaults_round_trip() {
        let d = Settings::default();
        let text = serde_json::to_string(&d.to_map()).unwrap();
        assert_eq!(Settings::parse(&text), Some(d));
    }

    #[test]
    fn timer_settings_bad_fields_fall_back() {
        let s = Settings::parse(r#"{ "sound": "..\\evil", "presets": [0, 3, "x", 2000], "ring_for_s": 0, "focus_work_min": 50, "focus_rounds": 99, "focus_dnd": true }"#).unwrap();
        assert_eq!(s.sound.as_deref(), Some(sound::DEFAULT));
        assert_eq!(s.presets, vec![3]);
        assert_eq!(s.ring_for_s, 0);
        assert_eq!((s.focus.work_min, s.focus.rounds, s.focus.dnd), (50, 4, true));
        assert_eq!(Settings::parse(r#"{ "sound": "none" }"#).unwrap().sound, None);
        assert_eq!(Settings::parse("[1]"), None);
    }
}
