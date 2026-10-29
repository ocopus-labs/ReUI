//! `update.json` as the updates module sees it, and the decisions made from it: when the
//! daily check is due, which version to offer and when "Later" stops holding it back.
//! Settings writes the check's findings (`available`, `notes`, `setup_url`, …); this
//! module adds `version`, `updated_from`, `snoozed` and `snoozed_until`.

use std::path::Path;

use reui_protocol::version::newer;
use serde_json::{Map, Value};

use crate::paths;

pub const STATE_FILE: &str = "update.json";
pub const DAY_MS: u64 = 24 * 60 * 60 * 1000;
/// "Later" keeps a version's card away this long.
pub const SNOOZE_MS: u64 = DAY_MS;
/// Release notes are read for the card's detail up to this many characters.
const LINE_MAX: usize = 140;

pub fn read(path: &Path) -> Map<String, Value> {
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|v| v.as_object().cloned()).unwrap_or_default()
}

pub fn load() -> Map<String, Value> {
    read(&paths::local().join(STATE_FILE))
}

/// Merges `pairs` into `update.json` (Settings writes other keys of it).
pub fn write(pairs: Vec<(&str, Value)>) {
    let path = paths::local().join(STATE_FILE);
    let mut map = read(&path);
    for (k, v) in pairs {
        map.insert(k.into(), v);
    }
    let temp = path.with_extension("json.tmp");
    let result = std::fs::create_dir_all(paths::local())
        .and_then(|_| std::fs::write(&temp, serde_json::to_string_pretty(&Value::Object(map)).unwrap_or_default() + "\n"))
        .and_then(|_| std::fs::rename(&temp, &path));
    if let Err(e) = result {
        eprintln!("updates: can't write {}: {e}", path.display());
    }
}

/// A key of `settings.json` that defaults to on.
pub fn setting_on(key: &str) -> bool {
    read(&paths::config().join("settings.json")).get(key).and_then(Value::as_bool).unwrap_or(true)
}

