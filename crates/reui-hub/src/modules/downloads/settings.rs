//! Downloads settings, in `%APPDATA%\ReUI\downloads.json`, written by the Settings window
//! too (which then sends the module `reload`). Same rules as the other modules': created
//! with the defaults when missing, a missing or mistyped field falls back to its default,
//! and a file that isn't JSON is never overwritten. `enabled` is read by `main.rs` at start.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Settings {
    /// Watched besides the Downloads folder (a browser set to save elsewhere).
    pub folders: Vec<PathBuf>,
}

impl Settings {
    pub fn path() -> PathBuf {
        crate::paths::config().join("downloads.json")
    }

    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).unwrap_or_else(|| {
                eprintln!("downloads: {} isn't valid JSON; using the defaults", path.display());
                Self::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if let Some(dir) = path.parent() {
                    _ = std::fs::create_dir_all(dir);
                }
                let defaults = json!({ "enabled": true, "folders": [] });
                if let Err(e) = std::fs::write(path, serde_json::to_string_pretty(&defaults).unwrap_or_default()) {
                    eprintln!("downloads: can't write {}: {e}", path.display());
                }
                Self::default()
            }
            Err(e) => {
                eprintln!("downloads: can't read {}: {e}", path.display());
                Self::default()
            }
        }
    }

    /// `None` if the text isn't a JSON object. Blank entries are dropped.
    pub fn parse(text: &str) -> Option<Self> {
        let Value::Object(map) = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()? else { return None };
        let folders = map.get("folders").and_then(Value::as_array).map(|list| list.iter().filter_map(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(PathBuf::from).collect()).unwrap_or_default();
        Some(Self { folders })
    }

    /// The folders to watch: Downloads (`downloads`, wherever it is) and the extra ones,
    /// each once however it's spelled.
    pub fn watched(&self, downloads: Option<PathBuf>) -> Vec<PathBuf> {
        let mut list: Vec<PathBuf> = Vec::new();
        for folder in downloads.into_iter().chain(self.folders.iter().cloned()) {
            let key = |p: &Path| p.to_string_lossy().trim_end_matches(['\\', '/']).to_lowercase();
            if !list.iter().any(|f| key(f) == key(&folder)) {
                list.push(folder);
            }
        }
        list
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_parse_leniently_and_are_watched_once() {
        assert_eq!(Settings::parse("{}"), Some(Settings::default()));
        assert_eq!(Settings::parse("[]"), None);
        let s = Settings::parse(r#"{ "folders": ["D:\\Saves", " ", 3, "c:\\users\\me\\downloads\\"] }"#).unwrap();
        assert_eq!(s.folders.len(), 2);
        assert_eq!(s.watched(Some(PathBuf::from(r"C:\Users\me\Downloads"))), vec![PathBuf::from(r"C:\Users\me\Downloads"), PathBuf::from(r"D:\Saves")]);
        assert_eq!(Settings::parse(r#"{ "folders": "D:\\x" }"#).unwrap().watched(None), Vec::<PathBuf>::new());
    }

    #[test]
    fn a_missing_file_is_created_and_a_broken_one_kept() {
        let dir = std::env::temp_dir().join(format!("reui-downloads-settings-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("downloads.json");
        assert_eq!(Settings::load_from(&path), Settings::default());
        let created: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!((created["enabled"].as_bool(), created["folders"].as_array().map(Vec::len)), (Some(true), Some(0)));
        std::fs::write(&path, "{ broken").unwrap();
        assert_eq!(Settings::load_from(&path), Settings::default());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ broken");
        _ = std::fs::remove_dir_all(&dir);
    }
}
