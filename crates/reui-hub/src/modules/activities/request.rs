//! What a script asks for, read and checked in the hub (docs/PRD-reminders-activities.md
//! §5.4). `reui-bridge activity` sends `update:<json>`; anything else that reaches the
//! per-user pipe could send it too, so nothing here trusts the bridge's own checks: ids are
//! a short safe alphabet, text is capped and stripped of control characters, numbers are
//! clamped. Only text and a number come through; nothing is opened, run or read from disk.

use serde_json::Value;

pub const MAX_ID: usize = 40;
pub const MAX_TITLE: usize = 40;
pub const MAX_DETAIL: usize = 80;
const MAX_SOURCE: usize = 32;
/// The longest a finished activity may stay.
pub const MAX_LINGER_MS: u64 = 600_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Running,
    Done,
    Failed,
}

/// A named glyph; the view picks the icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    Script,
    Build,
    Test,
    Download,
    Upload,
    Sync,
    Backup,
    Package,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Update {
    pub id: String,
    pub source: String,
    /// `None` keeps what an earlier update said.
    pub title: Option<String>,
    pub detail: Option<String>,
    /// 0..=1; `None` keeps the earlier one (there's no taking progress back).
    pub progress: Option<f32>,
    pub phase: Phase,
    pub glyph: Option<Glyph>,
    /// How long it stays once done or failed.
    pub linger_ms: Option<u64>,
    /// Never ends for want of updates.
    pub keep: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Update(Update),
    End { id: String },
}

/// Reads a command's JSON, or says why it isn't one.
pub fn read(json: &str) -> Result<Request, &'static str> {
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(json) else { return Err("not a JSON object") };
    let text = |key: &str| map.get(key).and_then(Value::as_str);
    let id = text("id").and_then(|id| name(id, MAX_ID)).ok_or("the id is missing or not a-z 0-9 - _ .")?;
    if map.get("end").and_then(Value::as_bool) == Some(true) {
        return Ok(Request::End { id });
    }
    let phase = match text("state").unwrap_or("running").to_lowercase().as_str() {
        "running" | "run" | "" => Phase::Running,
        "done" | "ok" | "success" => Phase::Done,
        "failed" | "fail" | "error" => Phase::Failed,
        _ => return Err("the state isn't running, done or failed"),
    };
    let progress = map.get("progress").and_then(Value::as_f64).filter(|p| p.is_finite()).map(|p| p.clamp(0.0, 1.0) as f32);
    let linger_ms = map.get("done_after_s").and_then(Value::as_f64).filter(|s| s.is_finite() && *s >= 0.0).map(|s| ((s * 1000.0) as u64).min(MAX_LINGER_MS));
    Ok(Request::Update(Update {
        id,
        source: text("source").and_then(|s| name(s, MAX_SOURCE)).unwrap_or_else(|| "script".into()),
        title: text("title").and_then(|t| clean(t, MAX_TITLE)),
        detail: text("detail").and_then(|t| clean(t, MAX_DETAIL)),
        progress,
        phase,
        glyph: text("glyph").map(glyph),
        linger_ms,
        keep: map.get("keep").and_then(Value::as_bool).unwrap_or(false),
    }))
}

/// An id or source: lower-cased, `a-z 0-9 - _ .`, not empty, at most `max` long.
fn name(raw: &str, max: usize) -> Option<String> {
    let lower = raw.trim().to_lowercase();
    let ok = !lower.is_empty() && lower.len() <= max && lower.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_' | b'.'));
    ok.then_some(lower)
}

/// Text to show: control characters gone, whitespace runs made one space, cut at `max`
/// characters with an ellipsis. `None` when nothing's left.
pub fn clean(raw: &str, max: usize) -> Option<String> {
    let spaced: String = raw.chars().map(|c| if c.is_control() || c.is_whitespace() { ' ' } else { c }).collect();
    let words: Vec<&str> = spaced.split(' ').filter(|w| !w.is_empty()).collect();
    let joined = words.join(" ");
    if joined.is_empty() {
        return None;
    }
    if joined.chars().count() <= max {
        return Some(joined);
    }
    let cut: String = joined.chars().take(max - 1).collect();
    Some(format!("{}…", cut.trim_end()))
}

