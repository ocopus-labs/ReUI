//! What a permission request is about, for humans: a glyph, the hero (the command, the
//! file, the host), a detail of facts, and a body for what the hero cuts off (the whole
//! command, an edit's changed lines). Knows Claude Code's, Codex's and OpenCode's tool
//! names (OpenCode's are lowercase and use `filePath`; Codex passes argv arrays). Pure.

use std::path::Path;

use reui_protocol::{BlockKind, Headline};
use serde_json::Value;

pub(super) const GLYPH_TERMINAL: char = '\u{E756}';
const GLYPH_EDIT: char = '\u{E70F}';
const GLYPH_READ: char = '\u{E8A5}';
const GLYPH_WEB: char = '\u{E774}';
const GLYPH_SEARCH: char = '\u{E721}';
const GLYPH_TOOL: char = '\u{E90F}';

/// A command longer than this is shown whole in the body too; the hero chip will have
/// cut it. (A character count, not a width: the island decides widths.)
const LONG_COMMAND: usize = 48;
/// The body never carries more lines than this; the block scrolls through them.
const BODY_LINES: usize = 60;

pub(super) struct Described {
    pub glyph: char,
    pub headline: Headline,
    pub detail: Option<String>,
    pub body: Option<(String, BlockKind)>,
}

pub(super) fn describe_tool(tool: &str, input: &Value) -> Described {
    let text = |k: &str| match input.get(k) {
        Some(Value::String(s)) => s.clone(),
        // Codex passes some commands as an argv array.
        Some(Value::Array(parts)) => parts.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "),
        _ => String::new(),
    };
    let path = || [text("file_path"), text("filePath"), text("path")].into_iter().find(|p| !p.is_empty()).unwrap_or_default();
    let file = |p: &str| Path::new(p).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string());
    let folder = |p: &str| Path::new(p).parent().and_then(Path::file_name).map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    let description = || Some(text("description")).filter(|d| !d.is_empty());
    let facts = |parts: &[&str]| Some(parts.iter().filter(|p| !p.is_empty()).copied().collect::<Vec<_>>().join(" · ")).filter(|s| !s.is_empty());
    let d = |glyph, headline, detail| Described { glyph, headline, detail, body: None };
    match tool.to_ascii_lowercase().as_str() {
        "bash" | "powershell" | "shell" | "local_shell" | "exec_command" => {
            let command = text("command");
            let first = command.lines().map(str::trim_end).find(|l| !l.trim().is_empty()).unwrap_or_default().to_string();
            let cut = command.trim().lines().count() > 1 || first.chars().count() > LONG_COMMAND;
            let body = cut.then(|| (first_lines(command.trim(), BODY_LINES), BlockKind::Code));
            Described { body, ..d(GLYPH_TERMINAL, Headline::Code(first), description()) }
        }
        "edit" | "multiedit" => {
            let p = path();
            let edits: Vec<(String, String)> = match input.get("edits").and_then(Value::as_array) {
                Some(edits) => edits.iter().map(|e| (str_of(e, "old_string"), str_of(e, "new_string"))).collect(),
                None => vec![(text("old_string") + &text("oldString"), text("new_string") + &text("newString"))],
            };
            let (diff, added, removed) = changed_lines(&edits);
            let counts = if added + removed > 0 { format!("+{added} \u{2212}{removed}") } else { String::new() };
            let body = (!diff.is_empty()).then(|| (first_lines(&diff, BODY_LINES), BlockKind::Diff));
            Described { body, ..d(GLYPH_EDIT, Headline::Text(file(&p)), facts(&["Edit", &folder(&p), &counts])) }
        }
        "notebookedit" | "patch" => {
            let p = path();
            d(GLYPH_EDIT, Headline::Text(file(&p)), facts(&["Edit", &folder(&p)]))
        }
        "apply_patch" => {
            // The patch names its files: "*** Update File: path".
            let patch = text("input") + &text("patch") + &text("command");
            let files: Vec<String> = patch.lines().filter_map(|l| l.split_once(" File: ").map(|(_, p)| file(p.trim()))).collect();
            let headline = Headline::Text(files.first().cloned().unwrap_or_else(|| "Patch".into()));
            let more = match files.len() {
                0 | 1 => String::new(),
                n => format!("{n} files"),
            };
            let body = (!patch.trim().is_empty()).then(|| (first_lines(patch.trim(), BODY_LINES), BlockKind::Diff));
            Described { body, ..d(GLYPH_EDIT, headline, facts(&["Patch", &more])) }
        }
        "write" => {
            let p = path();
            let content = text("content");
            let lines = match content.lines().count() {
                0 => String::new(),
                1 => "1 line".into(),
                n => format!("{n} lines"),
            };
            let body = (!content.trim().is_empty()).then(|| (first_lines(&content, BODY_LINES), BlockKind::Code));
            Described { body, ..d(GLYPH_EDIT, Headline::Text(file(&p)), facts(&["Write", &folder(&p), &lines])) }
        }
        "read" => {
            let p = path();
            d(GLYPH_READ, Headline::Text(file(&p)), facts(&["Read", &folder(&p)]))
        }
        "webfetch" => {
            let url = text("url");
            let host = url.split("://").nth(1).and_then(|r| r.split('/').next()).unwrap_or(&url).to_string();
            d(GLYPH_WEB, Headline::Text(host), Some("Fetch".into()))
        }
        "websearch" => d(GLYPH_SEARCH, Headline::Text(text("query")), Some("Web search".into())),
        _ => {
            let name = short_tool_name(tool);
            let detail = match tool.strip_prefix("mcp__").and_then(|r| r.split("__").next()) {
                Some(server) => Some(format!("{server} (MCP)")),
                None => description(),
            };
            d(GLYPH_TOOL, Headline::Text(name), detail)
        }
    }
}

