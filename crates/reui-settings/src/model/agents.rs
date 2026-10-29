//! Coding agents: whether ReUI is connected to each (read the same way the bridge's
//! installer finds its own entries, `reui-bridge/src/install.rs`), whether the agent is on
//! this PC at all, when its last hook event reached ReUI (`agents.json`, written by the
//! hub's agent module), and connecting, disconnecting or sending a test event by running
//! the bridge.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use crate::paths::Paths;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agent {
    Claude,
    Codex,
    OpenCode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Connected,
    /// Connected by an older build: Claude's questions and plans can't be answered from
    /// the island until the hooks are installed again (agent moments E-A13).
    Outdated,
    NotConnected,
    /// The file is there but isn't JSON: the installer won't touch it either.
    Unreadable,
}

impl Agent {
    pub const ALL: [Agent; 3] = [Agent::Claude, Agent::Codex, Agent::OpenCode];

    pub fn name(self) -> &'static str {
        match self {
            Agent::Claude => "Claude Code",
            Agent::Codex => "Codex",
            Agent::OpenCode => "OpenCode",
        }
    }

    /// The installer's argument.
    pub fn arg(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
            Agent::OpenCode => "opencode",
        }
    }

    /// The file Connect changes (user scope, as the installer's default).
    pub fn file(self, paths: &Paths) -> PathBuf {
        match self {
            Agent::Claude => paths.home.join(".claude").join("settings.json"),
            Agent::Codex => paths.home.join(".codex").join("hooks.json"),
            Agent::OpenCode => paths.home.join(".config").join("opencode").join("plugins").join("reui.js"),
        }
    }

    /// Folders the agent keeps under the user's home once it has run here.
    fn homes(self, paths: &Paths) -> Vec<PathBuf> {
        match self {
            Agent::Claude => vec![paths.home.join(".claude")],
            Agent::Codex => vec![paths.home.join(".codex")],
            Agent::OpenCode => vec![paths.home.join(".config").join("opencode"), paths.home.join(".local").join("share").join("opencode")],
        }
    }

    /// Program names on `PATH` (npm installs a `.cmd`).
    fn programs(self) -> &'static [&'static str] {
        match self {
            Agent::Claude => &["claude.exe", "claude.cmd"],
            Agent::Codex => &["codex.exe", "codex.cmd"],
            Agent::OpenCode => &["opencode.exe", "opencode.cmd"],
        }
    }

    /// Whether the agent is on this PC: its folder under the home, or its program on
    /// `PATH` (not looked up in the sandbox, which has its own home).
    pub fn installed(self, paths: &Paths) -> bool {
        if self.homes(paths).iter().any(|d| d.is_dir()) {
            return true;
        }
        !paths.sandbox && std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|dir| self.programs().iter().any(|p| dir.join(p).is_file())))
    }

    /// When its last hook event reached ReUI (unix ms).
    pub fn last_event(self, paths: &Paths) -> Option<u64> {
        let text = std::fs::read_to_string(paths.local.join(STATUS_FILE)).ok()?;
        last_event_in(&serde_json::from_str(&text).ok()?, self.arg())
    }

    pub fn status(self, paths: &Paths) -> Status {
        let file = self.file(paths);
        if self == Agent::OpenCode {
            return if file.exists() { Status::Connected } else { Status::NotConnected };
        }
        match std::fs::read_to_string(&file) {
            Ok(text) if text.trim().is_empty() => Status::NotConnected,
            Ok(text) => match serde_json::from_str::<Value>(&text) {
                Ok(root) if has_our_hooks(&root) && self == Agent::Claude && !has_ask_hook(&root) => Status::Outdated,
                Ok(root) if has_our_hooks(&root) => Status::Connected,
                Ok(Value::Object(_)) => Status::NotConnected,
                _ => Status::Unreadable,
            },
            Err(_) => Status::NotConnected,
        }
    }
}

/// The hub's agent module notes each agent's last hook event here (`%LOCALAPPDATA%\ReUI`).
pub const STATUS_FILE: &str = "agents.json";

/// `{ "claude": { "last_event_ms": 1761600000000 } }` → the agent's time.
pub fn last_event_in(root: &Value, agent: &str) -> Option<u64> {
    root.get(agent)?.get("last_event_ms")?.as_u64()
}

/// "just now", "2 min ago", "3 h ago", "1 day ago".
pub fn ago(now_ms: u64, then_ms: u64) -> String {
    let secs = now_ms.saturating_sub(then_ms) / 1000;
    match secs {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", secs / 60),
        3600..86_400 => format!("{} h ago", secs / 3600),
        86_400..172_800 => "1 day ago".into(),
        _ => format!("{} days ago", secs / 86_400),
    }
}

