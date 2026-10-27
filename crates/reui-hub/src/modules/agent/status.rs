//! What Settings shows about each agent: when its last hook event reached ReUI, in
//! `%LOCALAPPDATA%\ReUI\agents.json` (`{ "claude": { "last_event_ms": 1761600000000 } }`).
//! Written at most once per [`MIN_INTERVAL`] per agent, so a busy session costs a write
//! every 10 s at most; Settings shows minutes ("last event 2 min ago"), so that's plenty.
//!
//! Also the test event `reui-bridge test <agent>` sends (Settings' "Send test event"),
//! which the module answers with a short confirmation and never treats as a session.

use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{Map, Value};
use tokio::time::Instant;

pub const FILE: &str = "agents.json";
pub const MIN_INTERVAL: Duration = Duration::from_secs(10);
/// The event name `reui-bridge test` sends.
pub const TEST_EVENT: &str = "ReUITest";

/// The three module instances share the file.
static WRITING: Mutex<()> = Mutex::new(());

/// Whether a hook payload is the bridge's test event.
pub fn is_test(payload: &str) -> bool {
    serde_json::from_str::<Value>(payload).ok().and_then(|v| v.get("hook_event_name").and_then(Value::as_str).map(|e| e == TEST_EVENT)).unwrap_or(false)
}

/// One write per [`MIN_INTERVAL`].
#[derive(Default)]
pub struct Throttle {
    last: Option<Instant>,
}

impl Throttle {
    /// Whether to write now; if so, counts it.
    pub fn due(&mut self, now: Instant) -> bool {
        let due = self.last.is_none_or(|last| now.duration_since(last) >= MIN_INTERVAL);
        if due {
            self.last = Some(now);
        }
        due
    }
}

/// Notes `agent`'s last event at `ms`, keeping the other agents' entries.
pub fn record(dir: &Path, agent: &str, ms: u64) -> std::io::Result<()> {
    let _guard = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    let path = dir.join(FILE);
    let mut root = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|v| v.as_object().cloned()).unwrap_or_default();
    let mut entry = root.get(agent).and_then(Value::as_object).cloned().unwrap_or_else(Map::new);
    entry.insert("last_event_ms".into(), Value::from(ms));
    root.insert(agent.into(), Value::Object(entry));
    std::fs::create_dir_all(dir)?;
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, serde_json::to_string_pretty(&Value::Object(root)).unwrap_or_default() + "\n")?;
    std::fs::rename(&temp, &path)
}

/// Records this event off the hub thread, if the throttle allows.
pub fn note_event(throttle: &Mutex<Throttle>, agent: &'static str) {
    if !throttle.lock().map(|mut t| t.due(Instant::now())).unwrap_or(false) {
        return;
    }
    let dir = crate::paths::local();
    let ms = reui_protocol::unix_ms();
    tokio::task::spawn_blocking(move || {
        if let Err(e) = record(&dir, agent, ms) {
            eprintln!("{agent}: can't write {FILE}: {e}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_are_throttled_to_one_per_interval() {
        let start = Instant::now();
        let mut throttle = Throttle::default();
        assert!(throttle.due(start));
        assert!(!throttle.due(start + Duration::from_secs(3)));
        assert!(!throttle.due(start + Duration::from_millis(9_999)));
        assert!(throttle.due(start + MIN_INTERVAL));
        assert!(!throttle.due(start + MIN_INTERVAL + Duration::from_secs(1)));
        assert!(throttle.due(start + Duration::from_secs(25)));
    }

    #[test]
    fn each_agent_keeps_its_own_entry() {
        let dir = std::env::temp_dir().join(format!("reui-hub-agents-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&dir);
        record(&dir, "claude", 1).unwrap();
        record(&dir, "codex", 2).unwrap();
        record(&dir, "claude", 3).unwrap();
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(dir.join(FILE)).unwrap()).unwrap();
        assert_eq!(saved, serde_json::json!({ "claude": { "last_event_ms": 3 }, "codex": { "last_event_ms": 2 } }));
        assert!(!dir.join("agents.json.tmp").exists());
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_test_event_is_recognised() {
        assert!(is_test(r#"{ "hook_event_name": "ReUITest", "session_id": "reui-test" }"#));
        assert!(!is_test(r#"{ "hook_event_name": "Stop", "session_id": "abc" }"#));
        assert!(!is_test("not json"));
    }
}
