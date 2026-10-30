//! `reui-bridge activity`: a script's live activity in the island (docs/PRD-reminders-
//! activities.md §5.4).
//!
//! ```text
//! reui-bridge activity <id> --title "Build" [--detail "…"] [--progress 0.42|42%]
//!                          [--state running|done|failed] [--glyph build] [--done-after 5s]
//!                          [--keep] [--source name]
//! reui-bridge activity <id> --end
//! reui-bridge activity --json        one JSON object per line on stdin
//! ```
//!
//! The arguments become one JSON object, sent as the `activities` module's `update:<json>`
//! command, the path `reui-bridge timer` takes. The hub checks everything again (anything
//! on the per-user pipe could send it); the checks here are for a clear message on stderr.
//! Exits 1 when ReUI isn't running or the arguments don't read, so a script can tell.

use std::io::BufRead;

use serde_json::{json, Map, Value};

/// The `activities` module's update command for these arguments (after `activity`), or
/// what's wrong with them. `source` is used when there's no `--source`.
pub fn from_args(args: &[String], source: Option<&str>) -> Result<String, String> {
    let mut fields = Map::new();
    let mut it = args.iter();
    let mut id = None;
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().cloned().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--title" => _ = fields.insert("title".into(), value("--title")?.into()),
            "--detail" => _ = fields.insert("detail".into(), value("--detail")?.into()),
            "--glyph" => _ = fields.insert("glyph".into(), value("--glyph")?.into()),
            "--source" => _ = fields.insert("source".into(), value("--source")?.into()),
            "--state" => {
                let state = value("--state")?;
                if !matches!(state.as_str(), "running" | "done" | "failed") {
                    return Err(format!("--state is running, done or failed, not {state:?}"));
                }
                fields.insert("state".into(), state.into());
            }
            "--progress" => _ = fields.insert("progress".into(), json!(progress(&value("--progress")?)?)),
            "--done-after" => _ = fields.insert("done_after_s".into(), json!(seconds(&value("--done-after")?)?)),
            "--keep" => _ = fields.insert("keep".into(), true.into()),
            "--end" => _ = fields.insert("end".into(), true.into()),
            flag if flag.starts_with("--") => return Err(format!("unknown option {flag}")),
            name if id.is_none() => id = Some(name.to_string()),
            extra => return Err(format!("one id only ({extra:?} is a second)")),
        }
    }
    let id = id.ok_or("an id comes first: reui-bridge activity build --title Build")?;
    fields.insert("id".into(), id.into());
    Ok(command(fields, source))
}

/// A line of `--json` input as the update command, or why it isn't one.
pub fn from_line(line: &str, source: Option<&str>) -> Result<String, String> {
    match serde_json::from_str::<Value>(line) {
        Ok(Value::Object(fields)) if fields.get("id").is_some_and(Value::is_string) => Ok(command(fields, source)),
        Ok(Value::Object(_)) => Err("each object needs an \"id\"".into()),
        _ => Err("each line is one JSON object".into()),
    }
}

fn command(mut fields: Map<String, Value>, source: Option<&str>) -> String {
    if let (false, Some(source)) = (fields.contains_key("source"), source) {
        fields.insert("source".into(), source.into());
    }
    format!("update:{}", Value::Object(fields))
}

/// `0.42`, or `42%`.
fn progress(text: &str) -> Result<f64, String> {
    let (number, scale) = match text.strip_suffix('%') {
        Some(n) => (n, 100.0),
        None => (text, 1.0),
    };
    number.trim().parse::<f64>().ok().filter(|v| v.is_finite()).map(|v| (v / scale).clamp(0.0, 1.0)).ok_or_else(|| format!("--progress is a fraction (0.42) or a percentage (42%), not {text:?}"))
}

/// `5s`, `2m`, `500ms`, or seconds.
fn seconds(text: &str) -> Result<f64, String> {
    let t = text.trim().to_lowercase();
    let (number, per) = if let Some(n) = t.strip_suffix("ms") {
        (n, 0.001)
    } else if let Some(n) = t.strip_suffix('s') {
        (n, 1.0)
    } else if let Some(n) = t.strip_suffix('m') {
        (n, 60.0)
    } else {
        (t.as_str(), 1.0)
    };
    number.parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0).map(|v| v * per).ok_or_else(|| format!("--done-after is a length like 5s or 2m, not {text:?}"))
}

/// `--json`: every line of stdin, one update each, as they come (a script can pipe its
/// progress straight in). Lines that don't read are said on stderr and skipped.
pub fn stream(source: Option<&str>, mut send: impl FnMut(&str) -> Result<(), String>) -> Result<(), String> {
    for line in std::io::stdin().lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        match from_line(&line, source) {
            Ok(action) => send(&action)?,
            Err(e) => eprintln!("reui-bridge: {e}"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &[&str]) -> Vec<String> {
        s.iter().map(|a| a.to_string()).collect()
    }

    fn sent(action: &str) -> Value {
        serde_json::from_str(action.strip_prefix("update:").unwrap()).unwrap()
    }

    #[test]
    fn arguments_become_one_update() {
        let action = from_args(&args(&["build", "--title", "Build", "--detail", "Linking", "--progress", "42%", "--glyph", "build", "--done-after", "5s", "--keep"]), Some("pwsh")).unwrap();
        let v = sent(&action);
        assert_eq!(v["id"], "build");
        assert_eq!((v["title"].as_str(), v["detail"].as_str(), v["glyph"].as_str(), v["source"].as_str()), (Some("Build"), Some("Linking"), Some("build"), Some("pwsh")));
        assert_eq!((v["progress"].as_f64(), v["done_after_s"].as_f64(), v["keep"].as_bool()), (Some(0.42), Some(5.0), Some(true)));
        let end = sent(&from_args(&args(&["build", "--end"]), None).unwrap());
        assert_eq!((end["end"].as_bool(), end.get("source")), (Some(true), None));
        let own = sent(&from_args(&args(&["b", "--source", "nightly"]), Some("pwsh")).unwrap());
        assert_eq!(own["source"], "nightly");
        assert_eq!(sent(&from_args(&args(&["b", "--state", "done"]), None).unwrap())["state"], "done");
    }

    #[test]
    fn mistakes_are_said_plainly() {
        assert!(from_args(&args(&[]), None).unwrap_err().contains("an id comes first"));
        assert!(from_args(&args(&["b", "--title"]), None).unwrap_err().contains("needs a value"));
        assert!(from_args(&args(&["b", "--state", "great"]), None).is_err());
        assert!(from_args(&args(&["b", "--progress", "half"]), None).is_err());
        assert!(from_args(&args(&["b", "--done-after", "soon"]), None).is_err());
        assert!(from_args(&args(&["b", "c"]), None).is_err());
        assert!(from_args(&args(&["b", "--colour", "red"]), None).is_err());
    }

    #[test]
    fn lengths_and_progress_read() {
        assert_eq!((progress("0.5"), progress("150%"), progress("-1")), (Ok(0.5), Ok(1.0), Ok(0.0)));
        assert_eq!((seconds("5s"), seconds("2m"), seconds("500ms"), seconds("3")), (Ok(5.0), Ok(120.0), Ok(0.5), Ok(3.0)));
    }

    #[test]
    fn json_lines_need_an_id() {
        let v = sent(&from_line(r#"{"id":"backup","progress":0.1}"#, Some("python")).unwrap());
        assert_eq!((v["id"].as_str(), v["source"].as_str()), (Some("backup"), Some("python")));
        assert!(from_line(r#"{"progress":0.1}"#, None).is_err());
        assert!(from_line("progress 10", None).is_err());
    }
}
