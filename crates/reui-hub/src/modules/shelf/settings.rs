//! File shelf settings, in `%APPDATA%\ReUI\shelf.json`, written by the Settings window too
//! (which then sends the module `reload`). Same rules as the other modules': created with
//! the defaults when missing, a missing or mistyped field falls back to its default, and a
//! file that isn't JSON is never overwritten.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

/// How long held things stay (PRD §5.3): the user decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Keep {
    /// Until the user removes them, across restarts and reboots.
    #[default]
    Removed,
    /// Until ReUI next starts.
    Restart,
    Day,
    Week,
}

impl Keep {
    fn parse(text: &str) -> Option<Keep> {
        Some(match text {
            "removed" => Keep::Removed,
            "restart" => Keep::Restart,
            "day" => Keep::Day,
            "week" => Keep::Week,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Keep::Removed => "removed",
            Keep::Restart => "restart",
            Keep::Day => "day",
            Keep::Week => "week",
        }
    }

    /// How long a held thing lasts, when it's a length of time.
    pub fn max_age_ms(self) -> Option<i64> {
        match self {
            Keep::Day => Some(24 * 3_600_000),
            Keep::Week => Some(7 * 24 * 3_600_000),
            Keep::Removed | Keep::Restart => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub keep: Keep,
    /// The most ReUI's own copies (saved images, files from temporary folders) may take.
    pub saved_cap_mb: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self { keep: Keep::Removed, saved_cap_mb: 512 }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        crate::paths::config().join("shelf.json")
    }

    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).unwrap_or_else(|| {
                eprintln!("shelf: {} isn't valid JSON; using the defaults", path.display());
                Self::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let defaults = Self::default();
                write(path, &serde_json::to_string_pretty(&defaults.to_map()).unwrap_or_default());
                defaults
            }
            Err(e) => {
                eprintln!("shelf: can't read {}: {e}", path.display());
                Self::default()
            }
        }
    }

    /// `None` if the text isn't a JSON object.
    pub fn parse(text: &str) -> Option<Self> {
        let Value::Object(map) = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()? else { return None };
        let d = Self::default();
        Some(Self {
            keep: map.get("keep").and_then(Value::as_str).and_then(Keep::parse).unwrap_or(d.keep),
            saved_cap_mb: map.get("saved_cap_mb").and_then(Value::as_u64).filter(|mb| *mb > 0).unwrap_or(d.saved_cap_mb),
        })
    }

    pub fn saved_cap_bytes(&self) -> u64 {
        self.saved_cap_mb.saturating_mul(1024 * 1024)
    }

    fn to_map(&self) -> Map<String, Value> {
        let Value::Object(map) = json!({ "enabled": true, "keep": self.keep.name(), "saved_cap_mb": self.saved_cap_mb }) else { unreachable!() };
        map
    }
}

fn write(path: &Path, text: &str) {
    if let Some(dir) = path.parent() {
        _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(path, text) {
        eprintln!("shelf: can't write {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shelf_settings_defaults_round_trip() {
        let d = Settings::default();
        let text = serde_json::to_string(&d.to_map()).unwrap();
        assert_eq!(Settings::parse(&text), Some(d));
    }

    #[test]
    fn shelf_settings_bad_fields_fall_back() {
        let s = Settings::parse(r#"{ "keep": "forever", "saved_cap_mb": 0 }"#).unwrap();
        assert_eq!(s, Settings::default());
        let s = Settings::parse(r#"{ "keep": "week", "saved_cap_mb": 64 }"#).unwrap();
        assert_eq!((s.keep, s.saved_cap_bytes()), (Keep::Week, 64 * 1024 * 1024));
        assert_eq!(Settings::parse("[1]"), None);
        assert_eq!(Keep::Day.max_age_ms(), Some(86_400_000));
        assert_eq!(Keep::Restart.max_age_ms(), None);
    }
}