/// Unknown names are the plain script glyph.
fn glyph(name: &str) -> Glyph {
    match name.trim().to_lowercase().as_str() {
        "build" | "compile" => Glyph::Build,
        "test" | "tests" | "check" => Glyph::Test,
        "download" => Glyph::Download,
        "upload" | "deploy" | "publish" => Glyph::Upload,
        "sync" => Glyph::Sync,
        "backup" | "drive" => Glyph::Backup,
        "package" | "install" | "update" => Glyph::Package,
        _ => Glyph::Script,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(json: &str) -> Update {
        match read(json) {
            Ok(Request::Update(u)) => u,
            other => panic!("{json}: {other:?}"),
        }
    }

    #[test]
    fn a_full_update_reads() {
        let u = update(r#"{"id":"Build","source":"pwsh","title":"Build","detail":"Compiling reui-hub","progress":0.42,"state":"running","glyph":"build","done_after_s":5,"keep":true}"#);
        assert_eq!(u.id, "build");
        assert_eq!(u.source, "pwsh");
        assert_eq!((u.title.as_deref(), u.detail.as_deref()), (Some("Build"), Some("Compiling reui-hub")));
        assert_eq!((u.progress, u.phase, u.glyph, u.linger_ms, u.keep), (Some(0.42), Phase::Running, Some(Glyph::Build), Some(5000), true));
        assert_eq!(read(r#"{"id":"build","end":true}"#), Ok(Request::End { id: "build".into() }));
    }

    #[test]
    fn only_the_id_is_needed() {
        let u = update(r#"{"id":"backup-1"}"#);
        assert_eq!((u.source.as_str(), u.title, u.progress, u.phase, u.glyph, u.keep), ("script", None, None, Phase::Running, None, false));
        assert_eq!(update(r#"{"id":"x","state":"DONE"}"#).phase, Phase::Done);
        assert_eq!(update(r#"{"id":"x","state":"error"}"#).phase, Phase::Failed);
        assert_eq!(update(r#"{"id":"x","glyph":"rocket"}"#).glyph, Some(Glyph::Script));
    }

    #[test]
    fn what_cant_be_trusted_is_refused_or_tamed() {
        assert!(read("not json").is_err());
        assert!(read("[1]").is_err());
        assert!(read(r#"{"title":"no id"}"#).is_err());
        assert!(read(r#"{"id":"../../etc"}"#).is_err());
        assert!(read(r#"{"id":"has space"}"#).is_err());
        assert!(read(&format!(r#"{{"id":"{}"}}"#, "a".repeat(41))).is_err());
        assert!(read(r#"{"id":"x","state":"exploded"}"#).is_err());
        assert_eq!(update(r#"{"id":"x","progress":7}"#).progress, Some(1.0));
        assert_eq!(update(r#"{"id":"x","progress":-1}"#).progress, Some(0.0));
        assert_eq!(update(r#"{"id":"x","progress":"half"}"#).progress, None);
        assert_eq!(update(r#"{"id":"x","done_after_s":100000}"#).linger_ms, Some(MAX_LINGER_MS));
        assert_eq!(update(r#"{"id":"x","done_after_s":-3}"#).linger_ms, None);
        assert_eq!(update(r#"{"id":"x","source":"C:\\evil"}"#).source, "script");
        assert_eq!(update("{\"id\":\"x\",\"title\":\"\\u0007\\u001b[31m\"}").title.as_deref(), Some("[31m"));
        assert_eq!(update("{\"id\":\"x\",\"title\":\"\\u0000\\n\"}").title, None);
    }

    #[test]
    fn text_is_cleaned_and_capped() {
        assert_eq!(clean("  Build\n\tstep  2 ", 40).as_deref(), Some("Build step 2"));
        assert_eq!(clean(&"x".repeat(100), 40).unwrap().chars().count(), 40);
        assert!(clean(&"x".repeat(100), 40).unwrap().ends_with('…'));
        assert_eq!(clean("\u{0}\u{1}", 40), None);
    }
}
