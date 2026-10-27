//! Where the hub keeps things. Normally `%APPDATA%\ReUI` (settings, clipboard history) and
//! `%LOCALAPPDATA%\ReUI` (caches, logs, the installed bridge). With `REUI_CONFIG_DIR` set,
//! both move into that folder (`roaming\`, `local\`, and `home\` for coding agents'
//! configs), the same layout the Settings window uses, so a test hub never touches the
//! user's real files.

use std::path::PathBuf;

fn env(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// `REUI_CONFIG_DIR` is set: test mode (own files, own pipe, own single-instance lock).
pub fn sandbox() -> Option<PathBuf> {
    env("REUI_CONFIG_DIR")
}

/// `%APPDATA%\ReUI`: settings files and clipboard history.
pub fn config() -> PathBuf {
    match sandbox() {
        Some(root) => root.join("roaming"),
        None => env("APPDATA").unwrap_or_else(|| PathBuf::from(".")).join("ReUI"),
    }
}

/// `%LOCALAPPDATA%\ReUI`: caches, logs and the installed bridge.
pub fn local() -> PathBuf {
    match sandbox() {
        Some(root) => root.join("local"),
        None => env("LOCALAPPDATA").unwrap_or_else(|| PathBuf::from(".")).join("ReUI"),
    }
}

/// The user's home, where coding agents keep their configs.
pub fn home() -> PathBuf {
    match sandbox() {
        Some(root) => root.join("home"),
        None => env("USERPROFILE").unwrap_or_else(|| PathBuf::from(".")),
    }
}
