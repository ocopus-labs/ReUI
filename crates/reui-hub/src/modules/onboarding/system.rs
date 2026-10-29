//! The OS side of onboarding (all blocking: run off the hub thread): whether now is a bad
//! moment for a tip, which coding agents are running and connected, connecting one,
//! opening the welcome, and the new-user visibility default.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use windows::Win32::Foundation::{CloseHandle, RECT};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONULL};
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
use windows::Win32::UI::Shell::{SHQueryUserNotificationState, QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN};
use windows::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetForegroundWindow, GetWindowRect};

use crate::paths;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A tip now would be wasted or in the way: ReUI is paused, or something fullscreen (a
/// game, a video, a presentation) is in front, where the island hides anyway.
pub fn quiet() -> bool {
    paused(&paths::config().join("settings.json"), reui_protocol::unix_ms()) || fullscreen()
}

/// "Pause ReUI" is on (the island's `paused_until`, Unix ms), or the island is hidden with
/// its shortcut (`hidden`): either way a tip would show to no one.
pub fn paused(settings: &Path, now: u64) -> bool {
    let Ok(text) = std::fs::read_to_string(settings) else { return false };
    let Ok(v) = serde_json::from_str::<Value>(&text) else { return false };
    v.get("hidden").and_then(Value::as_bool) == Some(true) || v.get("paused_until").and_then(Value::as_u64).is_some_and(|until| until > now)
}

fn fullscreen() -> bool {
    if matches!(unsafe { SHQueryUserNotificationState() }, Ok(QUNS_BUSY | QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE)) {
        return true;
    }
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_invalid() {
            return false;
        }
        let mut class = [0u16; 64];
        let len = GetClassNameW(fg, &mut class) as usize;
        if matches!(String::from_utf16_lossy(&class[..len]).as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd") {
            return false;
        }
        let monitor = MonitorFromWindow(fg, MONITOR_DEFAULTTONULL);
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let mut rect = RECT::default();
        !monitor.is_invalid() && GetMonitorInfoW(monitor, &mut info).as_bool() && GetWindowRect(fg, &mut rect).is_ok() && covers(rect, info.rcMonitor)
    }
}

fn covers(window: RECT, monitor: RECT) -> bool {
    window.left <= monitor.left && window.top <= monitor.top && window.right >= monitor.right && window.bottom >= monitor.bottom
}

/// The agent (module name) an executable name belongs to.
pub fn agent_of(exe: &str) -> Option<&'static str> {
    match exe.to_ascii_lowercase().as_str() {
        "claude.exe" => Some("claude"),
        "codex.exe" => Some("codex"),
        "opencode.exe" => Some("opencode"),
        _ => None,
    }
}

/// Coding agents with a process running now (one ToolHelp snapshot).
pub fn running_agents() -> Vec<&'static str> {
    let mut found = Vec::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return found };
        let mut entry = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut more = Process32FirstW(snapshot, &mut entry).is_ok();
        while more {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            if let Some(agent) = agent_of(&String::from_utf16_lossy(&entry.szExeFile[..len])) {
                if !found.contains(&agent) {
                    found.push(agent);
                }
            }
            more = Process32NextW(snapshot, &mut entry).is_ok();
        }
        _ = CloseHandle(snapshot);
    }
    found
}

/// The file Connect changes for an agent (user scope, the installer's default).
pub fn agent_file(agent: &str, home: &Path) -> PathBuf {
    match agent {
        "codex" => home.join(".codex").join("hooks.json"),
        "opencode" => home.join(".config").join("opencode").join("plugins").join("reui.js"),
        _ => home.join(".claude").join("settings.json"),
    }
}

