//! Uninstall as a plan: a pure, ordered list of steps (tested), then a best-effort run that
//! keeps going past a failed step and reports what it couldn't do.
//!
//! Order matters: ReUI stops first (it puts Windows' flyout back as it quits), the flyout
//! is restored from the crash marker while the hub exe still exists, agent hooks are
//! removed while the bridge still exists, and the install folder goes last (it may hold
//! this very exe, so it's deleted after we exit).

use std::path::Path;

use crate::layout::{Layout, BRIDGE_EXE, HUB_EXE};
use crate::registry;
use crate::system::System;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agent {
    Claude,
    Codex,
    OpenCode,
    /// The shells' hook for long commands (docs/PRD-terminal-commands.md), removed the same
    /// way: `reui-bridge uninstall powershell|bash`.
    PowerShell,
    Bash,
}

impl Agent {
    pub fn arg(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
            Agent::OpenCode => "opencode",
            Agent::PowerShell => "powershell",
            Agent::Bash => "bash",
        }
    }
}

/// The line the bridge starts a shell's hook with (`reui-bridge/src/shells.rs`).
const SHELL_MARKER: &str = "# >>> ReUI: long commands >>>";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    StopReui,
    RestoreFlyout,
    RemoveHooks(Agent),
    RemoveAutostart,
    RemoveShortcut,
    RemoveUninstallEntry,
    RemoveBridgeCopy,
    DeleteUserData,
    RemoveInstallDir,
}

pub fn plan(agents: &[Agent], delete_data: bool) -> Vec<Step> {
    let mut steps = vec![Step::StopReui, Step::RestoreFlyout];
    steps.extend(agents.iter().map(|&a| Step::RemoveHooks(a)));
    steps.extend([Step::RemoveAutostart, Step::RemoveShortcut, Step::RemoveUninstallEntry, Step::RemoveBridgeCopy]);
    if delete_data {
        steps.push(Step::DeleteUserData);
    }
    steps.push(Step::RemoveInstallDir);
    steps
}

/// Shells with ReUI's hook in their startup file. `documents` is the user's Documents
/// folder (PowerShell's profiles live there, often moved into OneDrive). A hook left behind
/// stays quiet anyway: it ignores a missing bridge.
pub fn hooked_shells(home: &Path, documents: &Path) -> Vec<Agent> {
    let has = |path: &Path| std::fs::read_to_string(path).is_ok_and(|t| t.contains(SHELL_MARKER));
    let mut shells = Vec::new();
    if has(&documents.join(r"PowerShell\profile.ps1")) || has(&documents.join(r"WindowsPowerShell\profile.ps1")) {
        shells.push(Agent::PowerShell);
    }
    if has(&home.join(".bashrc")) {
        shells.push(Agent::Bash);
    }
    shells
}

/// Agents with ReUI's user-scope integration in place. (Project-scope hooks live in
/// project folders we can't enumerate; they fail open once the bridge is gone.)
pub fn hooked_agents(home: &Path) -> Vec<Agent> {
    let mentions_bridge = |path: &Path| std::fs::read_to_string(path).is_ok_and(|t| t.contains("reui-bridge"));
    let mut agents = Vec::new();
    if mentions_bridge(&home.join(r".claude\settings.json")) {
        agents.push(Agent::Claude);
    }
    if mentions_bridge(&home.join(r".codex\hooks.json")) {
        agents.push(Agent::Codex);
    }
    if home.join(r".config\opencode\plugins\reui.js").is_file() {
        agents.push(Agent::OpenCode);
    }
    agents
}

/// Runs the steps; returns what failed (empty when everything went).
pub fn run(layout: &Layout, steps: &[Step], current_exe: &Path, system: &mut dyn System) -> Vec<String> {
    let mut failures = Vec::new();
    for &step in steps {
        if let Err(e) = run_step(layout, step, current_exe, system) {
            failures.push(e);
        }
    }
    failures
}

