//! Coding agents: whether ReUI is connected to each (read the same way the bridge's
//! installer finds its own entries, `reui-bridge/src/install.rs`), and connecting or
//! disconnecting by running that installer.

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

    pub fn status(self, paths: &Paths) -> Status {
        let file = self.file(paths);
        if self == Agent::OpenCode {
            return if file.exists() { Status::Connected } else { Status::NotConnected };
        }
        match std::fs::read_to_string(&file) {
            Ok(text) if text.trim().is_empty() => Status::NotConnected,
            Ok(text) => match serde_json::from_str::<Value>(&text) {
                Ok(root) if has_our_hooks(&root) => Status::Connected,
                Ok(Value::Object(_)) => Status::NotConnected,
                _ => Status::Unreadable,
            },
            Err(_) => Status::NotConnected,
        }
    }
}

/// Any hook handler whose command is a `reui-bridge` executable (the installer's `is_ours`).
pub fn has_our_hooks(root: &Value) -> bool {
    let Some(hooks) = root.get("hooks").and_then(Value::as_object) else { return false };
    hooks.values().filter_map(Value::as_array).flatten().filter_map(|group| group.get("hooks").and_then(Value::as_array)).flatten().any(is_ours)
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
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = Command::new(bridge);
    command.args([if connect { "install" } else { "uninstall" }, agent.arg()]).creation_flags(CREATE_NO_WINDOW);
    if paths.sandbox {
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
    fn status_reads_the_agents_files() {
        let dir = crate::model::doc::tests::temp_dir("agents");
        let paths = Paths { config: dir.join("c"), local: dir.join("l"), home: dir.clone(), exe_dir: dir.clone(), sandbox: true };
        assert_eq!(Agent::Claude.status(&paths), Status::NotConnected);
        let file = Agent::Claude.file(&paths);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, r#"{ "hooks": { "Stop": [{ "hooks": [{ "command": "reui-bridge.exe" }] }] } }"#).unwrap();
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
}
