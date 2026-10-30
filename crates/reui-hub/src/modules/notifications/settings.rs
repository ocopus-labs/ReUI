//! Notification settings, in `%APPDATA%\ReUI\notifications.json`. Same rules as the other
//! modules' files (a missing or mistyped field falls back to its default, keys this
//! version doesn't know are kept, a file that isn't JSON is never overwritten), with one
//! difference: `enabled` defaults to **off**, and the file isn't created until the module
//! runs (`--notifications`, or `"enabled": true`), because reading other apps'
//! notifications is something a user turns on, never something ReUI starts doing.
//!
//! ```json
//! { "enabled": true, "apps": ["Microsoft.WindowsTerminal_8wekyb3d8bbwe!App"],
//!   "hide_content": false, "seen": [{ "app": "…AUMID…", "name": "Terminal" }] }
//! ```
//!
//! `apps` is the allow-list (AUMIDs): only their toasts show, and only theirs are read past
//! the app's name. `seen` lists apps that notified lately (AUMID and display name, never a
//! toast's text), newest first, so a picker can offer them.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

/// How many recent apps `seen` keeps.
const SEEN_MAX: usize = 24;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeenApp {
    pub app: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub apps: Vec<String>,
    /// Show the app alone ("Teams"), never the title or text, and don't read them.
    pub hide_content: bool,
    pub seen: Vec<SeenApp>,
}

/// Whether the module should run: `"enabled": true` in the file. Off when it's missing.
pub fn enabled() -> bool {
    let text = std::fs::read_to_string(Settings::path()).unwrap_or_default();
    serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')).ok().and_then(|v| v.get("enabled").and_then(Value::as_bool)).unwrap_or(false)
}

impl Settings {
    pub fn path() -> PathBuf {
        crate::paths::config().join("notifications.json")
    }

    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    pub fn allows(&self, app: &str) -> bool {
        self.apps.iter().any(|a| a.eq_ignore_ascii_case(app))
    }

    pub fn load_from(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(text.trim_start_matches('\u{feff}')).unwrap_or_else(|| {
                eprintln!("notifications: {} isn't valid JSON; using the defaults", path.display());
                Self::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Running without a file means `--notifications`: write the defaults so
                // the allow-list has somewhere to go, still off for the next start.
                let mut map = Self::default().to_map();
                map.insert("enabled".into(), Value::Bool(false));
                write(path, &serde_json::to_string_pretty(&Value::Object(map)).unwrap_or_default());
                Self::default()
            }
            Err(e) => {
                eprintln!("notifications: can't read {}: {e}", path.display());
                Self::default()
            }
        }
    }

    /// `None` if the text isn't a JSON object.
    pub fn parse(text: &str) -> Option<Self> {
        let Value::Object(map) = serde_json::from_str(text).ok()? else { return None };
        let strings = |v: Option<&Value>| -> Vec<String> { v.and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default() };
        let seen = map
            .get("seen")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|s| Some(SeenApp { app: s.get("app")?.as_str()?.to_owned(), name: s.get("name").and_then(Value::as_str).unwrap_or_default().to_owned() })).collect())
            .unwrap_or_default();
        Some(Self { apps: strings(map.get("apps")), hide_content: map.get("hide_content").and_then(Value::as_bool).unwrap_or(false), seen })
    }

    fn to_map(&self) -> Map<String, Value> {
        let seen: Vec<Value> = self.seen.iter().map(|s| json!({ "app": s.app, "name": s.name })).collect();
        let Value::Object(map) = json!({ "apps": self.apps, "hide_content": self.hide_content, "seen": seen }) else { unreachable!() };
        map
    }

    /// An app notified: to the front of `seen`. `true` if `seen` changed (worth saving).
    pub fn saw(&mut self, app: &str, name: &str) -> bool {
        if self.seen.first().is_some_and(|s| s.app == app && s.name == name) {
            return false;
        }
        self.seen.retain(|s| s.app != app);
        self.seen.insert(0, SeenApp { app: app.to_owned(), name: name.to_owned() });
        self.seen.truncate(SEEN_MAX);
        true
    }

    /// Saves over `path`, keeping keys this version doesn't know (and `enabled`). A file
    /// that isn't JSON is the user's to fix, so it's left as it is.
    pub fn save_to(&self, path: &Path) {
        let mut map = match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str(text.trim_start_matches('\u{feff}')) {
                Ok(Value::Object(map)) => map,
                _ => {
                    eprintln!("notifications: {} isn't valid JSON; not saving", path.display());
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
        eprintln!("notifications: can't write {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("reui-notifications-settings-{name}-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&dir);
        dir.join("notifications.json")
    }

    #[test]
    fn notifications_settings_parse_with_fallbacks() {
        let s = Settings::parse(r#"{ "apps": ["A!App", 3, "b"], "hide_content": "yes", "seen": [{ "app": "x" }, { "name": "no app" }] }"#).unwrap();
        assert_eq!(s.apps, vec!["A!App".to_owned(), "b".to_owned()]);
        assert!(!s.hide_content);
        assert_eq!(s.seen, vec![SeenApp { app: "x".into(), name: String::new() }]);
        assert!(s.allows("a!app") && !s.allows("c"));
        assert_eq!(Settings::parse("[]"), None);
    }

    #[test]
    fn notifications_seen_keeps_the_newest_first() {
        let mut s = Settings::default();
        assert!(s.saw("a", "A") && s.saw("b", "B"));
        assert!(!s.saw("b", "B"), "already in front");
        assert!(s.saw("a", "A"));
        assert_eq!(s.seen.iter().map(|s| s.app.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        for i in 0..40 {
            s.saw(&i.to_string(), "");
        }
        assert_eq!(s.seen.len(), SEEN_MAX);
    }

    #[test]
    fn notifications_settings_files_stay_off_and_are_never_clobbered() {
        let path = temp("files");
        assert_eq!(Settings::load_from(&path), Settings::default());
        let created: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(created["enabled"].as_bool(), Some(false), "created off");

        std::fs::write(&path, r#"{ "future": 1, "enabled": true, "apps": ["a"] }"#).unwrap();
        let mut s = Settings::load_from(&path);
        s.saw("a", "A");
        s.save_to(&path);
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!((saved["future"].as_u64(), saved["enabled"].as_bool(), saved["seen"][0]["name"].as_str()), (Some(1), Some(true), Some("A")));

        std::fs::write(&path, "{ broken").unwrap();
        Settings::default().save_to(&path);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ broken");
        _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