/// This start against the last: the keys to write, and the version it was updated from.
pub fn version_change(state: &Map<String, Value>, current: &str) -> (Vec<(&'static str, Value)>, Option<String>) {
    match state.get("version").and_then(Value::as_str) {
        Some(last) if last == current => (Vec::new(), None),
        Some(last) => (vec![("version", Value::from(current)), ("updated_from", Value::from(last))], Some(last.to_string())),
        // A first install has nothing new to tell.
        None => (vec![("version", Value::from(current))], None),
    }
}

/// Whether the daily check is due.
pub fn check_due(state: &Map<String, Value>, now_ms: u64, enabled: bool) -> bool {
    enabled && state.get("checked_ms").and_then(Value::as_u64).is_none_or(|t| now_ms.saturating_sub(t) >= DAY_MS)
}

/// A version the island can install: what the card says about it.
#[derive(Clone, Debug, PartialEq)]
pub struct Offer {
    pub version: String,
    /// The release notes' first line worth reading.
    pub line: Option<String>,
}

/// The newer version a check found, if it has an installer (one without can't be
/// installed from the island, and About says so).
pub fn known(state: &Map<String, Value>, current: &str) -> Option<Offer> {
    let version = state.get("available")?.as_str()?;
    state.get("setup_url")?.as_str()?;
    newer(version, current).then(|| Offer { version: version.to_string(), line: first_line(state.get("notes").and_then(Value::as_str).unwrap_or_default()) })
}

/// What to offer now: [`known`], unless "Later" put this very version off and the day
/// isn't over. A newer version than the one put off shows at once.
pub fn offer(state: &Map<String, Value>, current: &str, now_ms: u64) -> Option<Offer> {
    let offer = known(state, current)?;
    let snoozed = state.get("snoozed").and_then(Value::as_str) == Some(offer.version.as_str()) && state.get("snoozed_until").and_then(Value::as_u64).is_some_and(|until| now_ms < until);
    (!snoozed).then_some(offer)
}

/// "Later" on `version`.
pub fn snooze(version: &str, now_ms: u64) -> Vec<(&'static str, Value)> {
    vec![("snoozed", Value::from(version)), ("snoozed_until", Value::from(now_ms + SNOOZE_MS))]
}

/// The first line of the notes that says something: not a heading such as "What's
/// changed", without its bullet, and short enough for two lines.
pub fn first_line(notes: &str) -> Option<String> {
    const HEADINGS: &[&str] = &["what's changed", "what’s changed", "what's new", "what’s new", "changes", "changelog", "highlights", "release notes", "new", "added", "changed", "fixed", "fixes", "improvements"];
    let line = notes.lines().map(|l| l.trim().trim_start_matches(['•', '-', '*']).trim()).find(|l| {
        let lower = l.trim_end_matches(':').to_lowercase();
        !l.is_empty() && !l.ends_with(':') && !HEADINGS.contains(&lower.as_str())
    })?;
    Some(match line.char_indices().nth(LINE_MAX) {
        Some((cut, _)) => format!("{}…", line[..cut].trim_end()),
        None => line.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn a_new_version_is_noticed_once_and_a_first_install_isnt_an_update() {
        assert_eq!(version_change(&map(json!({})), "0.2.0"), (vec![("version", json!("0.2.0"))], None));
        let (pairs, from) = version_change(&map(json!({ "version": "0.1.0" })), "0.2.0");
        assert_eq!(from.as_deref(), Some("0.1.0"));
        assert_eq!(pairs, vec![("version", json!("0.2.0")), ("updated_from", json!("0.1.0"))]);
        assert_eq!(version_change(&map(json!({ "version": "0.2.0", "updated_from": "0.1.0" })), "0.2.0"), (Vec::new(), None));
    }

    #[test]
    fn the_daily_check_runs_once_a_day_and_only_when_on() {
        let now = 10 * DAY_MS;
        assert!(check_due(&map(json!({})), now, true));
        assert!(!check_due(&map(json!({})), now, false));
        assert!(!check_due(&map(json!({ "checked_ms": now - DAY_MS / 2 })), now, true));
        assert!(check_due(&map(json!({ "checked_ms": now - DAY_MS })), now, true));
    }

    fn found(version: &str) -> Value {
        json!({ "available": version, "setup_url": format!("https://github.com/o/r/releases/download/v{version}/ReUI-Setup-{version}.exe"), "notes": "• Faster clipboard" })
    }

    #[test]
    fn only_a_newer_installable_version_is_offered() {
        let offered = known(&map(found("0.2.0")), "0.1.0").unwrap();
        assert_eq!(offered, Offer { version: "0.2.0".into(), line: Some("Faster clipboard".into()) });
        assert_eq!(known(&map(found("0.2.0")), "0.2.0"), None, "already running it");
        assert_eq!(known(&map(found("0.1.0")), "0.2.0"), None, "installed something newer by hand");
        assert_eq!(known(&map(json!({ "available": "0.2.0" })), "0.1.0"), None, "no installer");
        assert_eq!(known(&map(json!({ "available": null })), "0.1.0"), None);
    }

    #[test]
    fn later_holds_a_version_back_for_a_day_but_not_a_newer_one() {
        let now = 100 * DAY_MS;
        let mut state = map(found("0.2.0"));
        assert!(offer(&state, "0.1.0", now).is_some());
        for (k, v) in snooze("0.2.0", now) {
            state.insert(k.into(), v);
        }
        assert_eq!(offer(&state, "0.1.0", now), None);
        assert_eq!(offer(&state, "0.1.0", now + DAY_MS - 1), None);
        assert!(offer(&state, "0.1.0", now + DAY_MS).is_some(), "a day later it's back");
        assert!(known(&state, "0.1.0").is_some(), "the tray can still install it");

        let mut newer_found = map(found("0.3.0"));
        newer_found.extend(snooze("0.2.0", now).into_iter().map(|(k, v)| (k.to_string(), v)));
        assert_eq!(offer(&newer_found, "0.1.0", now).unwrap().version, "0.3.0");
    }

    #[test]
    fn the_detail_is_the_first_line_that_says_something() {
        assert_eq!(first_line("What's changed\n\n• Updates from the island by @someone in #12\n• More"), Some("Updates from the island by @someone in #12".into()));
        assert_eq!(first_line("Fixed:\n- Crash on resume"), Some("Crash on resume".into()));
        assert_eq!(first_line("First public build.\n\n- The island"), Some("First public build.".into()));
        assert_eq!(first_line("\n\nHighlights\n"), None);
        assert_eq!(first_line(""), None);
        let long = first_line(&"word ".repeat(100)).unwrap();
        assert!(long.ends_with('…') && long.chars().count() <= LINE_MAX + 1);
    }
}
