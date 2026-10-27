//! Where an install lives. Every path and registry key comes from a [`Layout`], so tests
//! point the whole installer at a temp folder and a throwaway `HKCU\Software\ReUI-test-*`
//! key instead of the real profile.

use std::path::{Path, PathBuf};

pub const APP: &str = "ReUI";
pub const PUBLISHER: &str = "Rohit Kushwaha";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Ties the Start menu entry to the hub's windows (and later its notifications).
pub const APP_USER_MODEL_ID: &str = "ReUI.Hub";
pub const HUB_EXE: &str = "reui-hub.exe";
pub const BRIDGE_EXE: &str = "reui-bridge.exe";
pub const SETUP_EXE: &str = "reui-setup.exe";
const WINDOWS_CURRENT_VERSION: &str = r"Software\Microsoft\Windows\CurrentVersion";

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// `%LOCALAPPDATA%\Programs\ReUI`: the programs (per-user, no admin).
    pub install_dir: PathBuf,
    /// The Start menu's Programs folder.
    pub start_menu: PathBuf,
    /// `%APPDATA%\ReUI`: settings and clipboard history.
    pub roaming_data: PathBuf,
    /// `%LOCALAPPDATA%\ReUI`: caches, logs, the agent bridge copy.
    pub local_data: PathBuf,
    /// `%USERPROFILE%`: where coding agents keep the settings we hook into.
    pub home: PathBuf,
    /// The HKCU key holding `Run` and `Uninstall`.
    pub reg_base: String,
}

impl Layout {
    /// The current user's real locations.
    pub fn for_user() -> Result<Self, String> {
        let env = |name: &str| std::env::var_os(name).map(PathBuf::from).ok_or_else(|| format!("%{name}% is not set"));
        let local = env("LOCALAPPDATA")?;
        let roaming = env("APPDATA")?;
        let start_menu = crate::shortcut::programs_folder().unwrap_or_else(|| roaming.join(r"Microsoft\Windows\Start Menu\Programs"));
        Ok(Self {
            install_dir: local.join("Programs").join(APP),
            start_menu,
            roaming_data: roaming.join(APP),
            local_data: local.join(APP),
            home: env("USERPROFILE")?,
            reg_base: WINDOWS_CURRENT_VERSION.into(),
        })
    }

    #[cfg(test)]
    /// The same shape under `root`, with registry writes under `HKCU\<reg_base>`.
    pub fn under(root: &Path, reg_base: &str) -> Self {
        Self {
            install_dir: root.join(r"Local\Programs").join(APP),
            start_menu: root.join(r"Roaming\Microsoft\Windows\Start Menu\Programs"),
            roaming_data: root.join("Roaming").join(APP),
            local_data: root.join("Local").join(APP),
            home: root.join("home"),
            reg_base: reg_base.into(),
        }
    }

    pub fn exe(&self, name: &str) -> PathBuf {
        self.install_dir.join(name)
    }

    pub fn shortcut(&self) -> PathBuf {
        self.start_menu.join(format!("{APP}.lnk"))
    }

    pub fn run_key(&self) -> String {
        format!(r"{}\Run", self.reg_base)
    }

    pub fn uninstall_key(&self) -> String {
        format!(r"{}\Uninstall\{APP}", self.reg_base)
    }

    /// The copy of `reui-bridge.exe` agents' hooks run (kept in step by the hub).
    pub fn bridge_copy_dir(&self) -> PathBuf {
        self.local_data.join("bin")
    }
}

/// `"C:\path with spaces\x.exe"`: registry command lines always quote the program.
pub fn quoted(path: &Path) -> String {
    format!("\"{}\"", path.display())
}

/// Is `path` inside `dir` (case-insensitive, ignoring `\\?\` and trailing separators)?
pub fn is_under(path: &Path, dir: &Path) -> bool {
    fn norm(p: &Path) -> String {
        let s = p.to_string_lossy().replace('/', "\\").to_lowercase();
        let s = s.strip_prefix(r"\\?\").unwrap_or(&s).to_string();
        s.trim_end_matches('\\').to_string()
    }
    let (path, dir) = (norm(path), norm(dir));
    !dir.is_empty() && path.len() > dir.len() && path.starts_with(&dir) && path[dir.len()..].starts_with('\\')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_layout_stays_inside_its_root() {
        let root = Path::new(r"C:\tmp\reui-test");
        let layout = Layout::under(root, r"Software\ReUI-test-1");
        for path in [&layout.install_dir, &layout.start_menu, &layout.roaming_data, &layout.local_data, &layout.home] {
            assert!(is_under(path, root), "{}", path.display());
        }
        assert_eq!(layout.run_key(), r"Software\ReUI-test-1\Run");
        assert_eq!(layout.uninstall_key(), r"Software\ReUI-test-1\Uninstall\ReUI");
        assert_eq!(layout.shortcut().file_name().unwrap(), "ReUI.lnk");
        assert!(is_under(&layout.bridge_copy_dir(), &layout.local_data));
    }

    #[test]
    fn real_layout_uses_the_standard_places() {
        let Ok(layout) = Layout::for_user() else { return };
        assert!(layout.install_dir.ends_with(r"Programs\ReUI"));
        assert_eq!(layout.uninstall_key(), r"Software\Microsoft\Windows\CurrentVersion\Uninstall\ReUI");
        assert_eq!(layout.run_key(), r"Software\Microsoft\Windows\CurrentVersion\Run");
    }

    #[test]
    fn under_matching() {
        let dir = Path::new(r"C:\Users\a\AppData\Local\Programs\ReUI");
        assert!(is_under(Path::new(r"c:\users\A\appdata\local\programs\reui\reui-hub.exe"), dir));
        assert!(is_under(Path::new(r"\\?\C:\Users\a\AppData\Local\Programs\ReUI\reui-hub.exe"), dir));
        assert!(is_under(Path::new(r"C:\Users\a\AppData\Local\Programs\ReUI\sub\x.exe"), Path::new(r"C:\Users\a\AppData\Local\Programs\ReUI\")));
        assert!(!is_under(Path::new(r"C:\Users\a\AppData\Local\Programs\ReUI2\reui-hub.exe"), dir));
        assert!(!is_under(dir, dir));
        assert!(!is_under(Path::new(r"D:\ReUI\target\release\reui-hub.exe"), dir));
        assert!(!is_under(Path::new(r"C:\x"), Path::new("")));
    }

    #[test]
    fn quoting() {
        assert_eq!(quoted(Path::new(r"C:\Program Files\a.exe")), r#""C:\Program Files\a.exe""#);
    }
}
