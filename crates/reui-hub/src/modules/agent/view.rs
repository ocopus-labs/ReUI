//! How a session and a permission request look on the island: the status per phase, the
//! tool described for humans, and the published activities.

use std::path::Path;
use std::time::Duration;

use reui_protocol::{Art, Button, ButtonContent, ButtonRole, CompactView, Element, ExpandedView, Headline, Priority, Row, Tone};
use serde_json::Value;

use super::usage::format_tokens;
use super::{permission_key, session_key, Agentry, Phase, Session};
use crate::hub::{ModuleContext, View};

pub(super) const GLYPH_TERMINAL: char = '\u{E756}';
const GLYPH_EDIT: char = '\u{E70F}';
const GLYPH_READ: char = '\u{E8A5}';
const GLYPH_WEB: char = '\u{E774}';
const GLYPH_SEARCH: char = '\u{E721}';
const GLYPH_TOOL: char = '\u{E90F}';
const GLYPH_DONE: char = '\u{E73E}';
const GLYPH_FAILED: char = '\u{E7BA}';

/// (priority, compact trailing, status line) for a phase. Text is quiet by default; colour
/// only where it's your move.
fn present(phase: &Phase, session: &Session) -> (Priority, Element, String) {
    let quiet = |text: &str| Element::Text { text: text.into(), tone: Tone::Secondary };
    let with_agents = |status: String| match session.subagents {
        0 => status,
        1 => format!("{status} · 1 subagent"),
        n => format!("{status} · {n} subagents"),
    };
    match phase {
        Phase::Thinking => (Priority::Ongoing, quiet("Thinking"), with_agents("Thinking".into())),
        Phase::Tool(tool) => (Priority::Ongoing, quiet(tool), with_agents(format!("Running {tool}"))),
        Phase::Compacting => (Priority::Ongoing, quiet("Compacting"), "Compacting the conversation".into()),
        Phase::WaitingPermission => (Priority::Ongoing, quiet("Waiting"), "Waiting for your permission".into()),
        Phase::NeedsInput => (Priority::Alert, Element::Text { text: "Your turn".into(), tone: Tone::Accent }, "Has a question for you".into()),
        Phase::Done { took } => {
            let mut parts = vec![match took {
                Some(took) => format!("Done in {}", format_duration(*took)),
                None => "Done".to_string(),
            }];
            let usage = session.usage.unwrap_or_default();
            if let Some(tokens) = usage.session.filter(|t| *t > 0) {
                parts.push(format!("{} tokens", format_tokens(tokens)));
            }
            if let Some(today) = usage.today {
                parts.push(format!("{} today", format_tokens(today)));
            }
            (Priority::Alert, Element::Glyph { glyph: GLYPH_DONE, tone: Tone::Secondary }, parts.join(" · "))
        }
        Phase::Failed { reason } => (
            Priority::Alert,
            Element::Glyph { glyph: GLYPH_FAILED, tone: Tone::Negative },
            reason.clone().map(|r| format!("Stopped: {r}")).unwrap_or_else(|| "The turn failed".into()),
        ),
    }
}

pub(super) fn publish_session(ctx: &ModuleContext, me: &Agentry, id: &str, session: &Session) {
    let Some(phase) = &session.phase else { return };
    let (priority, trailing, status) = present(phase, session);
    ctx.publish(
        &session_key(id),
        priority,
        View {
            accent: Some(me.profile.accent),
            audio: None,
            compact: CompactView { leading: me.mark.clone(), trailing },
            minimal: me.mark.clone(),
            expanded: Some(ExpandedView {
                leading: me.mark.clone(),
                eyebrow: Some(format!("{} · {}", me.profile.display, session.project)),
                // What you asked is the most useful thing to read; the project otherwise.
                headline: Headline::Text(session.prompt.clone().unwrap_or_else(|| session.project.clone())),
                detail: Some(status),
                trailing: Element::Empty,
                wide: false,
                tint: false,
                pager: None,
                open: None,
                rows: Vec::new(),
            }),
        },
    );
}

pub(super) const TEST_KEY: &str = "test";

/// The answer to `reui-bridge test`: "Claude Code · Connected", briefly. Not a session.
pub(super) fn publish_test(ctx: &ModuleContext, me: &Agentry) {
    let done = Element::Glyph { glyph: GLYPH_DONE, tone: Tone::Positive };
    ctx.publish(
        TEST_KEY,
        Priority::Brief,
        View {
            accent: Some(me.profile.accent),
            audio: None,
            compact: CompactView { leading: me.mark.clone(), trailing: done.clone() },
            minimal: me.mark.clone(),
            expanded: Some(ExpandedView {
                leading: me.mark.clone(),
                eyebrow: Some(me.profile.display.to_string()),
                headline: Headline::Text("Connected".into()),
                detail: None,
                trailing: done,
                wide: false,
                tint: false,
                pager: None,
                open: None,
                rows: Vec::new(),
            }),
        },
    );
}

pub(super) fn publish_request(ctx: &ModuleContext, me: &Agentry, id: u64, project: &str, tool: &str, input: &Value, can_always: bool) {
    let (glyph, headline, detail) = describe_tool(tool, input);
    let button = |action: &str, label: &str, role| Button { action: action.into(), content: ButtonContent::Text(label.into()), role };
    let mut buttons = vec![button("deny", "Deny", ButtonRole::Destructive)];
    if can_always {
        buttons.push(button("always", "Always", ButtonRole::Secondary));
    }
    buttons.push(button("allow", "Allow", ButtonRole::Primary));
    ctx.publish(
        &permission_key(id),
        Priority::Decision,
        View {
            accent: Some(me.profile.accent),
            audio: None,
            compact: CompactView { leading: me.mark.clone(), trailing: Element::Text { text: "Allow?".into(), tone: Tone::Accent } },
            minimal: me.mark.clone(),
            expanded: Some(ExpandedView {
                leading: Element::Art(Art::Glyph(glyph)),
                eyebrow: Some(format!("{} · {project}", me.profile.display)),
                headline,
                detail: Some(detail),
                trailing: Element::Empty,
                wide: false,
                tint: false,
                pager: None,
                open: None,
                rows: vec![Row::Buttons(buttons)],
            }),
        },
    );
}

