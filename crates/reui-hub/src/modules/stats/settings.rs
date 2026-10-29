//! System stats settings, in `%APPDATA%\ReUI\stats.json`, written by the Settings window too
//! (which then sends the module `reload`). Same rules as the other modules': created with
//! the defaults when missing, a missing or mistyped field falls back to its default, and a
//! file that isn't JSON is never overwritten.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use super::sampler::Wanted;

pub const DEFAULT_SHORTCUT: &str = "Win+Alt+S";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// Opens the card ("Win+Alt+S").
    pub shortcut: String,
    /// The rows the card shows, and so what the sampler reads. The GPU costs the most.
    pub show: Wanted,
}

impl Default for Settings {
    fn default() -> Self {
        Self { shortcut: DEFAULT_SHORTCUT.into(), show: Wanted { gpu: true, disk: true, network: true, power: true } }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        crate::paths::config().join("stats.json")
    }

    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).unwrap_or_else(|| {
                eprintln!("stats: {} isn't valid JSON; using the defaults", path.display());
                Self::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let defaults = Self::default();
                write(path, &serde_json::to_string_pretty(&defaults.to_map()).unwrap_or_default());
                defaults
            }
            Err(e) => {
                eprintln!("stats: can't read {}: {e}", path.display());
                Self::default()
            }
        }
    }

    /// `None` if the text isn't a JSON object.
    pub fn parse(text: &str) -> Option<Self> {
        let Value::Object(map) = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()? else { return None };
        let d = Self::default();
        let flag = |key: &str, default: bool| map.get(key).and_then(Value::as_bool).unwrap_or(default);
        Some(Self {
            shortcut: map.get("shortcut").and_then(Value::as_str).map_or(d.shortcut, str::to_string),
            show: Wanted { gpu: flag("gpu", d.show.gpu), disk: flag("disk", d.show.disk), network: flag("network", d.show.network), power: flag("power", d.show.power) },
        })
    }

    fn to_map(&self) -> Map<String, Value> {
        let Value::Object(map) = json!({
            "enabled": true,
            "shortcut": self.shortcut,
            "gpu": self.show.gpu,
            "disk": self.show.disk,
            "network": self.show.network,
            "power": self.show.power,
        }) else {
            unreachable!()
        };
        map
    }
}

fn write(path: &Path, text: &str) {
    if let Some(dir) = path.parent() {
        _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(path, text) {
        eprintln!("stats: can't write {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_settings_defaults_round_trip() {
        let d = Settings::default();
        let text = serde_json::to_string(&d.to_map()).unwrap();
        assert_eq!(Settings::parse(&text), Some(d));
    }

    #[test]
    fn stats_settings_bad_fields_fall_back() {
        let s = Settings::parse(r#"{ "gpu": false, "disk": "no", "shortcut": 3 }"#).unwrap();
        assert_eq!((s.show.gpu, s.show.disk, s.show.network, s.shortcut.as_str()), (false, true, true, DEFAULT_SHORTCUT));
        assert_eq!(Settings::parse("[1]"), None);
    }
}
