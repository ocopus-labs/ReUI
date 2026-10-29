//! Quieting a kind of moment from the moment itself (docs/PRD-behaviour.md §4.4): a
//! right-click on a brief or an alert offers "Don't show these" (its module's own setting
//! for it, as the module declared in `Activity::quiet`) and "Snooze for 1 hour" (every
//! brief and alert of that module, kept in settings.json `snoozed`: module → Unix ms).
//! The rules are pure; [`turn_off`] writes a module's settings file as Settings would.

use std::path::Path;

use reui_protocol::{Activity, Priority, QuietSetting};
use serde_json::{Map, Value};

/// How long "Snooze for 1 hour" lasts.
pub const SNOOZE_MS: u64 = 3_600_000;

/// A menu entry's action: the island handles these itself (no module gets them).
pub const ACTION_OFF: &str = "quiet:off";
pub const ACTION_SNOOZE: &str = "quiet:snooze";

/// The menu for a moment: label and action, or nothing when it can't be quieted (a
/// decision, something ongoing, a module that declared nothing).
pub fn entries(activity: &Activity) -> Vec<(&'static str, &'static str)> {
    let Some(quiet) = activity.quiet.as_ref().filter(|_| matches!(activity.priority, Priority::Brief | Priority::Alert)) else { return Vec::new() };
    let off = quiet.setting.as_ref().map(|_| ("Don't show these", ACTION_OFF));
    off.into_iter().chain([("Snooze for 1 hour", ACTION_SNOOZE)]).collect()
}

/// `snoozed` from settings.json: (module, until Unix ms).
pub fn parse_snoozed(value: Option<&Value>) -> Vec<(String, u64)> {
    let Some(Value::Object(map)) = value else { return Vec::new() };
    let mut list: Vec<(String, u64)> = map.iter().filter_map(|(module, until)| Some((module.clone(), until.as_u64()?))).collect();
    list.sort();
    list
}

/// Whether `module`'s moments are snoozed at `now`.
pub fn snoozed(list: &[(String, u64)], module: &str, now: u64) -> bool {
    list.iter().any(|(m, until)| m == module && *until > now)
}

/// Whether `activity` is held back by a snooze: a snoozed module's moments (briefs and
/// alerts). Never a decision, which only the user can answer, nor what the user is in
/// (music, a running timer, a place they opened), which isn't an interruption.
pub fn held(list: &[(String, u64)], activity: &Activity, now: u64) -> bool {
    matches!(activity.priority, Priority::Brief | Priority::Alert) && snoozed(list, &activity.id.module, now)
}

/// The list with `module` snoozed until `until`, and the snoozes that have ended dropped.
pub fn snooze(list: &[(String, u64)], module: &str, until: u64, now: u64) -> Vec<(String, u64)> {
    let mut list: Vec<(String, u64)> = list.iter().filter(|(m, end)| m != module && *end > now).cloned().collect();
    list.push((module.to_string(), until));
    list.sort();
    list
}

/// Milliseconds until the next snooze ends, if one is running.
pub fn next_end(list: &[(String, u64)], now: u64) -> Option<u64> {
    list.iter().filter(|(_, until)| *until > now).map(|(_, until)| until - now).min()
}

/// A settings file the island may write: a plain `name.json` in ReUI's folder, never a path
/// (the module names it; this keeps a bad name from writing anywhere else).
pub fn valid_file(name: &str) -> bool {
    name.strip_suffix(".json").is_some_and(|stem| !stem.is_empty() && stem.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-'))
}

/// Sets the moment's setting to `false` in `dir`, keeping every other key, atomically. A
/// missing file gets just this key (the module reads the rest as defaults); a file that
/// isn't a JSON object is the user's to fix and is never written over.
pub fn turn_off(dir: &Path, setting: &QuietSetting) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind};
    if !valid_file(&setting.file) || setting.key.is_empty() {
        return Err(Error::new(ErrorKind::InvalidInput, "not a settings file"));
    }
    let path = dir.join(&setting.file);
    let mut map = match std::fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(map)) => map,
            _ => return Err(Error::new(ErrorKind::InvalidData, "not a JSON object; left as it is")),
        },
        Err(e) if e.kind() == ErrorKind::NotFound => Map::new(),
        Err(e) => return Err(e),
    };
    map.insert(setting.key.clone(), Value::Bool(false));
    std::fs::create_dir_all(dir)?;
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, serde_json::to_string_pretty(&Value::Object(map)).unwrap_or_default() + "\n")?;
    std::fs::rename(&temp, &path)
}


