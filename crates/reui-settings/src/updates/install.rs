//! `--install-update`: the island's Install, without a window. The hub runs it and reads
//! its stdout (the lines are [`InstallStep`]s) to drive the card. It installs the release
//! the last check stored in `update.json`, through the same [`fetch`] and
//! [`run_installer`] as the About page, so both verify the same way.
//!
//! In a sandbox (`REUI_CONFIG_DIR`) nothing is ever installed, as on the About page.
//! `REUI_UPDATE_SIMULATE=ok|fail` there plays the steps out instead, with no network: it
//! is how the island's card is tested without a published release.

use std::io::Write;
use std::time::Duration;

use reui_protocol::install::InstallStep;

use super::{fetch, read_state, run_installer, stored_release, FetchStep};
use crate::paths::Paths;

/// One line per step; the hub reads them as they come, so each is flushed.
fn say(step: InstallStep) {
    let mut out = std::io::stdout().lock();
    _ = writeln!(out, "{step}");
    _ = out.flush();
}

/// Runs the install and returns the process's exit code: 0 once the setup has started.
pub fn install_quietly(paths: &Paths) -> i32 {
    match install(paths) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("settings: install: {e}");
            say(InstallStep::Error(e));
            1
        }
    }
}

fn install(paths: &Paths) -> Result<(), String> {
    if paths.sandbox {
        if let Some(outcome) = std::env::var("REUI_UPDATE_SIMULATE").ok().filter(|v| !v.is_empty()) {
            return simulate(&outcome);
        }
    }
    let release = stored_release(&read_state(paths)).ok_or("No newer version is known")?;
    let total = release.setup.as_ref().map_or(0, |s| s.size);
    say(InstallStep::Progress(0.0));
    let path = fetch(&release, |step| match step {
        FetchStep::Downloaded(done) if total > 0 => say(InstallStep::Progress(done as f32 / total as f32)),
        FetchStep::Downloaded(_) => {}
        FetchStep::Verifying => say(InstallStep::Verifying),
    })?;
    if paths.sandbox {
        return Err(format!("Downloaded and verified {}; the sandbox doesn't install it", path.display()));
    }
    say(InstallStep::Installing);
    run_installer(&path)
}

/// The steps of an install, slowly enough to watch: `ok` ends as a started setup would,
/// anything else in a checksum failure.
fn simulate(outcome: &str) -> Result<(), String> {
    const STEPS: u32 = 40;
    for i in 0..=STEPS {
        say(InstallStep::Progress(i as f32 / STEPS as f32));
        std::thread::sleep(Duration::from_millis(200));
    }
    say(InstallStep::Verifying);
    std::thread::sleep(Duration::from_millis(500));
    if outcome != "ok" {
        return Err("The download doesn't match the release's checksum".into());
    }
    say(InstallStep::Installing);
    Ok(())
}
