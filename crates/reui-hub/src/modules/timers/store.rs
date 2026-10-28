//! Running and paused timers in `%APPDATA%\ReUI\timer-state.json`, so they survive a restart or a
//! reboot (S9). Written on every change, never on a tick. Across a restart only the wall
//! clock connects the two runs, so a running timer is saved as when it ends; while the hub
//! runs, the monotonic clock is the truth and a clock change moves nothing (E-T6).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::model::{Finished, Timers};

#[derive(Serialize, Deserialize, Default)]
struct File {
    #[serde(default)]
    timers: Vec<Saved>,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    length_ms: u64,
    /// Time left, for a paused timer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    left_ms: Option<u64>,
    /// When a running timer ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ends_at_unix_ms: Option<u64>,
}

pub fn path() -> PathBuf {
    crate::paths::config().join("timer-state.json")
}

pub fn save(path: &Path, timers: &Timers, now: u64, now_unix: u64) {
    let file = File {
        timers: timers
            .all()
            .iter()
            .map(|t| Saved {
                label: t.label.clone(),
                length_ms: t.length_ms,
                left_ms: (!t.running()).then(|| t.left(now)),
                ends_at_unix_ms: t.running().then(|| now_unix + t.left(now)),
            })
            .collect(),
    };
    let Ok(text) = serde_json::to_string_pretty(&file) else { return };
    let tmp = path.with_extension("json.tmp");
    if let Err(e) = std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, path)) {
        eprintln!("timers: can't save {}: {e}", path.display());
    }
}

/// The saved timers, put back on the monotonic clock at `now`; those that ended while ReUI
/// wasn't running come back as late finishes (E-T2).
pub fn load(path: &Path, timers: &mut Timers, now: u64, now_unix: u64) -> Vec<Finished> {
    let file: File = match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
            eprintln!("timers: {} isn't readable ({e}); starting with none", path.display());
            File::default()
        }),
        Err(_) => File::default(),
    };
    let mut finished = Vec::new();
    for saved in file.timers {
        match (saved.ends_at_unix_ms, saved.left_ms) {
            (Some(end), _) if end <= now_unix => finished.push(Finished { label: saved.label, length_ms: saved.length_ms, late_ms: now_unix - end }),
            (Some(end), _) => _ = timers.add(saved.label, saved.length_ms, end - now_unix, Some(now)),
            (None, Some(left)) if left > 0 => _ = timers.add(saved.label, saved.length_ms, left, None),
            _ => {}
        }
    }
    finished
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_late_finishes() {
        let dir = std::env::temp_dir().join(format!("reui-timers-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("timers.json");
        let mut t = Timers::default();
        t.start(Some("Tea".into()), 4 * 60_000, 0);
        let bread = t.start(Some("Bread".into()), 40 * 60_000, 0).unwrap();
        t.toggle(bread, 60_000);
        save(&path, &t, 60_000, 1_000_000);

        // Back 10 minutes later: tea ended 7 minutes ago, bread is still paused at 39 min.
        let mut back = Timers::default();
        let finished = load(&path, &mut back, 5, 1_000_000 + 600_000);
        assert_eq!(finished, vec![Finished { label: Some("Tea".into()), length_ms: 240_000, late_ms: 420_000 }]);
        let left: Vec<(u64, bool)> = back.all().iter().map(|t| (t.left(5), t.running())).collect();
        assert_eq!(left, vec![(39 * 60_000, false)]);

        // Back a minute later: tea runs on with 2 minutes left.
        let mut soon = Timers::default();
        assert!(load(&path, &mut soon, 0, 1_000_000 + 60_000).is_empty());
        assert_eq!(soon.ordered(0)[0].left(0), 120_000);

        std::fs::write(&path, "{ broken").unwrap();
        assert!(load(&path, &mut Timers::default(), 0, 0).is_empty());
        _ = std::fs::remove_dir_all(dir);
    }
}