fn str_of(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or_default().to_string()
}

/// The lines an edit changes, as `-` / `+` lines (the lines both sides share at the start
/// and end left out), with how many were added and removed.
fn changed_lines(edits: &[(String, String)]) -> (String, usize, usize) {
    let mut out = Vec::new();
    let (mut added, mut removed) = (0, 0);
    for (old, new) in edits {
        let (old, new): (Vec<&str>, Vec<&str>) = (old.lines().collect(), new.lines().collect());
        let head = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
        let tail = old[head..].iter().rev().zip(new[head..].iter().rev()).take_while(|(a, b)| a == b).count();
        let (gone, came) = (&old[head..old.len() - tail], &new[head..new.len() - tail]);
        removed += gone.len();
        added += came.len();
        out.extend(gone.iter().map(|l| format!("- {l}")));
        out.extend(came.iter().map(|l| format!("+ {l}")));
    }
    (out.join("\n"), added, removed)
}

fn first_lines(text: &str, n: usize) -> String {
    text.lines().take(n).collect::<Vec<_>>().join("\n")
}

/// `mcp__figma__get_screenshot` → `get_screenshot`.
pub(super) fn short_tool_name(tool: &str) -> String {
    tool.rsplit("__").next().unwrap_or(tool).to_string()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn tools_are_described_for_humans() {
        let bash = describe_tool("Bash", &json!({ "command": "cargo build --release", "description": "Build release" }));
        assert_eq!((bash.headline, bash.detail.as_deref(), bash.body), (Headline::Code("cargo build --release".into()), Some("Build release"), None));

        let edit = describe_tool("Edit", &json!({ "file_path": "D:\\ReUI\\crates\\reui-hub\\src\\main.rs", "old_string": "a\nb\nc", "new_string": "a\nB\nb2\nc" }));
        assert_eq!(edit.headline, Headline::Text("main.rs".into()));
        assert_eq!(edit.detail.as_deref(), Some("Edit · src · +2 \u{2212}1"));
        assert_eq!(edit.body, Some(("- b\n+ B\n+ b2".into(), BlockKind::Diff)));

        let fetch = describe_tool("WebFetch", &json!({ "url": "https://code.claude.com/docs/en/hooks" }));
        assert_eq!(fetch.headline, Headline::Text("code.claude.com".into()));

        let mcp = describe_tool("mcp__figma__get_screenshot", &json!({}));
        assert_eq!((mcp.headline, mcp.detail.as_deref()), (Headline::Text("get_screenshot".into()), Some("figma (MCP)")));
    }

    #[test]
    fn long_and_multi_line_commands_get_the_whole_command_as_a_body() {
        let script = describe_tool("Bash", &json!({ "command": "set -e\ncargo test\ncargo build --release" }));
        assert_eq!(script.headline, Headline::Code("set -e".into()));
        assert_eq!(script.body, Some(("set -e\ncargo test\ncargo build --release".into(), BlockKind::Code)));
        let long = describe_tool("Bash", &json!({ "command": "git log --oneline --decorate --graph --all -- crates/reui-hub" }));
        assert!(long.body.is_some());
    }

    #[test]
    fn codex_and_opencode_tools_read_the_same_way() {
        // Codex: argv-style command, apply_patch with the file in the patch.
        let shell = describe_tool("shell", &json!({ "command": ["cargo", "test"] }));
        assert_eq!(shell.headline, Headline::Code("cargo test".into()));
        let patch = describe_tool("apply_patch", &json!({ "input": "*** Begin Patch\n*** Update File: src/pill.rs\n@@\n-old\n+new" }));
        assert_eq!(patch.headline, Headline::Text("pill.rs".into()));
        assert_eq!(patch.body.map(|b| b.1), Some(BlockKind::Diff));
        // OpenCode: lowercase tools, camelCase paths, the plugin passes its title as description.
        let bash = describe_tool("bash", &json!({ "command": "npm run build", "description": "Build the site" }));
        assert_eq!((bash.headline, bash.detail.as_deref()), (Headline::Code("npm run build".into()), Some("Build the site")));
        let edit = describe_tool("edit", &json!({ "filePath": "D:/site/src/app.ts", "oldString": "x", "newString": "y" }));
        assert_eq!((edit.headline, edit.detail.as_deref()), (Headline::Text("app.ts".into()), Some("Edit · src · +1 \u{2212}1")));
    }

    #[test]
    fn multi_edits_add_up() {
        let (diff, added, removed) = changed_lines(&[("a".into(), "b".into()), ("x\ny".into(), "x".into())]);
        assert_eq!((diff.as_str(), added, removed), ("- a\n+ b\n- y", 1, 2));
    }
}