#[cfg(test)]
mod tests {
    use super::*;
    use reui_protocol::{ActivityId, Element, Quiet};

    fn moment(module: &str, priority: Priority, quiet: Option<Quiet>) -> Activity {
        Activity { id: ActivityId { module: module.into(), key: "k".into() }, priority, accent: None, compact: Default::default(), minimal: Element::Empty, expanded: None, audio: None, quiet }
    }

    #[test]
    fn the_menu_offers_what_the_module_declared_and_never_for_decisions() {
        let volume = Some(Quiet::setting("osd.json", "volume"));
        assert_eq!(entries(&moment("osd", Priority::Brief, volume.clone())), vec![("Don't show these", ACTION_OFF), ("Snooze for 1 hour", ACTION_SNOOZE)]);
        assert_eq!(entries(&moment("timers", Priority::Alert, Some(Quiet::snooze_only()))), vec![("Snooze for 1 hour", ACTION_SNOOZE)]);
        assert!(entries(&moment("osd", Priority::Decision, volume.clone())).is_empty());
        assert!(entries(&moment("osd", Priority::Ongoing, volume)).is_empty());
        assert!(entries(&moment("osd", Priority::Brief, None)).is_empty());
    }

    #[test]
    fn a_snooze_holds_back_all_but_decisions_until_it_ends() {
        let list = snooze(&[], "osd", 1_000, 0);
        assert!(held(&list, &moment("osd", Priority::Brief, None), 999));
        assert!(held(&list, &moment("osd", Priority::Alert, None), 999));
        assert!(!held(&list, &moment("osd", Priority::Decision, None), 999), "decisions always come through");
        assert!(!held(&list, &moment("osd", Priority::Ongoing, None), 999), "not an interruption");
        assert!(!held(&list, &moment("devices", Priority::Brief, None), 999));
        assert!(!held(&list, &moment("osd", Priority::Brief, None), 1_000), "ended");
        assert_eq!(next_end(&list, 400), Some(600));
        assert_eq!(next_end(&list, 1_000), None);
    }

    #[test]
    fn snoozing_again_replaces_and_drops_ended_ones() {
        let list = vec![("devices".to_string(), 50), ("osd".to_string(), 500)];
        assert_eq!(snooze(&list, "osd", 2_000, 100), vec![("osd".to_string(), 2_000)]);
        let value: Value = serde_json::from_str(r#"{ "osd": 5, "devices": "x", "timers": 9 }"#).unwrap();
        assert_eq!(parse_snoozed(Some(&value)), vec![("osd".to_string(), 5), ("timers".to_string(), 9)]);
        assert!(parse_snoozed(None).is_empty());
    }

    #[test]
    fn only_plain_settings_files_are_written() {
        assert!(valid_file("osd.json") && valid_file("devices.json") && valid_file("my_file-2.json"));
        assert!(!valid_file("../settings.json") && !valid_file(r"C:\x.json") && !valid_file(".json") && !valid_file("osd.txt") && !valid_file("OSD.json"));
    }

    #[test]
    fn turning_off_keeps_other_keys_and_never_clobbers_a_broken_file() {
        let dir = std::env::temp_dir().join(format!("reui-island-quiet-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&dir);
        let volume = QuietSetting { file: "osd.json".into(), key: "volume".into() };
        turn_off(&dir, &volume).unwrap();
        let read = |name: &str| serde_json::from_str::<Value>(&std::fs::read_to_string(dir.join(name)).unwrap()).unwrap();
        assert_eq!(read("osd.json")["volume"], false);
        std::fs::write(dir.join("osd.json"), r#"{ "brightness": true, "future": [1], "volume": true }"#).unwrap();
        turn_off(&dir, &volume).unwrap();
        let saved = read("osd.json");
        assert_eq!((saved["volume"].as_bool(), saved["brightness"].as_bool(), saved["future"][0].as_u64()), (Some(false), Some(true), Some(1)));
        std::fs::write(dir.join("osd.json"), "{ broken").unwrap();
        assert!(turn_off(&dir, &volume).is_err());
        assert_eq!(std::fs::read_to_string(dir.join("osd.json")).unwrap(), "{ broken");
        assert!(turn_off(&dir, &QuietSetting { file: "../x.json".into(), key: "k".into() }).is_err());
        _ = std::fs::remove_dir_all(&dir);
    }
}
