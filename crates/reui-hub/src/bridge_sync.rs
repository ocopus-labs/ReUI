//! Keeps the installed `reui-bridge` (the copy agents run as a hook) in step with the hub.
//!
//! Agents run the bridge from `%LOCALAPPDATA%\ReUI\bin`, not from the build folder, so a
//! hub update that changes the protocol would otherwise leave the old bridge talking a
//! version the hub rejects, and agent integration would silently stop (it fails open).
//! On start, the hub replaces the installed copy if it differs from the bridge next to it.
//! A running exe can't be overwritten but can be renamed, so the old one is moved aside and
//! deleted on a later start.

use std::path::{Path, PathBuf};

pub fn installed_path() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|dir| PathBuf::from(dir).join("ReUI").join("bin").join("reui-bridge.exe"))
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

/// Runs the sync for the standard install location, logging the outcome.
pub fn run() {
    let (Ok(exe), Some(installed)) = (std::env::current_exe(), installed_path()) else { return };
    match sync(&exe.with_file_name("reui-bridge.exe"), &installed) {
        Ok(true) => println!("hub: updated the installed bridge at {}", installed.display()),
        Ok(false) => {}
        Err(e) => eprintln!("hub: couldn't update the installed bridge at {}: {e}", installed.display()),
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
}
