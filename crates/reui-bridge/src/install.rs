//! Adds or removes ReUI's integration with a coding agent.
//!
//! Claude Code and Codex get hook entries in their JSON settings; OpenCode gets a plugin
//! file. Rules: merge, never overwrite (other hooks and settings are untouched and keep
//! their order); keep a backup of a settings file as it was before ReUI ever touched it;
//! remove exactly what we added (handlers whose command is a `reui-bridge` executable, or
//! our own plugin file); write atomically.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

/// (event, matcher, timeout seconds, async, what the bridge is asked to do besides
/// forwarding). Only what needs you blocks the agent: permission requests, and Claude's
/// tools that ask you something (`ask`: the bridge waits for the answer).
type Event = (&'static str, Option<&'static str>, Option<u64>, bool, Option<&'static str>);

/// The bridge's argument for the blocking `PreToolUse` entry: wait for an answer.
pub const ASK: &str = "ask";
/// Claude's tools answered from that entry (agent moments PRD §6).
pub const ASK_MATCHER: &str = "AskUserQuestion|ExitPlanMode";

const CLAUDE_EVENTS: [Event; 15] = [
    ("SessionStart", None, None, true, None),
    ("UserPromptSubmit", None, None, true, None),
    ("PreToolUse", Some("*"), None, true, None),
    // Beside the async `*` one, which also fires for these tools; the hub makes them one.
    ("PreToolUse", Some(ASK_MATCHER), Some(300), false, Some(ASK)),
    ("PostToolUse", Some("*"), None, true, None),
    ("PermissionRequest", Some("*"), Some(120), false, None),
    // MCP forms: the hub answers "no decision" at once until the island can show them
    // (agent moments M4), so Claude's own dialog comes without waiting.
    ("Elicitation", Some("*"), Some(300), false, None),
    ("Notification", None, None, true, None),
    ("PreCompact", None, None, true, None),
    ("PostCompact", None, None, true, None),
    ("SubagentStart", None, None, true, None),
    ("SubagentStop", None, None, true, None),
    ("Stop", None, None, true, None),
    ("StopFailure", None, None, true, None),
    ("SessionEnd", None, None, true, None),
];

/// Codex 0.15x: the same shape, without Claude-only events.
const CODEX_EVENTS: [Event; 11] = [
    ("SessionStart", None, None, true, None),
    ("UserPromptSubmit", None, None, true, None),
    ("PreToolUse", Some("*"), None, true, None),
    ("PostToolUse", Some("*"), None, true, None),
    ("PermissionRequest", Some("*"), Some(120), false, None),
    ("PreCompact", None, None, true, None),
    ("PostCompact", None, None, true, None),
    ("SubagentStart", None, None, true, None),
    ("SubagentStop", None, None, true, None),
    ("Stop", None, None, true, None),
    ("SessionEnd", None, None, true, None),
];

const OPENCODE_PLUGIN: &str = include_str!("opencode-plugin.js");
const OPENCODE_PLUGIN_FILE: &str = "reui.js";

#[derive(Clone, Copy, PartialEq, Debug)]
enum Agent {
    Claude,
    Codex,
    OpenCode,
}

pub fn run(args: &[String]) -> Result<String, String> {
    let install = args[0] == "install";
    let agent = match args.get(1).map(String::as_str) {
        Some("claude") => Agent::Claude,
        Some("codex") => Agent::Codex,
        Some("opencode") => Agent::OpenCode,
        other => return Err(format!("unsupported agent {other:?} (supported: claude, codex, opencode)")),
    };
    let mut scope = "user";
    let mut project: Option<PathBuf> = None;
    let mut rest = args[2..].iter();
    while let Some(flag) = rest.next() {
        match flag.as_str() {
            "--scope" => scope = rest.next().ok_or("--scope needs user, project or local")?,
            "--project" => project = Some(PathBuf::from(rest.next().ok_or("--project needs a directory")?)),
            other => return Err(format!("unknown option {other}")),
        }
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = exe.to_string_lossy().replace('\\', "/");
    let verb = if install { "Installed" } else { "Removed" };

    if agent == Agent::OpenCode {
        let path = opencode_plugin_path(scope, project)?;
        if install {
            write_atomic(&path, &opencode_plugin(&exe))?;
        } else if path.exists() {
            std::fs::remove_file(&path).map_err(|e| format!("can't remove {}: {e}", path.display()))?;
        }
        return Ok(format!("{verb} the ReUI plugin for OpenCode at {} (OpenCode loads it on its next start)", path.display()));
    }

    let path = settings_path(agent, scope, project)?;
    let backup = update(&path, agent, &exe, install)?;
    let mut message = format!("{verb} ReUI hooks for {} in {}", display(agent), path.display());
    if let Some(backup) = backup {
        message += &format!(" (original kept at {})", backup.display());
    }
    if install && agent == Agent::Codex {
        message += "\nCodex skips new hooks until you trust them: open Codex and run /hooks to review and trust ReUI's.";
    }
    Ok(message)
}

fn display(agent: Agent) -> &'static str {
    match agent {
        Agent::Claude => "Claude Code",
        Agent::Codex => "Codex",
        Agent::OpenCode => "OpenCode",
    }
}

fn home() -> Result<PathBuf, String> {
    std::env::var("USERPROFILE").map(PathBuf::from).map_err(|_| "USERPROFILE is not set".to_string())
}

fn settings_path(agent: Agent, scope: &str, project: Option<PathBuf>) -> Result<PathBuf, String> {
    let project = || project.clone().map(Ok).unwrap_or_else(|| std::env::current_dir().map_err(|e| e.to_string()));
    Ok(match (agent, scope) {
        (Agent::Claude, "user") => home()?.join(".claude").join("settings.json"),
        (Agent::Claude, "project") => project()?.join(".claude").join("settings.json"),
        (Agent::Claude, "local") => project()?.join(".claude").join("settings.local.json"),
        (Agent::Codex, "user") => home()?.join(".codex").join("hooks.json"),
        (Agent::Codex, "project") => project()?.join(".codex").join("hooks.json"),
        (_, other) => return Err(format!("unsupported scope {other:?} for {}", display(agent))),
    })
}

fn opencode_plugin_path(scope: &str, project: Option<PathBuf>) -> Result<PathBuf, String> {
    Ok(match scope {
        "user" => home()?.join(".config").join("opencode").join("plugins").join(OPENCODE_PLUGIN_FILE),
        "project" => project.map(Ok).unwrap_or_else(|| std::env::current_dir().map_err(|e| e.to_string()))?.join(".opencode").join("plugins").join(OPENCODE_PLUGIN_FILE),
        other => return Err(format!("unsupported scope {other:?} for OpenCode (user or project)")),
    })
}

/// The plugin with this bridge's path filled in (as a JSON string literal, so any path is safe).
fn opencode_plugin(exe: &str) -> String {
    OPENCODE_PLUGIN.replace("\"__REUI_BRIDGE__\"", &Value::String(exe.to_string()).to_string())
}

/// Merges or removes our hooks in a JSON settings file. Returns the backup path if one
/// exists for this file.
fn update(path: &Path, agent: Agent, exe: &str, install: bool) -> Result<Option<PathBuf>, String> {
    let original = match std::fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("can't read {}: {e}", path.display())),
    };
    let mut root: Value = match &original {
        Some(text) if !text.trim().is_empty() => serde_json::from_str(text).map_err(|e| format!("{} isn't valid JSON ({e}); not touching it", path.display()))?,
        _ => json!({}),
    };
    let settings = root.as_object_mut().ok_or_else(|| format!("{} isn't a JSON object; not touching it", path.display()))?;
    match agent {
        Agent::Claude => merge_hooks(settings, &CLAUDE_EVENTS, |extra| claude_handler(exe, extra), install)?,
        // Codex hooks are a command line (no `args`); quote the path only if it needs it.
        Agent::Codex => merge_hooks(settings, &CODEX_EVENTS, |_| json!({ "type": "command", "command": command_line(exe, "codex") }), install)?,
        Agent::OpenCode => unreachable!("OpenCode uses a plugin file"),
    }

    let backup = path.with_extension(format!("{}.reui-backup", path.extension().and_then(|e| e.to_str()).unwrap_or("json")));
    // Only a file ReUI has never touched is worth keeping as "the original".
    if let Some(text) = original.as_ref().filter(|t| !t.contains("reui-bridge")) {
        if !backup.exists() {
            std::fs::write(&backup, text).map_err(|e| format!("can't write backup {}: {e}", backup.display()))?;
        }
    }
    write_atomic(path, &(serde_json::to_string_pretty(&root).map_err(|e| e.to_string())? + "\n"))?;
    Ok(backup.exists().then_some(backup))
}

fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("can't create {}: {e}", dir.display()))?;
    }
    let temp = path.with_extension("reui-tmp");
    std::fs::write(&temp, text).map_err(|e| format!("can't write {}: {e}", temp.display()))?;
    std::fs::rename(&temp, path).map_err(|e| format!("can't replace {}: {e}", path.display()))
}

/// Claude Code runs the bridge with `args` (no shell, so any path works).
fn claude_handler(exe: &str, extra: Option<&str>) -> Value {
    let args: Vec<&str> = std::iter::once("claude").chain(extra).collect();
    json!({ "type": "command", "command": exe, "args": args })
}

fn command_line(exe: &str, arg: &str) -> String {
    if exe.contains(' ') {
        format!("\"{exe}\" {arg}")
    } else {
        format!("{exe} {arg}")
    }
}

/// Is this handler's program a `reui-bridge` executable? Handles a bare path, a quoted path,
/// and a command line with arguments.
fn is_ours(handler: &Value) -> bool {
    handler.get("command").and_then(Value::as_str).is_some_and(|c| {
        let c = c.trim();
        let program = match c.strip_prefix('"') {
            Some(rest) => rest.split('"').next().unwrap_or(rest),
            None => c.split_whitespace().next().unwrap_or(c),
        };
        let file = program.rsplit(['/', '\\']).next().unwrap_or(program).to_ascii_lowercase();
        file == "reui-bridge.exe" || file == "reui-bridge"
    })
}

