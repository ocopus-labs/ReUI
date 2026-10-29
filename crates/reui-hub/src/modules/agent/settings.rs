//! Coding agent settings, in `%APPDATA%\ReUI\coding.json`, shared by the Claude Code,
//! Codex and OpenCode modules and written by the Settings window too (which then sends
//! each of them `reload`). Same rules as the other modules' files: created with the
//! defaults when missing, a missing or mistyped field falls back to its default, keys this
//! version doesn't know are kept, and a file that isn't JSON is never overwritten.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

/// When a session is in the island at all (permissions always are).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShowSessions {
    /// Only when it needs you: a permission, a question, a failed turn.
    Needed,
    /// Also while it works (thinking, running a tool, compacting): the old behaviour.
    Working,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub show_sessions: ShowSessions,
    /// A finished turn ("Done in 2m") shows briefly.
    pub show_finished: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { show_sessions: ShowSessions::Needed, show_finished: false }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        crate::paths::config().join("coding.json")
    }

    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).unwrap_or_else(|| {
                eprintln!("agents: {} isn't valid JSON; using the defaults", path.display());
                Self::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let defaults = Self::default();
                write(path, &serde_json::to_string_pretty(&defaults.to_map()).unwrap_or_default());
                defaults
            }
            Err(e) => {
                eprintln!("agents: can't read {}: {e}", path.display());
                Self::default()
            }
        }
    }

    /// `None` if the text isn't a JSON object.
    pub fn parse(text: &str) -> Option<Self> {
        let Value::Object(map) = serde_json::from_str(text).ok()? else { return None };
        let d = Self::default();
        let show_sessions = match map.get("show_sessions").and_then(Value::as_str).map(|s| s.trim().to_ascii_lowercase()).as_deref() {
            Some("working") => ShowSessions::Working,
            Some("needed") => ShowSessions::Needed,
            _ => d.show_sessions,
        };
        Some(Self { show_sessions, show_finished: map.get("show_finished").and_then(Value::as_bool).unwrap_or(d.show_finished) })
    }

    fn to_map(&self) -> Map<String, Value> {
        let sessions = match self.show_sessions {
            ShowSessions::Needed => "needed",
            ShowSessions::Working => "working",
        };
        let Value::Object(map) = json!({ "show_sessions": sessions, "show_finished": self.show_finished }) else { unreachable!() };
        map
    }
}

fn write(path: &Path, text: &str) {
    if let Some(dir) = path.parent() {
        _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(path, text) {
        eprintln!("agents: can't write {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coding_defaults_are_quiet_and_round_trip() {
        let d = Settings::default();
        assert_eq!((d.show_sessions, d.show_finished), (ShowSessions::Needed, false));
        assert_eq!(Settings::parse(&serde_json::to_string(&d.to_map()).unwrap()), Some(d));
    }

    #[test]
    fn coding_fields_fall_back_one_by_one() {
        let s = Settings::parse(r#"{ "show_sessions": " Working ", "show_finished": "yes" }"#).unwrap();
        assert_eq!((s.show_sessions, s.show_finished), (ShowSessions::Working, false));
        assert_eq!(Settings::parse(r#"{ "show_sessions": "always", "show_finished": true }"#).unwrap(), Settings { show_finished: true, ..Settings::default() });
        assert_eq!(Settings::parse("[]"), None);
    }

    #[test]
    fn coding_file_is_created_and_never_clobbered() {
        let dir = std::env::temp_dir().join(format!("reui-coding-settings-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("coding.json");
        assert_eq!(Settings::load_from(&path), Settings::default());
        assert_eq!(Settings::parse(&std::fs::read_to_string(&path).unwrap()), Some(Settings::default()));
        std::fs::write(&path, "{ broken").unwrap();
        assert_eq!(Settings::load_from(&path), Settings::default());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ broken");
        _ = std::fs::remove_dir_all(&dir);
    }
}