/// What the request is about, readable at a glance. Knows Claude Code's, Codex's and
/// OpenCode's tool names (OpenCode's are lowercase and use `filePath`).
fn describe_tool(tool: &str, input: &Value) -> (char, Headline, String) {
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
    match tool.to_ascii_lowercase().as_str() {
        "bash" | "powershell" | "shell" | "local_shell" | "exec_command" => {
            let command = text("command").lines().next().unwrap_or_default().to_string();
            (GLYPH_TERMINAL, Headline::Code(command), description().unwrap_or_else(|| "wants to run a command".into()))
        }
        "edit" | "multiedit" | "notebookedit" | "patch" => {
            let p = path();
            (GLYPH_EDIT, Headline::Text(file(&p)), format!("wants to edit a file in {}", folder(&p)))
        }
        "apply_patch" => {
            // The patch names its files: "*** Update File: path".
            let patch = text("input") + &text("patch") + &text("command");
            let target = patch.lines().find_map(|l| l.split_once(" File: ").map(|(_, p)| p.trim().to_string())).unwrap_or_default();
            (GLYPH_EDIT, Headline::Text(if target.is_empty() { "a patch".into() } else { file(&target) }), "wants to apply a patch".into())
        }
        "write" => {
            let p = path();
            (GLYPH_EDIT, Headline::Text(file(&p)), format!("wants to write a file in {}", folder(&p)))
        }
        "read" => {
            let p = path();
            (GLYPH_READ, Headline::Text(file(&p)), format!("wants to read a file in {}", folder(&p)))
        }
        "webfetch" => {
            let url = text("url");
            let host = url.split("://").nth(1).and_then(|r| r.split('/').next()).unwrap_or(&url).to_string();
            (GLYPH_WEB, Headline::Text(host), "wants to fetch a web page".into())
        }
        "websearch" => (GLYPH_SEARCH, Headline::Text(text("query")), "wants to search the web".into()),
        _ => {
            let name = short_tool_name(tool);
            let detail = match tool.strip_prefix("mcp__").and_then(|r| r.split("__").next()) {
                Some(server) => format!("wants to use {server} (MCP)"),
                None => description().unwrap_or_else(|| "wants to use a tool".into()),
            };
            (GLYPH_TOOL, Headline::Text(name), detail)
        }
    }
}

/// `mcp__figma__get_screenshot` → `get_screenshot`.
pub(super) fn short_tool_name(tool: &str) -> String {
    tool.rsplit("__").next().unwrap_or(tool).to_string()
}

fn format_duration(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m {}s", s / 60, s % 60),
        _ => format!("{}h {}m", s / 3600, s % 3600 / 60),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn tools_are_described_for_humans() {
        let (_, headline, detail) = describe_tool("Bash", &json!({ "command": "cargo build --release\nmore", "description": "Build release" }));
        assert_eq!(headline, Headline::Code("cargo build --release".into()));
        assert_eq!(detail, "Build release");

        let (_, headline, detail) = describe_tool("Edit", &json!({ "file_path": "D:\\ReUI\\crates\\reui-hub\\src\\main.rs" }));
        assert_eq!(headline, Headline::Text("main.rs".into()));
        assert_eq!(detail, "wants to edit a file in src");

        let (_, headline, _) = describe_tool("WebFetch", &json!({ "url": "https://code.claude.com/docs/en/hooks" }));
        assert_eq!(headline, Headline::Text("code.claude.com".into()));

        let (_, headline, detail) = describe_tool("mcp__figma__get_screenshot", &json!({}));
        assert_eq!(headline, Headline::Text("get_screenshot".into()));
        assert_eq!(detail, "wants to use figma (MCP)");
    }

    #[test]
    fn codex_and_opencode_tools_read_the_same_way() {
        // Codex: argv-style command, apply_patch with the file in the patch.
        let (_, headline, _) = describe_tool("shell", &json!({ "command": ["cargo", "test"] }));
        assert_eq!(headline, Headline::Code("cargo test".into()));
        let (_, headline, _) = describe_tool("apply_patch", &json!({ "input": "*** Begin Patch\n*** Update File: src/pill.rs\n@@" }));
        assert_eq!(headline, Headline::Text("pill.rs".into()));
        // OpenCode: lowercase tools, camelCase paths, the plugin passes its title as description.
        let (_, headline, detail) = describe_tool("bash", &json!({ "command": "npm run build", "description": "Build the site" }));
        assert_eq!((headline, detail.as_str()), (Headline::Code("npm run build".into()), "Build the site"));
        let (_, headline, _) = describe_tool("edit", &json!({ "filePath": "D:/site/src/app.ts" }));
        assert_eq!(headline, Headline::Text("app.ts".into()));
    }

    #[test]
    fn durations_read_naturally() {
        assert_eq!(format_duration(Duration::from_secs(45)), "45s");
        assert_eq!(format_duration(Duration::from_secs(134)), "2m 14s");
        assert_eq!(format_duration(Duration::from_secs(3780)), "1h 3m");
    }
}
