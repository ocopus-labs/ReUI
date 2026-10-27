//! Token usage from Claude Code transcripts (`~/.claude/projects/**/*.jsonl`).
//!
//! Each assistant line carries `message.usage`. Streaming writes several lines for one
//! message, so usage is keyed by `message.id` + `requestId` and the latest line wins.
//! Files are read incrementally from where we stopped, only up to the last full line.
//!
//! Cache reads are kept apart: they are the same context re-read on every call, usually
//! ~97% of all tokens and ~10% of the price, so headline numbers use [`Usage::fresh`].

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use chrono::{DateTime, Local, NaiveDate};
use serde_json::Value;

#[derive(Default, Clone, Copy, Debug, PartialEq)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_write: u64,
    pub cache_read: u64,
}

impl Usage {
    /// Tokens actually processed: input, output and cache writes (not cache re-reads).
    pub fn fresh(&self) -> u64 {
        self.input + self.output + self.cache_write
    }

    fn add(mut self, other: &Usage) -> Usage {
        self.input += other.input;
        self.output += other.output;
        self.cache_write += other.cache_write;
        self.cache_read += other.cache_read;
        self
    }
}

#[derive(Default)]
pub struct Ledger {
    files: HashMap<PathBuf, FileUsage>,
}

#[derive(Default)]
struct FileUsage {
    offset: u64,
    /// message key → (usage, local day)
    messages: HashMap<String, (Usage, NaiveDate)>,
}

impl Ledger {
    /// Reads whatever was appended to `path` since the last call.
    pub fn update(&mut self, path: &Path) {
        let file = self.files.entry(path.to_path_buf()).or_default();
        let Ok(mut handle) = std::fs::File::open(path) else { return };
        if handle.seek(SeekFrom::Start(file.offset)).is_err() {
            return;
        }
        let mut bytes = Vec::new();
        if handle.read_to_end(&mut bytes).is_err() {
            return;
        }
        // A line still being written is picked up next time.
        let Some(end) = bytes.iter().rposition(|b| *b == b'\n') else { return };
        for (i, line) in bytes[..end].split(|b| *b == b'\n').enumerate() {
            if let Some((key, usage, day)) = parse_line(line) {
                let key = key.unwrap_or_else(|| format!("@{}:{i}", file.offset));
                file.messages.insert(key, (usage, day));
            }
        }
        file.offset += end as u64 + 1;
    }

    pub fn file_total(&self, path: &Path) -> Usage {
        self.files.get(path).map_or_else(Usage::default, |f| f.messages.values().fold(Usage::default(), |sum, (u, _)| sum.add(u)))
    }

    pub fn today_total(&self) -> Usage {
        let today = Local::now().date_naive();
        self.files.values().flat_map(|f| f.messages.values()).filter(|(_, day)| *day == today).fold(Usage::default(), |sum, (u, _)| sum.add(u))
    }

    /// Reads every transcript touched today. Takes the lock per file, so live updates
    /// aren't held up behind a long scan.
    pub fn scan_today(ledger: &Mutex<Ledger>, root: &Path) {
        let today = Local::now().date_naive();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(meta) = entry.metadata() else { continue };
                if meta.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "jsonl") && meta.modified().map(local_day).ok() == Some(today) {
                    ledger.lock().unwrap().update(&path);
                }
            }
        }
    }
}

fn local_day(time: SystemTime) -> NaiveDate {
    DateTime::<Local>::from(time).date_naive()
}

/// (dedupe key, usage, local day) for a line that carries usage.
fn parse_line(line: &[u8]) -> Option<(Option<String>, Usage, NaiveDate)> {
    // Cheap pre-check: most lines (tool results, user turns) carry no usage.
    if !line.windows(7).any(|w| w == b"\"usage\"") {
        return None;
    }
    let value: Value = serde_json::from_slice(line).ok()?;
    let message = value.get("message")?;
    let usage = message.get("usage")?;
    let count = |k: &str| usage.get(k).and_then(Value::as_u64).unwrap_or(0);
    let tokens = Usage {
        input: count("input_tokens"),
        output: count("output_tokens"),
        cache_write: count("cache_creation_input_tokens"),
        cache_read: count("cache_read_input_tokens"),
    };
    let day = DateTime::parse_from_rfc3339(value.get("timestamp")?.as_str()?).ok()?.with_timezone(&Local).date_naive();
    let key = match (message.get("id").and_then(Value::as_str), value.get("requestId").and_then(Value::as_str)) {
        (Some(id), Some(request)) => Some(format!("{id}:{request}")),
        (Some(id), None) => Some(id.to_string()),
        _ => None,
    };
    Some((key, tokens, day))
}

pub fn claude_projects_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").map(|home| PathBuf::from(home).join(".claude").join("projects"))
}

/// Local midnight in Unix milliseconds, for "today" in the agents' databases.
fn midnight_ms() -> i64 {
    let midnight = Local::now().date_naive().and_hms_opt(0, 0, 0).unwrap();
    midnight.and_local_timezone(Local).earliest().map_or(0, |t| t.timestamp_millis())
}

fn open_read_only(path: &Path) -> Option<rusqlite::Connection> {
    let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let con = rusqlite::Connection::open_with_flags(path, flags).ok()?;
    // The agent may be writing; wait briefly instead of failing.
    con.busy_timeout(std::time::Duration::from_millis(500)).ok()?;
    Some(con)
}

