//! Keeps the installed `reui-bridge` (the copy agents run as a hook) in step with the hub.
//!
//! Agents run the bridge from `%LOCALAPPDATA%\ReUI\bin`, not from the build folder, so a
//! hub update that changes the protocol would otherwise leave the old bridge talking a
//! version the hub rejects, and agent integration would silently stop (it fails open).
//! On start, the hub replaces the installed copy if it differs from the bridge next to it.
//! A running exe can't be overwritten but can be renamed, so the old one is moved aside and
//! deleted on a later start. The OpenCode plugin, which names the bridge and is code
//! OpenCode runs itself, is rewritten the same way when an older build wrote it.

use std::path::{Path, PathBuf};

pub fn installed_path() -> Option<PathBuf> {
    Some(crate::paths::local().join("bin").join("reui-bridge.exe"))
}

/// Replaces `installed` with `source` if they differ. Returns whether it changed anything.
pub fn sync(source: &Path, installed: &Path) -> std::io::Result<bool> {
    let aside = installed.with_extension("exe.old");
    // Left over from an earlier update whose bridge was still running then.
    _ = std::fs::remove_file(&aside);
    if !installed.exists() || !source.exists() {
        return Ok(false); // not installed: nothing to keep in step
    }
    if std::fs::read(source)? == std::fs::read(installed)? {
        return Ok(false);
    }
    std::fs::rename(installed, &aside)?;
    if let Err(e) = std::fs::copy(source, installed) {
        // Put the old one back rather than leave agents with no bridge at all.
        _ = std::fs::rename(&aside, installed);
        return Err(e);
    }
    _ = std::fs::remove_file(&aside);
    Ok(true)
}

/// The plugin `reui-bridge install opencode` writes, `"__REUI_BRIDGE__"` standing for the
/// bridge's path.
const OPENCODE_PLUGIN: &str = include_str!("../../reui-bridge/src/opencode-plugin.js");

/// Rewrites an installed OpenCode plugin from another build with this build's, keeping the
/// bridge path it names. The bridge's exe is kept in step above, but the plugin is code
/// OpenCode runs itself, and the one before `reui-plugin: 2` relied on a hook OpenCode
/// stopped calling in 1.3 (so its permission answers did nothing). Only our own file is
/// touched; returns whether it changed.
pub fn sync_plugin(path: &Path) -> std::io::Result<bool> {
    let Ok(old) = std::fs::read_to_string(path) else { return Ok(false) };
    // `const BRIDGE = "<path as a JSON string>";`, as the installer wrote it.
    let Some(bridge) = old.lines().find_map(|l| l.trim().strip_prefix("const BRIDGE = ")).and_then(|l| l.strip_suffix(';')) else { return Ok(false) };
    if !old.starts_with("// ReUI for OpenCode") || !bridge.starts_with('"') {
        return Ok(false);
    }
    let new = OPENCODE_PLUGIN.replace("\"__REUI_BRIDGE__\"", bridge);
    if new == old {
        return Ok(false);
    }
    let temp = path.with_extension("reui-tmp");
    std::fs::write(&temp, &new)?;
    std::fs::rename(&temp, path)?;
    Ok(true)
}

/// Runs the sync for the standard install location, logging the outcome.
pub fn run() {
    let (Ok(exe), Some(installed)) = (std::env::current_exe(), installed_path()) else { return };
    match sync(&exe.with_file_name("reui-bridge.exe"), &installed) {
        Ok(true) => println!("hub: updated the installed bridge at {}", installed.display()),
        Ok(false) => {}
        Err(e) => eprintln!("hub: couldn't update the installed bridge at {}: {e}", installed.display()),
    }
    // User scope only: a project's plugin is wherever that project is.
    let plugin = crate::paths::home().join(".config").join("opencode").join("plugins").join("reui.js");
    match sync_plugin(&plugin) {
        Ok(true) => println!("hub: updated the OpenCode plugin at {}", plugin.display()),
        Ok(false) => {}
        Err(e) => eprintln!("hub: couldn't update the OpenCode plugin at {}: {e}", plugin.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_a_stale_copy_and_leaves_a_current_one_alone() {
        let dir = std::env::temp_dir().join(format!("reui-bridge-sync-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("new.exe");
        let installed = dir.join("reui-bridge.exe");
        std::fs::write(&source, b"v6").unwrap();
        std::fs::write(&installed, b"v5").unwrap();

        assert!(sync(&source, &installed).unwrap());
        assert_eq!(std::fs::read(&installed).unwrap(), b"v6");
        assert!(!installed.with_extension("exe.old").exists());
        assert!(!sync(&source, &installed).unwrap(), "already current");

        // Not installed: never creates one.
        std::fs::remove_file(&installed).unwrap();
        assert!(!sync(&source, &installed).unwrap());
        assert!(!installed.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_older_plugin_is_replaced_and_keeps_its_bridge() {
        let dir = std::env::temp_dir().join(format!("reui-plugin-sync-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("reui.js");
        // The first plugin's head, as `reui-bridge install opencode` wrote it.
        let old = "// ReUI for OpenCode: shows OpenCode in the ReUI island\nconst BRIDGE = \"C:/Users/me/AppData/Local/ReUI/bin/reui-bridge.exe\";\n\"permission.ask\": async () => {}\n";
        std::fs::write(&path, old).unwrap();
        assert!(sync_plugin(&path).unwrap());
        let new = std::fs::read_to_string(&path).unwrap();
        assert!(new.contains("const BRIDGE = \"C:/Users/me/AppData/Local/ReUI/bin/reui-bridge.exe\";"));
        assert!(new.contains("reui-plugin: 2") && new.contains("permission.asked") && !new.contains("__REUI_BRIDGE__"));
        assert!(!sync_plugin(&path).unwrap(), "already current");
        // Someone else's file of that name stays as it is.
        std::fs::write(&path, "export const Mine = async () => ({});\n").unwrap();
        assert!(!sync_plugin(&path).unwrap());
        assert!(!sync_plugin(&dir.join("missing.js")).unwrap());
        std::fs::remove_dir_all(&dir).ok();
    }
}
