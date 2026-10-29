//! The updates module's runs of `reui-settings.exe`, which owns all the networking: the
//! daily check, the install (whose printed steps come back as [`Event`]s), and About.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use reui_protocol::install::InstallStep;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

const CHECK_TIMEOUT: Duration = Duration::from_secs(120);
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn settings_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.with_file_name("reui-settings.exe");
    exe.exists().then_some(exe)
}

/// Opens Settings on About.
pub fn open_about() {
    if let Some(exe) = settings_exe() {
        _ = std::process::Command::new(exe).args(["--page", "about"]).spawn();
    }
}

/// Runs `reui-settings --check-updates` and waits for it.
pub async fn run_check() {
    let Some(exe) = settings_exe() else { return };
    let child = tokio::process::Command::new(exe).arg("--check-updates").creation_flags(CREATE_NO_WINDOW).kill_on_drop(true).spawn();
    match child {
        Ok(mut child) => {
            if tokio::time::timeout(CHECK_TIMEOUT, child.wait()).await.is_err() {
                eprintln!("updates: the check took too long");
            }
        }
        Err(e) => eprintln!("updates: can't run the check: {e}"),
    }
}

/// What the install reports.
#[derive(Debug)]
pub enum Event {
    Step(InstallStep),
    /// It ended; `true` for exit code 0 (the setup has started).
    Exited(bool),
}

/// Starts `reui-settings --install-update` and forwards its lines as they're printed, then
/// how it ended. The reading runs on its own task, so the module's loop only wakes when a
/// line arrives.
pub fn start_install() -> Result<mpsc::UnboundedReceiver<Event>, String> {
    let exe = settings_exe().ok_or("reui-settings.exe is missing")?;
    let mut child = tokio::process::Command::new(exe)
        .arg("--install-update")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Can't start the update ({e})"))?;
    let stdout = child.stdout.take().ok_or("The update has no output")?;
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(step) = InstallStep::parse(&line) {
                _ = tx.send(Event::Step(step));
            }
        }
        let success = child.wait().await.is_ok_and(|status| status.success());
        _ = tx.send(Event::Exited(success));
    });
    Ok(rx)
}