fn run_step(layout: &Layout, step: Step, current_exe: &Path, system: &mut dyn System) -> Result<(), String> {
    let remove_dir = |dir: &Path| match std::fs::remove_dir_all(dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("Couldn't delete {}: {e}", dir.display())),
        _ => Ok(()),
    };
    match step {
        Step::StopReui => system.stop_reui(&layout.install_dir),
        Step::RestoreFlyout => {
            let hub = layout.exe(HUB_EXE);
            if hub.is_file() {
                system.run(&hub, &["--restore-flyout"])?;
            }
        }
        Step::RemoveHooks(agent) => {
            let bridge = layout.exe(BRIDGE_EXE);
            let bridge = if bridge.is_file() { bridge } else { layout.bridge_copy_dir().join(BRIDGE_EXE) };
            system.run(&bridge, &["uninstall", agent.arg()]).map_err(|e| format!("Couldn't remove ReUI's hooks for {}: {e}", agent.arg()))?;
        }
        Step::RemoveAutostart => registry::remove_autostart(layout).map_err(|e| format!("Couldn't remove start with Windows: {e}"))?,
        // The Start menu entry and Send to → ReUI shelf.
        Step::RemoveShortcut => {
            if let Err(e) = std::fs::remove_file(layout.send_to_shortcut()) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    return Err(format!("Couldn't remove Send to → ReUI shelf: {e}"));
                }
            }
            match std::fs::remove_file(layout.shortcut()) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(format!("Couldn't remove the Start menu entry: {e}")),
                _ => {}
            }
        }
        Step::RemoveUninstallEntry => registry::remove_uninstall_entry(layout).map_err(|e| format!("Couldn't remove the Apps & features entry: {e}"))?,
        Step::RemoveBridgeCopy => remove_dir(&layout.bridge_copy_dir())?,
        Step::DeleteUserData => {
            remove_dir(&layout.roaming_data)?;
            remove_dir(&layout.local_data)?;
        }
        Step::RemoveInstallDir => {
            let dir = &layout.install_dir;
            if crate::layout::is_under(current_exe, dir) {
                // Everything else now; the folder (with this exe) once we've exited.
                if let Ok(entries) = std::fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if !same_file(&path, current_exe) {
                            _ = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
                        }
                    }
                }
                system.delete_after_exit(dir);
            } else {
                remove_dir(dir)?;
            }
        }
    }
    Ok(())
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Step::*;

    #[test]
    fn plan_order() {
        assert_eq!(
            plan(&[Agent::Claude, Agent::OpenCode], false),
            [StopReui, RestoreFlyout, RemoveHooks(Agent::Claude), RemoveHooks(Agent::OpenCode), RemoveAutostart, RemoveShortcut, RemoveUninstallEntry, RemoveBridgeCopy, RemoveInstallDir]
        );
        let with_data = plan(&[], true);
        assert_eq!(with_data, [StopReui, RestoreFlyout, RemoveAutostart, RemoveShortcut, RemoveUninstallEntry, RemoveBridgeCopy, DeleteUserData, RemoveInstallDir]);
    }

    #[test]
    fn plan_invariants() {
        for agents in [&[][..], &[Agent::Codex], &[Agent::Claude, Agent::Codex, Agent::OpenCode]] {
            for data in [false, true] {
                let steps = plan(agents, data);
                let at = |s: Step| steps.iter().position(|&x| x == s);
                assert_eq!(steps[0], StopReui, "nothing runs while ReUI does");
                assert!(at(RestoreFlyout).unwrap() < at(RemoveInstallDir).unwrap(), "needs the hub exe");
                for &a in agents {
                    assert!(at(RemoveHooks(a)).unwrap() < at(RemoveBridgeCopy).unwrap(), "needs a bridge");
                }
                assert_eq!(*steps.last().unwrap(), RemoveInstallDir);
                assert_eq!(at(DeleteUserData).is_some(), data, "data only when asked");
            }
        }
    }

    #[test]
    fn finds_hooked_agents() {
        let home = std::env::temp_dir().join(format!("reui-setup-agents-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&home);
        assert!(hooked_agents(&home).is_empty());
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        std::fs::write(home.join(r".claude\settings.json"), r#"{"model":"x"}"#).unwrap();
        assert!(hooked_agents(&home).is_empty(), "settings without our hooks");
        std::fs::write(home.join(r".claude\settings.json"), r#"{"hooks":{"Stop":[{"hooks":[{"command":"C:/x/reui-bridge.exe"}]}]}}"#).unwrap();
        std::fs::create_dir_all(home.join(r".config\opencode\plugins")).unwrap();
        std::fs::write(home.join(r".config\opencode\plugins\reui.js"), "").unwrap();
        assert_eq!(hooked_agents(&home), [Agent::Claude, Agent::OpenCode]);
        _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn finds_hooked_shells() {
        let home = std::env::temp_dir().join(format!("reui-setup-shells-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&home);
        let docs = home.join("OneDrive").join("Documents");
        assert!(hooked_shells(&home, &docs).is_empty());
        std::fs::create_dir_all(docs.join("WindowsPowerShell")).unwrap();
        std::fs::write(docs.join(r"WindowsPowerShell\profile.ps1"), format!("Set-Alias g git\n{SHELL_MARKER}\n")).unwrap();
        std::fs::write(home.join(".bashrc"), "alias ll='ls -l'\n").unwrap();
        assert_eq!(hooked_shells(&home, &docs), [Agent::PowerShell], "a .bashrc without our block");
        std::fs::write(home.join(".bashrc"), SHELL_MARKER).unwrap();
        assert_eq!(hooked_shells(&home, &docs), [Agent::PowerShell, Agent::Bash]);
        _ = std::fs::remove_dir_all(home);
    }
}
