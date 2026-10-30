//! Clipboard settings, in `%APPDATA%\ReUI\clipboard.json`, written by the Settings window
//! too (which then sends the module `reload`).
//! Created with the defaults when missing; read at start and again on every hotkey open,
//! so edits apply without a restart. A field that's missing or the wrong type falls back
//! to its default; a file that isn't JSON is left alone (never overwritten) and ignored.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// Password managers mark most copies private already; these catch the ones that don't.
const PASSWORD_MANAGERS: [&str; 8] = ["keepass.exe", "keepassxc.exe", "bitwarden.exe", "1password.exe", "dashlane.exe", "keeper.exe", "enpass.exe", "lastpass.exe"];

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub hotkey: String,
    /// Unpinned items older than this are removed; 0 keeps them forever.
    pub retention_days: u32,
    /// Executable file names (lower case) whose copies are never recorded.
    pub ignored_apps: Vec<String>,
    pub link_previews: bool,
    /// The window's card layout: "strip" or "grid" (remembered from the window).
    pub layout: String,
    /// The copy confirmation in the island (a moment's glimpse of what was copied).
    pub copy_toast: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { hotkey: "Win+Alt+V".into(), retention_days: 30, ignored_apps: PASSWORD_MANAGERS.map(String::from).to_vec(), link_previews: true, layout: "strip".into(), copy_toast: true }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        crate::paths::config().join("clipboard.json")
    }

    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).unwrap_or_else(|| {
                eprintln!("clipboard: {} isn't valid JSON; using the defaults", path.display());
                Self::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let defaults = Self::default();
                if let Some(dir) = path.parent() {
                    _ = std::fs::create_dir_all(dir);
                }
                if let Err(e) = std::fs::write(path, defaults.to_json()) {
                    eprintln!("clipboard: can't write {}: {e}", path.display());
                }
                defaults
            }
            Err(e) => {
                eprintln!("clipboard: can't read {}: {e}", path.display());
                Self::default()
            }
        }
    }

    /// `None` if the text isn't a JSON object.
    pub fn parse(text: &str) -> Option<Self> {
        let Value::Object(map) = serde_json::from_str(text).ok()? else { return None };
        let d = Self::default();
        Some(Self {
            hotkey: map.get("hotkey").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map_or(d.hotkey, String::from),
            retention_days: map.get("retention_days").and_then(Value::as_u64).map_or(d.retention_days, |n| n.min(u64::from(u32::MAX)) as u32),
            ignored_apps: map
                .get("ignored_apps")
                .and_then(Value::as_array)
                .map_or(d.ignored_apps, |apps| apps.iter().filter_map(Value::as_str).map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect()),
            link_previews: map.get("link_previews").and_then(Value::as_bool).unwrap_or(d.link_previews),
            layout: map.get("layout").and_then(Value::as_str).and_then(layout).map_or(d.layout, String::from),
            copy_toast: map.get("copy_toast").and_then(Value::as_bool).unwrap_or(d.copy_toast),
        })
    }

    pub fn to_json(&self) -> String {
        let value = json!({
            "hotkey": self.hotkey,
            "retention_days": self.retention_days,
            "ignored_apps": self.ignored_apps,
            "link_previews": self.link_previews,
            "layout": self.layout,
            "copy_toast": self.copy_toast,
        });
        serde_json::to_string_pretty(&value).unwrap_or_default()
    }

    /// Remembers the window's layout, keeping every other key in the file as it is. A file
    /// that isn't a JSON object is never overwritten.
    pub fn save_layout(path: &Path, value: &str) {
        let Some(value) = layout(value) else { return };
        let mut map = match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(Value::Object(map)) => map,
                _ => return,
            },
            Err(_) => match serde_json::from_str(&Self::default().to_json()) {
                Ok(Value::Object(map)) => map,
                _ => return,
            },
        };
        map.insert("layout".into(), Value::String(value.into()));
        if let Err(e) = std::fs::write(path, serde_json::to_string_pretty(&Value::Object(map)).unwrap_or_default()) {
            eprintln!("clipboard: can't write {}: {e}", path.display());
        }
    }

    /// `exe`: the copying app's file name ("KeePass.exe" matches "keepass.exe").
    pub fn ignores(&self, exe: &str) -> bool {
        let exe = exe.rsplit(['\\', '/']).next().unwrap_or(exe);
        self.ignored_apps.iter().any(|app| app.eq_ignore_ascii_case(exe))
    }
}

/// A known layout name.
fn layout(name: &str) -> Option<&'static str> {
    ["strip", "grid"].into_iter().find(|l| *l == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip() {
        let d = Settings::default();
        assert_eq!(d.hotkey, "Win+Alt+V");
        assert_eq!(d.retention_days, 30);
        assert!(d.link_previews);
        assert!(d.ignores("KeePassXC.exe") && d.ignores(r"C:\Program Files\Bitwarden\bitwarden.exe"));
        assert!(!d.ignores("chrome.exe"));
        assert!(d.copy_toast && !Settings::parse(r#"{ "copy_toast": false }"#).unwrap().copy_toast);
        assert_eq!(Settings::parse(&d.to_json()), Some(d));
    }

    #[test]
    fn partial_and_wrong_fields_fall_back() {
        let s = Settings::parse(r#"{ "retention_days": 0, "ignored_apps": ["Foo.EXE", 3, ""], "link_previews": "no" }"#).unwrap();
        assert_eq!(s.retention_days, 0);
        assert_eq!(s.ignored_apps, ["foo.exe"]);
        assert!(s.link_previews);
        assert_eq!(s.hotkey, "Win+Alt+V");
        assert_eq!(Settings::parse("not json"), None);
        assert_eq!(Settings::parse("[1]"), None);
        assert_eq!(Settings::parse("{}"), Some(Settings::default()));
    }

    #[test]
    fn missing_file_is_created_with_defaults() {
        let dir = std::env::temp_dir().join(format!("reui-settings-test-{}", std::process::id()));
        let path = dir.join("clipboard.json");
        _ = std::fs::remove_file(&path);
        assert_eq!(Settings::load_from(&path), Settings::default());
        assert_eq!(Settings::parse(&std::fs::read_to_string(&path).unwrap()), Some(Settings::default()));
        std::fs::write(&path, "{ broken").unwrap();
        assert_eq!(Settings::load_from(&path), Settings::default());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ broken");
        Settings::save_layout(&path, "grid");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ broken");
        std::fs::write(&path, r#"{ "retention_days": 7, "extra": [1] }"#).unwrap();
        Settings::save_layout(&path, "grid");
        Settings::save_layout(&path, "bogus");
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!((saved["layout"].as_str(), saved["retention_days"].as_u64(), saved["extra"][0].as_u64()), (Some("grid"), Some(7), Some(1)));
        assert_eq!(Settings::load_from(&path).layout, "grid");
        _ = std::fs::remove_dir_all(&dir);
    }
}