/// Codex keeps threads in `~/.codex/state_N.sqlite`; the newest N is current.
fn codex_db() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("USERPROFILE")?).join(".codex");
    std::fs::read_dir(&dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("state_") && n.ends_with(".sqlite")))
        .max()
}

/// (this thread's tokens, tokens across threads active today). Codex reports one running
/// total per thread, so "today" counts whole threads that were active today.
pub fn codex(thread_id: &str) -> Option<(Option<u64>, u64)> {
    let con = open_read_only(&codex_db()?)?;
    let session = con.query_row("select tokens_used from threads where id = ?1", [thread_id], |r| r.get::<_, i64>(0)).ok().map(|t| t.max(0) as u64);
    let today: i64 = con.query_row("select coalesce(sum(tokens_used), 0) from threads where updated_at_ms >= ?1", [midnight_ms()], |r| r.get(0)).ok()?;
    Some((session, today.max(0) as u64))
}

/// (this session's fresh tokens, fresh tokens across sessions active today) from OpenCode's
/// database; cache reads are left out, as for Claude.
pub fn opencode(session_id: &str) -> Option<(Option<u64>, u64)> {
    let db = PathBuf::from(std::env::var_os("USERPROFILE")?).join(".local").join("share").join("opencode").join("opencode.db");
    let con = open_read_only(&db)?;
    const FRESH: &str = "coalesce(tokens_input, 0) + coalesce(tokens_output, 0) + coalesce(tokens_reasoning, 0) + coalesce(tokens_cache_write, 0)";
    let session = con.query_row(&format!("select {FRESH} from session where id = ?1"), [session_id], |r| r.get::<_, i64>(0)).ok().map(|t| t.max(0) as u64);
    let today: i64 = con.query_row(&format!("select coalesce(sum({FRESH}), 0) from session where time_updated >= ?1"), [midnight_ms()], |r| r.get(0)).ok()?;
    Some((session, today.max(0) as u64))
}

/// 950 → "950", 45_230 → "45.2k", 1_204_000 → "1.2M", 1_486_000_000 → "1.5B".
pub fn format_tokens(n: u64) -> String {
    let (value, unit) = match n {
        0..1_000 => return n.to_string(),
        1_000..1_000_000 => (n as f64 / 1e3, "k"),
        1_000_000..1_000_000_000 => (n as f64 / 1e6, "M"),
        _ => (n as f64 / 1e9, "B"),
    };
    // One decimal below 100 ("45.2k"), none above ("152M").
    let text = if value < 100.0 { format!("{value:.1}") } else { format!("{value:.0}") };
    format!("{}{unit}", text.strip_suffix(".0").unwrap_or(&text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn line(id: &str, request: &str, output: u64) -> String {
        let ts = Local::now().to_rfc3339();
        format!(
            r#"{{"type":"assistant","requestId":"{request}","timestamp":"{ts}","message":{{"id":"{id}","usage":{{"input_tokens":10,"output_tokens":{output},"cache_creation_input_tokens":100,"cache_read_input_tokens":1000}}}}}}"#
        )
    }

    #[test]
    fn counts_each_message_once_and_reads_incrementally() {
        let dir = std::env::temp_dir().join(format!("reui-usage-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.jsonl");
        let mut f = std::fs::File::create(&path).unwrap();
        // Two streaming lines for the same message: only the last counts.
        writeln!(f, "{}", line("msg_1", "req_1", 5)).unwrap();
        writeln!(f, "{}", line("msg_1", "req_1", 50)).unwrap();
        writeln!(f, r#"{{"type":"user","message":{{"content":"hi"}}}}"#).unwrap();
        write!(f, "{{\"partial").unwrap(); // still being written
        f.flush().unwrap();

        let mut ledger = Ledger::default();
        ledger.update(&path);
        assert_eq!(ledger.file_total(&path), Usage { input: 10, output: 50, cache_write: 100, cache_read: 1000 });
        assert_eq!(ledger.file_total(&path).fresh(), 160);

        // Finish the partial line (it's junk) and append a new message.
        writeln!(f, "\"}}").unwrap();
        writeln!(f, "{}", line("msg_2", "req_2", 1)).unwrap();
        f.flush().unwrap();
        ledger.update(&path);
        let both = Usage { input: 20, output: 51, cache_write: 200, cache_read: 2000 };
        assert_eq!(ledger.file_total(&path), both);
        assert_eq!(ledger.today_total(), both);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn token_counts_read_naturally() {
        assert_eq!(format_tokens(950), "950");
        assert_eq!(format_tokens(45_230), "45.2k");
        assert_eq!(format_tokens(12_000), "12k");
        assert_eq!(format_tokens(1_204_000), "1.2M");
        assert_eq!(format_tokens(152_212_190), "152M");
        assert_eq!(format_tokens(1_486_360_655), "1.5B");
    }
}

#[cfg(test)]
mod real_databases {
    /// Reads this machine's Codex and OpenCode databases; run by hand with
    /// `cargo test -p reui-hub -- --ignored real_databases --nocapture`.
    #[test]
    #[ignore]
    fn real_databases() {
        let codex = super::codex("01a13bde-1466-7382-87af-4f0108c0a8b8");
        let opencode = super::opencode("ses_fc9e663edffex2NFQAbtjOHZGR");
        println!("codex: {codex:?}\nopencode: {opencode:?}");
        assert!(codex.is_some() && opencode.is_some(), "both databases should open read-only");
    }
}
