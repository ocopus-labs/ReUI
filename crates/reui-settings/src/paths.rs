//! Where everything lives. Normally the real folders; with `REUI_CONFIG_DIR` set, every
//! file this window reads or writes (settings, data sizes, agent configs, the startup entry)
//! moves into that folder, and nothing is sent to a running ReUI. That is for testing the
//! window without touching the user's setup.

use std::path::{Path, PathBuf};

/// The Run key the startup entry goes in (HKCU).
pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// Where a sandboxed window keeps its startup entry instead.
const SANDBOX_RUN_KEY: &str = r"Software\ReUI\SettingsSandbox\Run";

#[derive(Clone, Debug)]
pub struct Paths {
    /// `%APPDATA%\ReUI`: settings files and clipboard history.
    pub config: PathBuf,
    /// `%LOCALAPPDATA%\ReUI`: caches, logs and the installed bridge.
    pub local: PathBuf,
    /// The user's home, where coding agents keep their configs.
    pub home: PathBuf,
    /// The folder this exe is in (the hub and the bridge sit next to it).
    pub exe_dir: PathBuf,
    /// `REUI_CONFIG_DIR` is set: test mode.
    pub sandbox: bool,
}

impl Paths {
    pub fn from_env() -> Self {
        let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_else(|| PathBuf::from("."));
        let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
        match var("REUI_CONFIG_DIR") {
            Some(root) => Self { config: root.join("roaming"), local: root.join("local"), home: root.join("home"), exe_dir, sandbox: true },
            None => Self {
                config: var("APPDATA").unwrap_or_else(|| PathBuf::from(".")).join("ReUI"),
                local: var("LOCALAPPDATA").unwrap_or_else(|| PathBuf::from(".")).join("ReUI"),
                home: var("USERPROFILE").unwrap_or_else(|| PathBuf::from(".")),
                exe_dir,
                sandbox: false,
            },
        }
    }

    pub fn file(&self, name: &str) -> PathBuf {
        self.config.join(name)
    }

    pub fn run_key(&self) -> &'static str {
        if self.sandbox {
            SANDBOX_RUN_KEY
        } else {
            RUN_KEY
        }
    }

    /// The bridge agents run: the stable copy the hub keeps in step with itself.
    pub fn installed_bridge(&self) -> PathBuf {
        self.local.join("bin").join("reui-bridge.exe")
    }

    pub fn hub_exe(&self) -> PathBuf {
        self.exe_dir.join("reui-hub.exe")
    }

    /// `C:\Users\me\.claude\settings.json` → `~\.claude\settings.json`, for display.
    pub fn short(&self, path: &Path) -> String {
        match path.strip_prefix(&self.home) {
            Ok(rest) if !self.home.as_os_str().is_empty() => format!("~\\{}", rest.display()),
            _ => path.display().to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_under_home_are_shortened() {
        let paths = Paths { config: "c".into(), local: "l".into(), home: r"C:\Users\me".into(), exe_dir: ".".into(), sandbox: true };
        assert_eq!(paths.short(Path::new(r"C:\Users\me\.claude\settings.json")), r"~\.claude\settings.json");
        assert_eq!(paths.short(Path::new(r"D:\x.json")), r"D:\x.json");
        assert_eq!(paths.run_key(), SANDBOX_RUN_KEY);
    }
}
