//! Calendar settings, in `%APPDATA%\ReUI\calendar.json`, written by the Settings window too
//! (which then sends the module `reload`). Same rules as the other modules': created with
//! the defaults when missing, a mistyped field falls back to its default, and a file that
//! isn't JSON is never overwritten. One more: a link typed into the file by hand (no
//! `dpapi:`) is encrypted the first time it's read, and the file rewritten with every key
//! it had (PRD §5.5).

use std::fmt;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::dpapi;

#[derive(Clone, PartialEq, Eq)]
pub struct Source {
    /// Stable: keys the cache file, the refresh state and dismissed starts.
    pub id: String,
    /// What Settings shows ("calendar.google.com").
    pub host: String,
    /// The secret link itself; never logged (no `Debug`).
    pub url: String,
    pub on: bool,
}

impl fmt::Debug for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Source").field("id", &self.id).field("host", &self.host).field("on", &self.on).finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// Minutes before a meeting the countdown starts.
    pub lead_min: u32,
    /// Read Windows' own calendar store (PRD §6.1).
    pub windows: bool,
    pub sources: Vec<Source>,
}

impl Default for Settings {
    fn default() -> Self {
        Self { lead_min: 10, windows: false, sources: Vec::new() }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        crate::paths::config().join("calendar.json")
    }

    /// Reads the file, encrypting any plain link in it first. Blocking (DPAPI, the disk).
    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &Path) -> Self {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                write(path, &defaults());
                return Self::default();
            }
            Err(e) => {
                eprintln!("calendar: can't read {}: {e}", path.display());
                return Self::default();
            }
        };
        let Ok(Value::Object(mut map)) = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) else {
            eprintln!("calendar: {} isn't valid JSON; using the defaults", path.display());
            return Self::default();
        };
        if seal_plain(&mut map) {
            write(path, &Value::Object(map.clone()));
        }
        Self::from_map(&map)
    }

    fn from_map(map: &serde_json::Map<String, Value>) -> Self {
        let d = Self::default();
        let sources = map
            .get("sources")
            .and_then(Value::as_array)
            .map(|list| list.iter().filter_map(source).collect())
            .unwrap_or_default();
        Self {
            lead_min: map.get("lead_min").and_then(Value::as_u64).filter(|m| (1..=60).contains(m)).map_or(d.lead_min, |m| m as u32),
            windows: map.get("windows").and_then(Value::as_bool).unwrap_or(d.windows),
            sources,
        }
    }

    pub fn lead_ms(&self) -> u64 {
        u64::from(self.lead_min) * 60_000
    }
}

fn source(v: &Value) -> Option<Source> {
    let url = dpapi::open(v.get("url")?.as_str()?)?;
    let url = normalize(&url)?;
    let id = v.get("id").and_then(Value::as_str).filter(|s| !s.is_empty()).map_or_else(|| id_for(&url), str::to_string);
    Some(Source { id, host: host(&url).unwrap_or_default(), on: v.get("on").and_then(Value::as_bool).unwrap_or(true), url })
}

/// Encrypts the links written in the clear; true if any was.
fn seal_plain(map: &mut serde_json::Map<String, Value>) -> bool {
    let Some(Value::Array(list)) = map.get_mut("sources") else { return false };
    let mut changed = false;
    for entry in list.iter_mut() {
        let Some(obj) = entry.as_object_mut() else { continue };
        let Some(plain) = obj.get("url").and_then(Value::as_str).filter(|u| !u.starts_with(dpapi::PREFIX)).map(str::to_string) else { continue };
        let (Some(url), Some(sealed)) = (normalize(&plain), dpapi::seal(&plain)) else { continue };
        obj.insert("url".into(), Value::String(sealed));
        obj.entry("id").or_insert_with(|| Value::String(id_for(&url)));
        obj.entry("host").or_insert_with(|| Value::String(host(&url).unwrap_or_default()));
        obj.entry("on").or_insert(Value::Bool(true));
        changed = true;
    }
    changed
}

/// A link as fetched: `webcal://` read as https; https anywhere, plain http only to this
/// PC (a test server). `None` for anything else.
pub fn normalize(url: &str) -> Option<String> {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    let url = if lower.starts_with("webcal://") { format!("https://{}", &url["webcal://".len()..]) } else { url.to_string() };
    let lower = url.to_ascii_lowercase();
    let ok = lower.starts_with("https://") || lower.starts_with("http://") && host(&url).is_some_and(|h| h == "localhost" || h == "127.0.0.1" || h == "[::1]");
    (ok && host(&url).is_some_and(|h| !h.is_empty()) && !url.contains(char::is_whitespace)).then_some(url)
}

pub fn host(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    let host = if host.starts_with('[') { host.split_inclusive(']').next()? } else { host.split(':').next()? };
    Some(host.to_ascii_lowercase())
}

/// A short id for a link that came without one (hand-written into the file).
fn id_for(url: &str) -> String {
    blake3::hash(url.as_bytes()).to_hex()[..10].to_string()
}

fn defaults() -> Value {
    json!({ "enabled": true, "lead_min": 10, "windows": false, "sources": [] })
}

fn write(path: &Path, value: &Value) {
    if let Some(dir) = path.parent() {
        _ = std::fs::create_dir_all(dir);
    }
    let text = serde_json::to_string_pretty(value).unwrap_or_default();
    let tmp = path.with_extension("json.tmp");
    if let Err(e) = std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, path)) {
        eprintln!("calendar: can't write {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_are_normalized_and_only_safe_ones_kept() {
        assert_eq!(normalize("webcal://p01-caldav.icloud.com/published/2/abc").as_deref(), Some("https://p01-caldav.icloud.com/published/2/abc"));
        assert_eq!(normalize(" https://calendar.google.com/calendar/ical/x/private-y/basic.ics ").as_deref(), Some("https://calendar.google.com/calendar/ical/x/private-y/basic.ics"));
        assert!(normalize("http://example.com/cal.ics").is_none(), "plain http only to this PC");
        assert!(normalize("http://127.0.0.1:8123/test.ics").is_some());
        assert!(normalize("file:///C:/cal.ics").is_none());
        assert!(normalize("https:// spaced.ics").is_none());
        assert_eq!(host("https://user@outlook.office365.com:443/owa/calendar/x/reachcalendar.ics").as_deref(), Some("outlook.office365.com"));
    }

    #[test]
    fn a_hand_typed_link_is_sealed_and_the_file_keeps_its_keys() {
        let dir = std::env::temp_dir().join(format!("reui-calendar-settings-{}", std::process::id()));
        _ = std::fs::create_dir_all(&dir);
        let path = dir.join("calendar.json");
        std::fs::write(&path, r#"{ "lead_min": 5, "future_key": 1, "sources": [ { "url": "https://calendar.example.com/secret.ics" } ] }"#).unwrap();
        let s = Settings::load_from(&path);
        assert_eq!(s.lead_min, 5);
        assert_eq!(s.sources.len(), 1);
        assert_eq!((s.sources[0].host.as_str(), s.sources[0].url.as_str(), s.sources[0].on), ("calendar.example.com", "https://calendar.example.com/secret.ics", true));
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(!written.contains("secret.ics"), "the link is no longer in the clear");
        assert!(written.contains("future_key"));
        assert!(!format!("{:?}", s.sources[0]).contains("secret"), "Debug never prints the link");
        let again = Settings::load_from(&path);
        assert_eq!(again, s, "and reads back the same");
        _ = std::fs::remove_dir_all(&dir);
        assert_eq!(Settings::from_map(json!({ "lead_min": 0, "windows": "yes" }).as_object().unwrap()), Settings::default());
    }
}