fn merge_hooks(settings: &mut Map<String, Value>, events: &[Event], handler: impl Fn(Option<&str>) -> Value, install: bool) -> Result<(), String> {
    let hooks = settings.entry("hooks").or_insert_with(|| json!({}));
    let hooks = hooks.as_object_mut().ok_or("\"hooks\" isn't an object; not touching it")?;

    // Remove our handlers everywhere (idempotent re-install), dropping groups and events
    // that become empty only if we emptied them.
    for groups in hooks.values_mut() {
        let Some(groups) = groups.as_array_mut() else { continue };
        groups.retain_mut(|group| {
            let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) else { return true };
            let before = handlers.len();
            handlers.retain(|h| !is_ours(h));
            !(before > 0 && handlers.is_empty())
        });
    }
    hooks.retain(|_, groups| groups.as_array().is_none_or(|g| !g.is_empty()));

    if install {
        for &(event, matcher, timeout, background, extra) in events {
            let mut handler = handler(extra);
            if let Some(timeout) = timeout {
                handler["timeout"] = json!(timeout);
            }
            if background {
                handler["async"] = json!(true);
            }
            let mut group = Map::new();
            if let Some(matcher) = matcher {
                // "matcher" first, as the agents' own docs write it.
                group.insert("matcher".into(), json!(matcher));
            }
            group.insert("hooks".into(), json!([handler]));
            let groups = hooks.entry(event).or_insert_with(|| json!([]));
            groups.as_array_mut().ok_or_else(|| format!("hooks.{event} isn't an array; not touching it"))?.push(Value::Object(group));
        }
    }
    if hooks.is_empty() {
        settings.remove("hooks");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = "D:/ReUI/target/release/reui-bridge.exe";

    fn claude(settings: &mut Map<String, Value>, install: bool) {
        merge_hooks(settings, &CLAUDE_EVENTS, |extra| claude_handler(EXE, extra), install).unwrap();
    }

    fn codex(settings: &mut Map<String, Value>, install: bool) {
        merge_hooks(settings, &CODEX_EVENTS, |_| json!({ "type": "command", "command": command_line(EXE, "codex") }), install).unwrap();
    }

    #[test]
    fn install_merges_and_uninstall_restores() {
        let original = json!({
            "model": "opus",
            "hooks": {
                "PreToolUse": [{ "matcher": "Bash", "hooks": [{ "type": "command", "command": "my-linter" }] }]
            }
        });
        let mut settings = original.as_object().unwrap().clone();

        claude(&mut settings, true);
        let hooks = settings["hooks"].as_object().unwrap();
        assert_eq!(hooks.len(), CLAUDE_EVENTS.len() - 1, "PreToolUse has two entries");
        // The user's own hook survives next to ours.
        assert_eq!(hooks["PreToolUse"].as_array().unwrap().len(), 3);
        assert_eq!(hooks["PreToolUse"][0]["hooks"][0]["command"], "my-linter");
        let permission = &hooks["PermissionRequest"][0]["hooks"][0];
        assert_eq!(permission["timeout"], 120);
        assert!(permission.get("async").is_none(), "permission requests must block");
        assert_eq!(hooks["Stop"][0]["hooks"][0]["async"], true);
        // Every tool, async; Claude's questions and plans also through a blocking entry
        // that tells the bridge to wait.
        assert_eq!(hooks["PreToolUse"][1], json!({ "matcher": "*", "hooks": [{ "type": "command", "command": EXE, "args": ["claude"], "async": true }] }));
        assert_eq!(hooks["PreToolUse"][2], json!({ "matcher": "AskUserQuestion|ExitPlanMode", "hooks": [{ "type": "command", "command": EXE, "args": ["claude", "ask"], "timeout": 300 }] }));
        assert_eq!(hooks["Elicitation"][0], json!({ "matcher": "*", "hooks": [{ "type": "command", "command": EXE, "args": ["claude"], "timeout": 300 }] }));

        // Re-installing doesn't duplicate.
        claude(&mut settings, true);
        assert_eq!(settings["hooks"]["PreToolUse"].as_array().unwrap().len(), 3);

        claude(&mut settings, false);
        assert_eq!(Value::Object(settings), original);
    }

    #[test]
    fn codex_hooks_sit_beside_other_tools_and_leave_cleanly() {
        // The shape of a real ~/.codex/hooks.json with two other tools' hooks.
        let original = json!({ "hooks": {
            "PreToolUse": [
                { "hooks": [{ "type": "command", "command": "C:/Users/me/dev/paw-mascot/scripts/paw-event.sh" }] },
                { "hooks": [{ "type": "command", "command": "C:/Users/me/.orca/agent-hooks/codex-hook.cmd", "timeout": 10 }] }
            ],
            "Interrupt": [{ "hooks": [{ "type": "command", "command": "C:/Users/me/.orca/agent-hooks/codex-hook.cmd", "timeout": 3 }] }]
        }});
        let mut settings = original.as_object().unwrap().clone();
        codex(&mut settings, true);
        let hooks = settings["hooks"].as_object().unwrap();
        assert_eq!(hooks["PreToolUse"].as_array().unwrap().len(), 3);
        assert_eq!(hooks["PreToolUse"][2]["hooks"][0]["command"], format!("{EXE} codex"));
        assert!(hooks["PreToolUse"][2]["hooks"][0].get("args").is_none(), "Codex has no args field");
        assert_eq!(hooks["PermissionRequest"][0]["hooks"][0]["timeout"], 120);
        // Untouched events of other tools stay as they were.
        assert_eq!(hooks["Interrupt"], original["hooks"]["Interrupt"]);
        codex(&mut settings, false);
        assert_eq!(Value::Object(settings), original);
    }

    #[test]
    fn uninstall_drops_hooks_key_it_created() {
        let mut settings = Map::new();
        claude(&mut settings, true);
        claude(&mut settings, false);
        assert!(settings.is_empty());
    }

    #[test]
    fn recognises_only_bridge_commands() {
        assert!(is_ours(&json!({ "command": "C:\\tools\\reui-bridge.exe" })));
        assert!(is_ours(&json!({ "command": EXE })));
        assert!(is_ours(&json!({ "command": format!("{EXE} codex") })));
        assert!(is_ours(&json!({ "command": "\"C:/Program Files/ReUI/reui-bridge.exe\" codex" })));
        assert!(!is_ours(&json!({ "command": "reui-bridge-helper.exe" })));
        assert!(!is_ours(&json!({ "command": "C:/Users/me/.orca/agent-hooks/codex-hook.cmd" })));
        assert!(!is_ours(&json!({ "command": "my-linter" })));
    }

    #[test]
    fn paths_with_spaces_are_quoted() {
        assert_eq!(command_line("C:/Program Files/ReUI/reui-bridge.exe", "codex"), "\"C:/Program Files/ReUI/reui-bridge.exe\" codex");
        assert_eq!(command_line(EXE, "codex"), format!("{EXE} codex"));
    }

    #[test]
    fn plugin_gets_the_bridge_path_as_a_safe_string() {
        let plugin = opencode_plugin("C:/Users/O'Neil \"x\"/reui-bridge.exe");
        assert!(plugin.contains(r#"const BRIDGE = "C:/Users/O'Neil \"x\"/reui-bridge.exe";"#));
        assert!(!plugin.contains("__REUI_BRIDGE__"));
    }
}
