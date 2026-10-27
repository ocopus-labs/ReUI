//! One settings file: read, change a key, write back. The rules every ReUI settings file
//! follows: a missing file means the defaults; keys this window doesn't know are kept (and
//! keep their order); writes are atomic (temp file + rename); and a file that isn't a JSON
//! object is the user's to fix, so it is never written over: the page shows an error
//! instead.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

#[derive(Clone, Debug, PartialEq)]
pub enum Contents {
    Missing,
    Valid(Map<String, Value>),
    /// Not a JSON object; the parser's message.
    Invalid(String),
}

#[derive(Clone, Debug)]
pub struct Doc {
    pub path: PathBuf,
    pub contents: Contents,
    /// What the module uses when a key is missing.
    defaults: Map<String, Value>,
}

impl Doc {
    pub fn open(path: PathBuf, defaults: Value) -> Self {
        let Value::Object(defaults) = defaults else { panic!("defaults must be an object") };
        let contents = read(&path);
        Self { path, contents, defaults }
    }

    /// Reads the file again (it may have been edited by hand, or by the hub).
    pub fn reload(&mut self) {
        self.contents = read(&self.path);
    }

    pub fn name(&self) -> String {
        self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }

    pub fn error(&self) -> Option<&str> {
        match &self.contents {
            Contents::Invalid(e) => Some(e),
            _ => None,
        }
    }

    /// The file's value, or the default when it's missing (type checks are the caller's:
    /// a wrong type falls back exactly as the module does).
    pub fn get(&self, key: &str) -> Option<&Value> {
        match &self.contents {
            Contents::Valid(map) => map.get(key).or_else(|| self.defaults.get(key)),
            _ => self.defaults.get(key),
        }
    }

    pub fn default(&self, key: &str) -> Option<&Value> {
        self.defaults.get(key)
    }

    pub fn bool(&self, key: &str) -> bool {
        self.get(key).and_then(Value::as_bool).or_else(|| self.default(key).and_then(Value::as_bool)).unwrap_or(false)
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Value::as_str)
    }

    /// Sets `key` and writes the file. A missing file is written with every default, so it
    /// documents itself; an invalid one is refused.
    pub fn set(&mut self, key: &str, value: Value) -> Result<(), String> {
        // Someone may have changed it since we read it: merge into what's there now.
        let mut map = match read(&self.path) {
            Contents::Valid(map) => map,
            Contents::Missing => self.defaults.clone(),
            Contents::Invalid(e) => {
                let message = format!("{} isn't valid JSON ({e}); not writing it", self.name());
                self.contents = Contents::Invalid(e);
                return Err(message);
            }
        };
        map.insert(key.to_string(), value);
        write_atomic(&self.path, &(serde_json::to_string_pretty(&Value::Object(map.clone())).unwrap_or_default() + "\n")).map_err(|e| format!("can't write {}: {e}", self.path.display()))?;
        self.contents = Contents::Valid(map);
        Ok(())
    }
}

fn read(path: &Path) -> Contents {
    match std::fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) {
            Ok(Value::Object(map)) => Contents::Valid(map),
            Ok(_) => Contents::Invalid("not a JSON object".into()),
            Err(e) => Contents::Invalid(e.to_string()),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Contents::Missing,
        Err(e) => Contents::Invalid(e.to_string()),
    }
}

pub fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, text)?;
    std::fs::rename(&temp, path).inspect_err(|_| {
        _ = std::fs::remove_file(&temp);
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    pub fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("reui-settings-{name}-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_files_read_as_defaults_and_write_them_all() {
        let dir = temp_dir("missing");
        let mut doc = Doc::open(dir.join("media.json"), json!({ "lyrics": false, "swipe_to_skip": true }));
        assert_eq!(doc.contents, Contents::Missing);
        assert!(!doc.bool("lyrics") && doc.bool("swipe_to_skip"));
        doc.set("lyrics", json!(true)).unwrap();
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(&doc.path).unwrap()).unwrap();
        assert_eq!(saved, json!({ "lyrics": true, "swipe_to_skip": true }));
        assert!(!dir.join("media.json.tmp").exists());
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_keys_and_their_order_survive_a_write() {
        let dir = temp_dir("unknown");
        let path = dir.join("osd.json");
        std::fs::write(&path, r#"{ "zeta": 1, "volume": true, "alpha": { "x": [1, 2] } }"#).unwrap();
        let mut doc = Doc::open(path.clone(), json!({ "volume": true }));
        doc.set("volume", json!(false)).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let saved: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(saved, json!({ "zeta": 1, "volume": false, "alpha": { "x": [1, 2] } }));
        assert!(text.find("zeta").unwrap() < text.find("alpha").unwrap());
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_files_are_reported_and_never_written() {
        let dir = temp_dir("invalid");
        let path = dir.join("devices.json");
        std::fs::write(&path, "{ broken").unwrap();
        let mut doc = Doc::open(path.clone(), json!({ "drives": true }));
        assert!(doc.error().is_some());
        assert!(doc.bool("drives"), "reads as the defaults, like the module");
        assert!(doc.set("drives", json!(false)).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ broken");
        std::fs::write(&path, "[1]").unwrap();
        doc.reload();
        assert_eq!(doc.error(), Some("not a JSON object"));
        // Fixed by hand: writable again.
        std::fs::write(&path, "{}").unwrap();
        doc.reload();
        doc.set("drives", json!(false)).unwrap();
        assert!(!doc.bool("drives"));
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_broken_after_reading_is_not_written_over() {
        let dir = temp_dir("raced");
        let path = dir.join("media.json");
        std::fs::write(&path, "{}").unwrap();
        let mut doc = Doc::open(path.clone(), json!({ "lyrics": false }));
        std::fs::write(&path, "{ half-typed").unwrap();
        assert!(doc.set("lyrics", json!(true)).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ half-typed");
        assert!(doc.error().is_some());
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wrong_types_fall_back_to_the_default() {
        let dir = temp_dir("types");
        let path = dir.join("media.json");
        std::fs::write(&path, r#"{ "swipe_to_skip": "yes" }"#).unwrap();
        let doc = Doc::open(path, json!({ "swipe_to_skip": true }));
        assert!(doc.bool("swipe_to_skip"));
        _ = std::fs::remove_dir_all(&dir);
    }
}
