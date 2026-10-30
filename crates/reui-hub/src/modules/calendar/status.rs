//! How each source's last refresh went, for Settings' "Your calendars" rows
//! (`%LOCALAPPDATA%\ReUI\calendar\status.json`): whether it worked, when, how many events
//! it holds, when its body last changed, or why not. Without it a mistyped link would fail
//! in silence, and a provider serving an hours-old copy would look "updated just now".
//! Counts, times and reasons only: never a link, a title or a time of an event.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refresh {
    pub ok: bool,
    /// Unix ms of the attempt.
    pub at: u64,
    /// Events in the feed (all of them, not just this week's).
    pub events: usize,
    pub error: Option<String>,
    /// When the copy held last came with a different body (Unix ms): a 304, or the same
    /// body again, checks a feed without changing it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changed: Option<u64>,
    /// The server's own `Last-Modified` for that copy (Unix ms), when it sends one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<u64>,
}

/// By source id ("windows" for Windows' store).
pub type Status = BTreeMap<String, Refresh>;

pub fn path() -> PathBuf {
    crate::paths::local().join("calendar").join("status.json")
}

/// Blocking (disk).
pub fn save(status: &Status) {
    let path = path();
    if let Some(dir) = path.parent() {
        _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(status).unwrap_or_default();
    if let Err(e) = std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, &path)) {
        eprintln!("calendar: can't write the refresh state: {e}");
    }
}