/// `C:\Users\me\.claude\settings.json` → `~\.claude\settings.json`.
pub fn short(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) => format!("~\\{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// The agent's hooks already point at a `reui-bridge` (or OpenCode's plugin is there).
pub fn connected(agent: &str, home: &Path) -> bool {
    let file = agent_file(agent, home);
    if agent == "opencode" {
        return file.exists();
    }
    std::fs::read_to_string(&file).is_ok_and(|text| has_bridge(&text))
}

/// Any hook command naming `reui-bridge` (the installer's own test is stricter; a mention
/// is enough to not offer again).
fn has_bridge(text: &str) -> bool {
    text.to_ascii_lowercase().contains("reui-bridge")
}

/// The bridge agents run: the installed copy the hub keeps in step, else the one beside us.
fn bridge() -> Option<PathBuf> {
    let installed = paths::local().join("bin").join("reui-bridge.exe");
    if installed.exists() {
        return Some(installed);
    }
    let shipped = std::env::current_exe().ok()?.with_file_name("reui-bridge.exe");
    if !shipped.exists() {
        return None;
    }
    std::fs::create_dir_all(installed.parent()?).ok()?;
    std::fs::copy(&shipped, &installed).ok()?;
    Some(installed)
}

/// `reui-bridge install <agent>`: the installer's error, if any.
pub fn connect(agent: &str) -> Result<(), String> {
    let bridge = bridge().ok_or("reui-bridge.exe is missing")?;
    let mut command = Command::new(&bridge);
    command.args(["install", agent]).creation_flags(CREATE_NO_WINDOW);
    if paths::sandbox().is_some() {
        command.env("USERPROFILE", paths::home());
    }
    let output = command.output().map_err(|e| format!("can't run the bridge: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(err.strip_prefix("reui-bridge: ").unwrap_or(&err).to_string())
}

/// Opens the welcome (`reui-settings.exe --welcome`, beside the hub). It inherits
/// `REUI_CONFIG_DIR`, so a test hub opens a test window.
pub fn open_welcome() {
    let Ok(exe) = std::env::current_exe() else { return };
    let settings = exe.with_file_name("reui-settings.exe");
    if !settings.is_file() {
        eprintln!("onboarding: {} not found; no welcome", settings.display());
        return;
    }
    if let Err(e) = Command::new(&settings).arg("--welcome").spawn() {
        eprintln!("onboarding: can't open the welcome: {e}");
    }
}

/// Files only an existing ReUI user has.
const EXISTING_USER_FILES: [&str; 6] = ["settings.json", "clipboard.db", "clipboard.json", "media.json", "osd.json", "devices.json"];

/// A brand-new user (nothing of ReUI's in `config` yet) gets "When active" visibility: the
/// island shows while something is happening (PRD D4). Existing users keep what they had.
/// Returns whether it wrote the default.
pub fn new_user_defaults(config: &Path) -> bool {
    if EXISTING_USER_FILES.iter().any(|f| config.join(f).exists()) {
        return false;
    }
    let path = config.join("settings.json");
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(config)?;
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, "{\n  \"visibility\": \"when-active\"\n}\n")?;
        std::fs::rename(&temp, &path)
    };
    match write() {
        Ok(()) => true,
        Err(e) => {
            eprintln!("onboarding: can't write {}: {e}", path.display());
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("reui-onboarding-{name}-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn agents_by_process_name() {
        assert_eq!(agent_of("Claude.exe"), Some("claude"));
        assert_eq!(agent_of("codex.exe"), Some("codex"));
        assert_eq!(agent_of("opencode.exe"), Some("opencode"));
        assert_eq!(agent_of("node.exe"), None);
    }

    #[test]
    fn connection_is_read_from_the_agents_files() {
        let home = temp("home");
        assert!(!connected("claude", &home));
        let file = agent_file("claude", &home);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, r#"{ "hooks": { "Stop": [{ "hooks": [{ "command": "C:/x/reui-bridge.exe claude" }] }] } }"#).unwrap();
        assert!(connected("claude", &home));
        assert_eq!(short(&file, &home), r"~\.claude\settings.json");
        _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn only_new_users_get_the_when_active_default() {
        let fresh = temp("fresh");
        assert!(new_user_defaults(&fresh));
        let text = std::fs::read_to_string(fresh.join("settings.json")).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&text).unwrap()["visibility"], "when-active");
        // Second time: the file is there, nothing changes.
        assert!(!new_user_defaults(&fresh));
        let existing = temp("existing");
        std::fs::write(existing.join("media.json"), "{}").unwrap();
        assert!(!new_user_defaults(&existing));
        assert!(!existing.join("settings.json").exists());
        _ = std::fs::remove_dir_all(&fresh);
        _ = std::fs::remove_dir_all(&existing);
    }

    #[test]
    fn pause_is_read_from_the_island_settings() {
        let dir = temp("pause");
        let path = dir.join("settings.json");
        assert!(!paused(&path, 10));
        std::fs::write(&path, r#"{ "paused_until": 100 }"#).unwrap();
        assert!(paused(&path, 10));
        assert!(!paused(&path, 100));
        std::fs::write(&path, r#"{ "hidden": true }"#).unwrap();
        assert!(paused(&path, 100), "hidden with its shortcut");
        _ = std::fs::remove_dir_all(&dir);
    }
}