/// What a row says about an agent: connection first, then whether it has been heard from.
pub fn status_line(status: Status, installed: bool, last_event: Option<u64>, now_ms: u64) -> String {
    match (status, last_event) {
        (Status::Connected | Status::Outdated, Some(t)) => format!("Connected · last event {}", ago(now_ms, t)),
        (Status::Connected | Status::Outdated, None) => "Connected · no events yet".into(),
        (_, _) if !installed => "Agent not found on this PC".into(),
        _ => "Not connected".into(),
    }
}

/// Any hook handler whose command is a `reui-bridge` executable (the installer's `is_ours`).
pub fn has_our_hooks(root: &Value) -> bool {
    let Some(hooks) = root.get("hooks").and_then(Value::as_object) else { return false };
    hooks.values().filter_map(Value::as_array).flatten().filter_map(|group| group.get("hooks").and_then(Value::as_array)).flatten().any(is_ours)
}

/// Claude's blocking `PreToolUse` entry for its questions and plans (the installer's
/// `ASK_MATCHER`, a handler of ours that isn't async), which builds before agent moments
/// M2 didn't install.
pub fn has_ask_hook(root: &Value) -> bool {
    let Some(groups) = root.get("hooks").and_then(|h| h.get("PreToolUse")).and_then(Value::as_array) else { return false };
    groups.iter().any(|g| {
        let asks = g.get("matcher").and_then(Value::as_str).is_some_and(|m| m.contains("AskUserQuestion"));
        asks && g.get("hooks").and_then(Value::as_array).is_some_and(|h| h.iter().any(|h| is_ours(h) && h.get("async").and_then(Value::as_bool) != Some(true)))
    })
}

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

/// The bridge to run: the installed copy (agents keep running it from there, and the hub
/// keeps it up to date), put in place from next to this exe the first time.
pub fn bridge(paths: &Paths) -> Option<PathBuf> {
    let installed = paths.installed_bridge();
    if installed.exists() {
        return Some(installed);
    }
    let shipped = paths.exe_dir.join("reui-bridge.exe");
    if !shipped.exists() {
        return None;
    }
    std::fs::create_dir_all(installed.parent()?).ok()?;
    std::fs::copy(&shipped, &installed).ok()?;
    Some(installed)
}

/// Runs `reui-bridge install|uninstall <agent>`; the installer's message, or its error.
pub fn run(bridge: &Path, agent: Agent, connect: bool, paths: &Paths) -> Result<String, String> {
    bridge_command(bridge, &[if connect { "install" } else { "uninstall" }, agent.arg()], paths)
}

/// Runs `reui-bridge test <agent>`: a synthetic event through the normal hook path, which
/// the island answers with a short "connected" moment.
pub fn send_test(bridge: &Path, agent: Agent, paths: &Paths) -> Result<String, String> {
    bridge_command(bridge, &["test", agent.arg()], paths)
}

