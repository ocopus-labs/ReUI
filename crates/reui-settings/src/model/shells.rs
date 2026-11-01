//! Terminals (docs/PRD-terminal-commands.md): whether a shell has ReUI's hook for long
//! commands (read from the same files, by the same marker, as the bridge's installer in
//! `reui-bridge/src/shells.rs`), and connecting or disconnecting it by running the bridge.

use std::path::{Path, PathBuf};

use crate::paths::Paths;

/// The line the bridge's installer starts its block with.
const MARKER: &str = "# >>> ReUI: long commands >>>";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shell {
    PowerShell,
    Bash,
}

impl Shell {
    pub const ALL: [Shell; 2] = [Shell::PowerShell, Shell::Bash];

    pub fn arg(self) -> &'static str {
        match self {
            Shell::PowerShell => "powershell",
            Shell::Bash => "bash",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Shell::PowerShell => "PowerShell",
            Shell::Bash => "Git Bash",
        }
    }

    /// The file the hook goes in (PowerShell 7's profile for all hosts; `~/.bashrc`).
    pub fn file(self, paths: &Paths) -> PathBuf {
        match self {
            Shell::PowerShell => documents(paths).join("PowerShell").join("profile.ps1"),
            Shell::Bash => paths.home.join(".bashrc"),
        }
    }

    /// Whether ReUI's hook is in any of the shell's files (Windows PowerShell's profile too).
    pub fn connected(self, paths: &Paths) -> bool {
        let has = |p: &Path| std::fs::read_to_string(p).is_ok_and(|t| t.contains(MARKER));
        match self {
            Shell::PowerShell => has(&self.file(paths)) || has(&documents(paths).join("WindowsPowerShell").join("profile.ps1")),
            Shell::Bash => has(&self.file(paths)),
        }
    }
}

/// The Documents folder (often in OneDrive); a sandbox keeps it under its own home, as the
/// bridge does.
fn documents(paths: &Paths) -> PathBuf {
    if paths.sandbox {
        return paths.home.join("Documents");
    }
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_Documents, SHGetKnownFolderPath, KF_FLAG_DEFAULT};
    unsafe {
        if let Ok(raw) = SHGetKnownFolderPath(&FOLDERID_Documents, KF_FLAG_DEFAULT, None) {
            let path = raw.to_string().ok().map(PathBuf::from);
            CoTaskMemFree(Some(raw.0 as *const _));
            if let Some(path) = path {
                return path;
            }
        }
    }
    paths.home.join("Documents")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shell_is_connected_by_the_marker_in_its_file() {
        let dir = std::env::temp_dir().join(format!("reui-shells-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&dir);
        let paths = Paths { config: dir.join("roaming"), local: dir.join("local"), home: dir.join("home"), exe_dir: dir.clone(), sandbox: true };
        std::fs::create_dir_all(dir.join("home").join("Documents").join("WindowsPowerShell")).unwrap();
        assert!(!Shell::Bash.connected(&paths) && !Shell::PowerShell.connected(&paths));
        std::fs::write(dir.join("home").join(".bashrc"), format!("alias ll='ls -l'\n\n{MARKER}\n…\n")).unwrap();
        assert!(Shell::Bash.connected(&paths));
        // Windows PowerShell's profile counts too.
        std::fs::write(dir.join("home").join("Documents").join("WindowsPowerShell").join("profile.ps1"), MARKER).unwrap();
        assert!(Shell::PowerShell.connected(&paths));
        assert!(Shell::PowerShell.file(&paths).ends_with(r"Documents\PowerShell\profile.ps1"));
        _ = std::fs::remove_dir_all(&dir);
    }
}
