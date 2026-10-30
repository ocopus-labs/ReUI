//! Script activity settings, in `%APPDATA%\ReUI\activities.json`, written by the Settings
//! window too (which then sends the module `reload`). Same rules as the other modules':
//! created with the defaults when missing, a missing or mistyped field falls back to its
//! default, and a file that isn't JSON is never overwritten.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// Off: commands are ignored and nothing shows. Read on every reload, so switching it
    /// needs no restart.
    pub enabled: bool,
    /// No update for this many minutes ends a running activity (unless it's kept).
    pub stale_min: u32,
    /// At most this many at once from one source.
    pub per_source: u32,
    /// Sources never shown (`pwsh`, `python`, a script's own `--source`).
    pub blocked: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self { enabled: true, stale_min: 10, per_source: 3, blocked: Vec::new() }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        crate::paths::config().join("activities.json")
    }

    pub fn load() -> Self {
        let path = Self::path();
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::parse(text.trim_start_matches('\u{feff}')).unwrap_or_else(|| {
                eprintln!("activities: {} isn't valid JSON; using the defaults", path.display());
                Self::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let d = Self::default();
                write(&path, &json!({ "enabled": d.enabled, "stale_min": d.stale_min, "per_source": d.per_source, "blocked": d.blocked }));
                d
            }
            Err(e) => {
                eprintln!("activities: can't read {}: {e}", path.display());
                Self::default()
            }
        }
    }

    /// `None` if the text isn't a JSON object.
    pub fn parse(text: &str) -> Option<Self> {
        let Value::Object(map) = serde_json::from_str(text).ok()? else { return None };
        let d = Self::default();
        let number = |key: &str, range: std::ops::RangeInclusive<u64>, default: u32| map.get(key).and_then(Value::as_u64).filter(|n| range.contains(n)).map_or(default, |n| n as u32);
        let blocked = map.get("blocked").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect()).unwrap_or_default();
        Some(Self {
            enabled: map.get("enabled").and_then(Value::as_bool).unwrap_or(d.enabled),
            stale_min: number("stale_min", 1..=24 * 60, d.stale_min),
            per_source: number("per_source", 1..=8, d.per_source),
            blocked,
        })
    }
}

fn write(path: &Path, value: &Value) {
    if let Some(dir) = path.parent() {
        _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(path, serde_json::to_string_pretty(value).unwrap_or_default()) {
        eprintln!("activities: can't write {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_settings_fall_back_field_by_field() {
        assert_eq!(Settings::parse("{}"), Some(Settings::default()));
        let s = Settings::parse(r#"{ "enabled": false, "stale_min": 0, "per_source": 5, "blocked": ["Python", 3, " "] }"#).unwrap();
        assert_eq!(s, Settings { enabled: false, stale_min: 10, per_source: 5, blocked: vec!["python".into()] });
        assert_eq!(Settings::parse("[]"), None);
    }
}
