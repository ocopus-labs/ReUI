//! The calendar's sources as Settings shows and changes them (`calendar.json`; the module's
//! `settings.rs` reads the same). A link is a password to someone's calendar, so it's
//! encrypted with DPAPI for this user before it's written (`dpapi:` + hex, exactly what
//! the hub decrypts) and shown only by its host. How each source's last refresh went
//! comes from the hub's `%LOCALAPPDATA%\ReUI\calendar\status.json`.

use std::collections::HashMap;

use serde_json::{json, Value};
use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::Security::Cryptography::{CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB};

use super::doc::Doc;
use crate::paths::Paths;

/// One source as a row shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub id: String,
    pub host: String,
    pub on: bool,
}

/// The last refresh of a source, as the hub wrote it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refresh {
    pub ok: bool,
    pub at: u64,
    pub events: u64,
    pub error: Option<String>,
    /// When the feed's body last changed (unix ms), as the hub saw it.
    pub changed: Option<u64>,
    /// The server's `Last-Modified` for it (unix ms), when it sends one.
    pub modified: Option<u64>,
}

impl Refresh {
    /// A source's one line: how many events, and how fresh they really are. A check that
    /// found nothing new isn't an update, so the time the calendar last changed is said
    /// apart from the time it was asked whenever the two differ. A failure says why (the
    /// island quietly goes on with the last good copy, so this row is where it shows).
    pub fn line(&self, now: u64) -> String {
        if !self.ok {
            return format!("Couldn't refresh: {}", self.error.as_deref().unwrap_or("unknown"));
        }
        let events = if self.events == 1 { "1 event".into() } else { format!("{} events", self.events) };
        let checked = super::agents::ago(now, self.at);
        // The server knows best when it changed; never later than when it came.
        let changed = match (self.modified, self.changed) {
            (Some(m), Some(c)) => Some(m.min(c)),
            (m, c) => m.or(c),
        };
        match changed.map(|c| super::agents::ago(now, c)) {
            Some(changed) if changed != checked => format!("{events} · checked {checked} · changed {changed}"),
            _ => format!("{events} · updated {checked}"),
        }
    }
}

pub fn sources(doc: &Doc) -> Vec<Source> {
    let Some(list) = doc.get("sources").and_then(Value::as_array) else { return Vec::new() };
    list.iter()
        .filter_map(|s| {
            Some(Source {
                id: s.get("id").and_then(Value::as_str)?.to_string(),
                host: s.get("host").and_then(Value::as_str).unwrap_or("").to_string(),
                on: s.get("on").and_then(Value::as_bool).unwrap_or(true),
            })
        })
        .collect()
}

/// The `sources` value with `url` added (sealed), or why not.
pub fn add(doc: &Doc, url: &str) -> Result<Value, &'static str> {
    let url = normalize(url).ok_or("Paste an https link to a calendar (.ics)")?;
    let host = host(&url).unwrap_or_default();
    let mut list = doc.get("sources").and_then(Value::as_array).cloned().unwrap_or_default();
    let sealed = seal(&url).ok_or("Couldn't encrypt the link")?;
    let id = format!("{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()) & 0xff_ffff_ffff);
    list.push(json!({ "id": id, "host": host, "url": sealed, "on": true }));
    Ok(Value::Array(list))
}

/// The `sources` value with source `id` switched on or off, or removed.
pub fn change(doc: &Doc, id: &str, on: Option<bool>) -> Value {
    let mut list = doc.get("sources").and_then(Value::as_array).cloned().unwrap_or_default();
    match on {
        Some(on) => list.iter_mut().filter(|s| s.get("id").and_then(Value::as_str) == Some(id)).for_each(|s| s["on"] = Value::Bool(on)),
        None => list.retain(|s| s.get("id").and_then(Value::as_str) != Some(id)),
    }
    Value::Array(list)
}