fn bridge_command(bridge: &Path, args: &[&str], paths: &Paths) -> Result<String, String> {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = Command::new(bridge);
    command.args(args).creation_flags(CREATE_NO_WINDOW);
    if paths.sandbox {
        // `REUI_CONFIG_DIR` is inherited too: a test event goes to a sandboxed hub's own
        // pipe, never the real ReUI's.
        command.env("USERPROFILE", &paths.home);
    }
    let output = command.output().map_err(|e| format!("can't run {}: {e}", bridge.display()))?;
    let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).trim().to_string();
    if output.status.success() {
        Ok(text(&output.stdout))
    } else {
        let err = text(&output.stderr);
        Err(err.strip_prefix("reui-bridge: ").unwrap_or(&err).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn finds_only_bridge_hooks() {
        let ours = json!({ "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "C:/Users/me/AppData/Local/ReUI/bin/reui-bridge.exe", "args": ["claude"] }] }] } });
        let codex = json!({ "hooks": { "Stop": [{ "hooks": [{ "command": "\"C:/Program Files/ReUI/reui-bridge.exe\" codex" }] }] } });
        let theirs = json!({ "hooks": { "PreToolUse": [{ "matcher": "Bash", "hooks": [{ "command": "my-linter" }] }] }, "model": "opus" });
        assert!(has_our_hooks(&ours) && has_our_hooks(&codex));
        assert!(!has_our_hooks(&theirs));
        assert!(!has_our_hooks(&json!({ "hooks": "nope" })));
        assert!(!has_our_hooks(&json!({ "hooks": { "Stop": [{ "hooks": [{ "command": "reui-bridge-helper.exe" }] }] } })));
    }

    #[test]
    fn the_blocking_question_entry_is_told_apart_from_the_async_one() {
        let entry = |matcher: &str, extra: Value| json!({ "hooks": { "PreToolUse": [{ "matcher": matcher, "hooks": [extra] }] } });
        assert!(has_ask_hook(&entry("AskUserQuestion|ExitPlanMode", json!({ "command": "reui-bridge.exe", "args": ["claude", "ask"], "timeout": 300 }))));
        assert!(!has_ask_hook(&entry("*", json!({ "command": "reui-bridge.exe", "args": ["claude"], "async": true }))));
        assert!(!has_ask_hook(&entry("AskUserQuestion", json!({ "command": "reui-bridge.exe", "async": true }))));
        assert!(!has_ask_hook(&entry("AskUserQuestion", json!({ "command": "my-hook.exe" }))));
    }

    #[test]
    fn status_reads_the_agents_files() {
        let dir = crate::model::doc::tests::temp_dir("agents");
        let paths = Paths { config: dir.join("c"), local: dir.join("l"), home: dir.clone(), exe_dir: dir.clone(), sandbox: true };
        assert_eq!(Agent::Claude.status(&paths), Status::NotConnected);
        let file = Agent::Claude.file(&paths);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, r#"{ "hooks": { "Stop": [{ "hooks": [{ "command": "reui-bridge.exe" }] }] } }"#).unwrap();
        assert_eq!(Agent::Claude.status(&paths), Status::Outdated, "hooks without the blocking question entry");
        std::fs::write(&file, r#"{ "hooks": { "PreToolUse": [{ "matcher": "AskUserQuestion|ExitPlanMode", "hooks": [{ "command": "reui-bridge.exe", "args": ["claude", "ask"], "timeout": 300 }] }] } }"#).unwrap();
        assert_eq!(Agent::Claude.status(&paths), Status::Connected);
        std::fs::write(&file, "{ oops").unwrap();
        assert_eq!(Agent::Claude.status(&paths), Status::Unreadable);
        let plugin = Agent::OpenCode.file(&paths);
        std::fs::create_dir_all(plugin.parent().unwrap()).unwrap();
        std::fs::write(&plugin, "// plugin").unwrap();
        assert_eq!(Agent::OpenCode.status(&paths), Status::Connected);
        assert!(bridge(&paths).is_none(), "no bridge shipped next to the exe here");
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn installs_are_found_by_their_folder_and_last_events_by_the_hubs_file() {
        let dir = crate::model::doc::tests::temp_dir("agents-installed");
        let paths = Paths { config: dir.join("c"), local: dir.join("l"), home: dir.join("h"), exe_dir: dir.clone(), sandbox: true };
        assert!(!Agent::Codex.installed(&paths));
        std::fs::create_dir_all(paths.home.join(".codex")).unwrap();
        assert!(Agent::Codex.installed(&paths));
        std::fs::create_dir_all(paths.home.join(".local").join("share").join("opencode")).unwrap();
        assert!(Agent::OpenCode.installed(&paths));
        assert_eq!(Agent::Claude.last_event(&paths), None);
        std::fs::create_dir_all(&paths.local).unwrap();
        std::fs::write(paths.local.join(STATUS_FILE), r#"{ "claude": { "last_event_ms": 42 }, "codex": {} }"#).unwrap();
        assert_eq!(Agent::Claude.last_event(&paths), Some(42));
        assert_eq!(Agent::Codex.last_event(&paths), None);
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn times_and_status_lines_read_naturally() {
        let now = 10_000_000_000;
        assert_eq!(ago(now, now - 5_000), "just now");
        assert_eq!(ago(now, now - 120_000), "2 min ago");
        assert_eq!(ago(now, now - 3 * 3_600_000), "3 h ago");
        assert_eq!(ago(now, now - 30 * 3_600_000), "1 day ago");
        assert_eq!(ago(now, now - 72 * 3_600_000), "3 days ago");
        assert_eq!(ago(now, now + 9_000), "just now", "a clock that moved back isn't negative");
        assert_eq!(status_line(Status::Connected, true, Some(now - 120_000), now), "Connected · last event 2 min ago");
        assert_eq!(status_line(Status::Connected, false, None, now), "Connected · no events yet");
        assert_eq!(status_line(Status::NotConnected, true, Some(1), now), "Not connected");
        assert_eq!(status_line(Status::NotConnected, false, None, now), "Agent not found on this PC");
    }
}
