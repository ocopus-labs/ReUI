//! `reui-setup`: ReUI's per-user installer and uninstaller (no admin, no UI framework).
//!
//! - `reui-setup.exe [--no-autostart] [--no-launch] [--quiet]` installs or upgrades into
//!   `%LOCALAPPDATA%\Programs\ReUI`, adds the Start menu entry, the Apps & features entry and
//!   start with Windows, then launches ReUI.
//! - `reui-setup.exe --uninstall [--quiet]` removes all of it (and, if you say so, your
//!   data). Apps & features runs the installed copy this way.
//! - `reui-setup.exe --extract DIR` writes the embedded programs to `DIR` and exits
//!   (verifies a packaged setup without installing anything).
//!
//! The programs are embedded at build time when `REUI_PAYLOAD_DIR` is set (see
//! `payload_gen.rs`, `scripts/package.ps1`); otherwise the ones next to this exe are used.

#![windows_subsystem = "windows"]

mod install;
mod layout;
mod payload;
#[cfg_attr(not(test), allow(dead_code))] // `generate` runs in build.rs
mod payload_gen;
mod registry;
mod shortcut;
mod system;
mod ui;
mod uninstall;

#[cfg(test)]
mod flow_tests;

use std::path::PathBuf;

use crate::layout::{Layout, APP, VERSION};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    let quiet = has("--quiet");

    let result = if let Some(i) = args.iter().position(|a| a == "--extract") {
        extract(args.get(i + 1).map(PathBuf::from))
    } else if has("--uninstall") {
        run_uninstall(quiet)
    } else {
        run_install(install::Options { no_autostart: has("--no-autostart"), no_launch: has("--no-launch") }, quiet)
    };
    if let Err(message) = result {
        eprintln!("reui-setup: {message}");
        if !quiet {
            ui::error(&message);
        }
        std::process::exit(1);
    }
}

fn current_exe() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|e| format!("Couldn't find this setup's own file: {e}"))
}

fn extract(dir: Option<PathBuf>) -> Result<(), String> {
    let dir = dir.ok_or("--extract needs a folder")?;
    let files = payload::embedded();
    payload::check(&files)?;
    payload::extract(&files, &dir).map_err(|e| format!("Couldn't write to {}: {e}", dir.display()))?;
    println!("extracted {} files ({} bytes) to {}", files.len(), payload::total_size(&files), dir.display());
    Ok(())
}

fn run_install(options: install::Options, quiet: bool) -> Result<(), String> {
    let exe = current_exe()?;
    let files = payload::resolve(exe.parent().ok_or("no setup folder")?)?;
    let layout = Layout::for_user()?;
    let outcome = install::install(&layout, &files, &exe, options, &mut system::Real)?;
    println!("{} ReUI {VERSION} in {}", if outcome.upgrade { "updated" } else { "installed" }, layout.install_dir.display());
    if !quiet {
        let what = if outcome.upgrade { format!("{APP} is updated to {VERSION}.") } else { format!("{APP} {VERSION} is installed.") };
        ui::info(&what);
    }
    Ok(())
}

fn run_uninstall(quiet: bool) -> Result<(), String> {
    let exe = current_exe()?;
    // Apps & features may start us inside the install folder; don't hold it open.
    _ = std::env::set_current_dir(std::env::temp_dir());
    let layout = Layout::for_user()?;
    let delete_data = !quiet && ui::ask("Also delete your ReUI data?\n\nClipboard history, settings and caches. Choose No to keep them for a later install.");
    let steps = uninstall::plan(&uninstall::hooked_agents(&layout.home), delete_data);
    let failures = uninstall::run(&layout, &steps, &exe, &mut system::Real);
    if failures.is_empty() {
        println!("uninstalled ReUI");
        Ok(())
    } else {
        Err(format!("ReUI is uninstalled, except:\n\n{}", failures.join("\n")))
    }
}