/// The hub's refresh states, by source id ("windows" for Windows' store).
pub fn status(paths: &Paths) -> HashMap<String, Refresh> {
    let text = std::fs::read_to_string(paths.local.join("calendar").join("status.json")).unwrap_or_default();
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&text) else { return HashMap::new() };
    map.into_iter()
        .map(|(id, r)| {
            let refresh = Refresh {
                ok: r.get("ok").and_then(Value::as_bool).unwrap_or(false),
                at: r.get("at").and_then(Value::as_u64).unwrap_or(0),
                events: r.get("events").and_then(Value::as_u64).unwrap_or(0),
                error: r.get("error").and_then(Value::as_str).map(str::to_string),
                changed: r.get("changed").and_then(Value::as_u64),
                modified: r.get("modified").and_then(Value::as_u64),
            };
            (id, refresh)
        })
        .collect()
}

/// As the module reads links: `webcal://` is https; https anywhere, plain http only to
/// this PC.
pub fn normalize(url: &str) -> Option<String> {
    let url = url.trim();
    let url = if url.to_ascii_lowercase().starts_with("webcal://") { format!("https://{}", &url["webcal://".len()..]) } else { url.to_string() };
    let lower = url.to_ascii_lowercase();
    let local = host(&url).is_some_and(|h| h == "localhost" || h == "127.0.0.1" || h == "[::1]");
    let ok = lower.starts_with("https://") || lower.starts_with("http://") && local;
    (ok && host(&url).is_some_and(|h| !h.is_empty()) && !url.contains(char::is_whitespace)).then_some(url)
}

pub fn host(url: &str) -> Option<String> {
    let authority = url.split_once("://")?.1.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    let host = if host.starts_with('[') { host.split_inclusive(']').next()? } else { host.split(':').next()? };
    Some(host.to_ascii_lowercase())
}

fn seal(text: &str) -> Option<String> {
    let input = CRYPT_INTEGER_BLOB { cbData: text.len() as u32, pbData: text.as_ptr() as *mut u8 };
    let mut out = CRYPT_INTEGER_BLOB::default();
    unsafe { CryptProtectData(&input, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out).ok()? };
    let bytes = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) };
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    unsafe { _ = LocalFree(Some(HLOCAL(out.pbData.cast()))) };
    Some(format!("dpapi:{hex}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::doc::tests::temp_dir;

    #[test]
    fn links_are_sealed_shown_by_host_and_removed() {
        let dir = temp_dir("calendar-sources");
        let mut doc = Doc::open(dir.join("calendar.json"), json!({ "sources": [] }));
        let list = add(&doc, " webcal://calendar.example.com/private-abc/basic.ics ").unwrap();
        assert!(!list.to_string().contains("private-abc"), "only the sealed link is written");
        doc.set("sources", list).unwrap();
        let shown = sources(&doc);
        assert_eq!((shown.len(), shown[0].host.as_str(), shown[0].on), (1, "calendar.example.com", true));
        doc.set("sources", change(&doc, &shown[0].id, Some(false))).unwrap();
        assert!(!sources(&doc)[0].on);
        doc.set("sources", change(&doc, &shown[0].id, None)).unwrap();
        assert!(sources(&doc).is_empty());
        assert!(add(&doc, "http://example.com/cal.ics").is_err());
        assert!(add(&doc, "not a link").is_err());
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_check_is_told_apart_from_a_change() {
        const MIN: u64 = 60_000;
        let now = 1_000 * MIN;
        let ok = |at, changed, modified| Refresh { ok: true, at, events: 12, error: None, changed, modified };
        assert_eq!(ok(now - 2 * MIN, Some(now - 180 * MIN), None).line(now), "12 events · checked 2 min ago · changed 3 h ago");
        assert_eq!(ok(now, Some(now), None).line(now), "12 events · updated just now", "a new body is an update");
        assert_eq!(ok(now, None, None).line(now), "12 events · updated just now", "an older hub's status");
        assert_eq!(ok(now, Some(now), Some(now - 300 * MIN)).line(now), "12 events · checked just now · changed 5 h ago", "the server's own date when older");
        let failed = Refresh { ok: false, at: now, events: 12, error: Some("was sent to a web page".into()), changed: Some(0), modified: None };
        assert_eq!(failed.line(now), "Couldn't refresh: was sent to a web page");
        let single = Refresh { events: 1, ..ok(now, None, None) };
        assert_eq!(single.line(now), "1 event · updated just now");
    }
}
