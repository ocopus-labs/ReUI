//! Its one setting, in `%APPDATA%\ReUI\update-restart.json`: `enabled` (Settings →
//! General, or right-click → off). Same rules as the other modules' files: created with
//! the default when missing, a mistyped field falls back to it, and a file that isn't JSON
//! is never overwritten.

use serde_json::{json, Value};

pub const FILE: &str = "update-restart.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub enabled: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { enabled: true }
    }
}

impl Settings {
    pub fn load() -> Self {
        let path = crate::paths::config().join(FILE);
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::parse(text.trim_start_matches('\u{feff}')).unwrap_or_else(|| {
                eprintln!("update-restart: {} isn't valid JSON; using the defaults", path.display());
                Self::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let d = Self::default();
                if let Some(dir) = path.parent() {
                    _ = std::fs::create_dir_all(dir);
                }
                _ = std::fs::write(&path, serde_json::to_string_pretty(&json!({ "enabled": d.enabled })).unwrap_or_default());
                d
            }
            Err(e) => {
                eprintln!("update-restart: can't read {}: {e}", path.display());
                Self::default()
            }
        }
    }

    /// `None` if the text isn't a JSON object.
    pub fn parse(text: &str) -> Option<Self> {
        let Value::Object(map) = serde_json::from_str(text).ok()? else { return None };
        Some(Self { enabled: map.get("enabled").and_then(Value::as_bool).unwrap_or(Self::default().enabled) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_unless_the_file_says_off() {
        assert_eq!(Settings::parse("{}"), Some(Settings { enabled: true }));
        assert_eq!(Settings::parse(r#"{ "enabled": false }"#), Some(Settings { enabled: false }));
        assert_eq!(Settings::parse(r#"{ "enabled": "no" }"#), Some(Settings { enabled: true }));
        assert_eq!(Settings::parse("not json"), None);
    }
}
