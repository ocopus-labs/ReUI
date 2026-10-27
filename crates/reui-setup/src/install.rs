//! Install and upgrade: one flow. Stop ReUI, put the new files in place atomically, then
//! the Start entry, the Apps & features entry and start-with-Windows, then launch it.
//! User data (`%APPDATA%\ReUI`) is never touched.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::layout::{Layout, APP, APP_USER_MODEL_ID, HUB_EXE, SETUP_EXE};
use crate::payload::{total_size, File};
use crate::registry;
use crate::system::System;

#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    pub no_autostart: bool,
    pub no_launch: bool,
}

#[derive(Debug, PartialEq)]
pub struct Outcome {
    pub upgrade: bool,
    pub autostart: bool,
}

/// `setup_exe` is copied in as `reui-setup.exe` (the uninstaller Apps & features runs).
pub fn install(layout: &Layout, files: &[File], setup_exe: &Path, options: Options, system: &mut dyn System) -> Result<Outcome, String> {
    let upgrade = registry::has_uninstall_entry(layout);
    system.stop_reui(&layout.install_dir);

    let dir = &layout.install_dir;
    std::fs::create_dir_all(dir).map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
    clean_leftovers(dir);
    let setup_bytes = setup_copy(setup_exe, &layout.exe(SETUP_EXE))?;
    let mut staged: Vec<(&str, &[u8])> = files.iter().map(|f| (f.name, &*f.bytes)).collect();
    if let Some(bytes) = &setup_bytes {
        staged.push((SETUP_EXE, bytes));
    }
    replace_all(dir, &staged)?;

    let hub = layout.exe(HUB_EXE);
    crate::shortcut::create(&layout.shortcut(), &hub, APP, APP_USER_MODEL_ID).map_err(|e| format!("Couldn't create the Start menu entry: {e}"))?;
    let size = total_size(files) + setup_bytes.as_ref().map_or(0, |b| b.len() as u64);
    registry::write_uninstall_entry(layout, size, &hub).map_err(|e| format!("Couldn't register ReUI in Apps & features: {e}"))?;

    let autostart = registry::want_autostart(upgrade, registry::autostart(layout).is_some(), options.no_autostart);
    let result = if autostart { registry::set_autostart(layout, &hub) } else { registry::remove_autostart(layout) };
    result.map_err(|e| format!("Couldn't set ReUI to start with Windows: {e}"))?;

    if !options.no_launch {
        system.launch(&hub)?;
    }
    Ok(Outcome { upgrade, autostart })
}

/// The setup's own bytes, unless it already is the installed copy.
fn setup_copy(setup_exe: &Path, installed: &Path) -> Result<Option<Vec<u8>>, String> {
    let same = match (std::fs::canonicalize(setup_exe), std::fs::canonicalize(installed)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if same {
        return Ok(None);
    }
    std::fs::read(setup_exe).map(Some).map_err(|e| format!("Couldn't read {}: {e}", setup_exe.display()))
}

fn new_name(path: &Path) -> PathBuf {
    with_suffix(path, ".new")
}

fn old_name(path: &Path) -> PathBuf {
    with_suffix(path, ".old")
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// `.new` files from an interrupted install and `.old` ones still locked last time.
fn clean_leftovers(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if name.ends_with(".exe.new") || name.ends_with(".exe.old") {
            _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Writes every file as `<name>.new`, then swaps each in: the current file is renamed to
/// `<name>.old` (allowed even while it runs), the new one renamed into place. If any swap
/// fails, the ones already done are rolled back, so ReUI is never left half-upgraded.
pub fn replace_all(dir: &Path, files: &[(&str, &[u8])]) -> Result<(), String> {
    for (name, bytes) in files {
        let path = new_name(&dir.join(name));
        std::fs::write(&path, bytes).map_err(|e| {
            discard_staged(dir, files);
            format!("Couldn't write {}: {e}", path.display())
        })?;
    }
    let mut swapped: Vec<(PathBuf, bool)> = Vec::new();
    for (name, _) in files {
        let target = dir.join(name);
        let had_old = target.exists();
        let result = (|| {
            if had_old {
                retry(|| std::fs::rename(&target, old_name(&target)))?;
            }
            retry(|| std::fs::rename(new_name(&target), &target)).inspect_err(|_| {
                if had_old {
                    _ = std::fs::rename(old_name(&target), &target);
                }
            })
        })();
        if let Err(e) = result {
            for (done, had_old) in swapped.iter().rev() {
                _ = std::fs::remove_file(done);
                if *had_old {
                    _ = std::fs::rename(old_name(done), done);
                }
            }
            discard_staged(dir, files);
            return Err(format!("Couldn't replace {} (is it in use?): {e}", target.display()));
        }
        swapped.push((target, had_old));
    }
    for (target, had_old) in swapped {
        if had_old {
            // Still locked if it's running (the installed setup itself): next install cleans it.
            _ = std::fs::remove_file(old_name(&target));
        }
    }
    Ok(())
}

fn discard_staged(dir: &Path, files: &[(&str, &[u8])]) {
    for (name, _) in files {
        _ = std::fs::remove_file(new_name(&dir.join(name)));
    }
}

/// Antivirus scanners briefly lock freshly written exes.
fn retry(mut op: impl FnMut() -> std::io::Result<()>) -> std::io::Result<()> {
    let mut attempt = 0;
    loop {
        match op() {
            Err(_) if attempt < 10 => {
                attempt += 1;
                std::thread::sleep(Duration::from_millis(200));
            }
            other => return other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("reui-setup-install-{tag}-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }

    #[test]
    fn replaces_files_and_leaves_no_temp_names() {
        let dir = temp("swap");
        std::fs::write(dir.join("a.exe"), b"old a").unwrap();
        replace_all(&dir, &[("a.exe", b"new a"), ("b.exe", b"new b")]).unwrap();
        assert_eq!(std::fs::read(dir.join("a.exe")).unwrap(), b"new a");
        assert_eq!(std::fs::read(dir.join("b.exe")).unwrap(), b"new b");
        assert_eq!(names(&dir), ["a.exe", "b.exe"]);
        _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_failed_swap_rolls_back_the_ones_already_done() {
        let dir = temp("rollback");
        use std::os::windows::fs::OpenOptionsExt;
        std::fs::write(dir.join("a.exe"), b"old a").unwrap();
        std::fs::write(dir.join("b.exe"), b"old b").unwrap();
        // Held open with no sharing (like a scanner might): it can't be renamed.
        let lock = std::fs::OpenOptions::new().read(true).share_mode(0).open(dir.join("b.exe")).unwrap();
        let err = replace_all(&dir, &[("a.exe", b"new a"), ("b.exe", b"new b")]).unwrap_err();
        drop(lock);
        assert!(err.contains("b.exe"), "{err}");
        assert_eq!(std::fs::read(dir.join("a.exe")).unwrap(), b"old a");
        assert_eq!(std::fs::read(dir.join("b.exe")).unwrap(), b"old b");
        assert_eq!(names(&dir), ["a.exe", "b.exe"]);
        _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn leftovers_are_cleaned() {
        let dir = temp("leftovers");
        for name in ["reui-hub.exe.old", "reui-hub.exe.new", "reui-hub.exe", "notes.txt"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        clean_leftovers(&dir);
        assert_eq!(names(&dir), ["notes.txt", "reui-hub.exe"]);
        _ = std::fs::remove_dir_all(dir);
    }
}
