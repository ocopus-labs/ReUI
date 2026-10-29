//! ReUI hub: owns all module state, decides what's in front, and serves it to surfaces.
//!
//! Flags: `--demo` enables the demo module, `--no-agents` / `--no-media` / `--no-osd` /
//! `--no-devices` / `--no-timers` / `--no-privacy` / `--no-stats` / `--no-calendar` / `--no-shelf` disable the coding agent
//! (Claude Code, Codex, OpenCode) / Now Playing / system OSD / Devices / Timers / privacy
//! indicator / System stats / Calendar / file shelf modules, `--no-link-previews` keeps clipboard links offline,
//! `--no-island` doesn't launch the island (run `reui-island` yourself, e.g. under a
//! debugger). `--no-updates` never looks for updates or mentions them. Clipboard, Now
//! Playing, Devices, Timers, the privacy indicators, System stats, Calendar and the shelf also stay off when their
//! file says `"enabled": false` (Settings, the welcome). `--no-onboarding` skips the first
//! run and one-time tips; `--first-run` replays the demo and welcome. `--restore-flyout` only
//! puts back Windows' volume flyout if a crashed run left it hidden, then exits (the
//! uninstaller). `REUI_CONFIG_DIR=<dir>` keeps every file in that folder and runs beside the
//! real ReUI (own pipe, own single-instance lock), for testing.
//!
//! One hub per user: launching it again (the Start menu entry) opens Settings and exits.

// Release builds start from the Start menu / at login: no console window. Dev builds keep
// theirs for the logs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bridge_sync;
mod hub;
mod ipc;
mod island;
mod modules;
mod paths;
mod single_instance;

use anyhow::Result;

use crate::hub::Hub;
use crate::modules::Module;

// One thread is plenty: modules are event-driven and the hub mostly sleeps.
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    log_panics();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);

    if has("--restore-flyout") {
        modules::osd::recover_flyout();
        return Ok(());
    }
    let Some(_instance) = single_instance::acquire() else {
        single_instance::open_settings(&std::env::current_exe()?);
        return Ok(());
    };

    // WinRT (media sessions) and WIC (artwork) need COM on this thread; the runtime is
    // single-threaded, so every module runs here.
    unsafe { windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED)? };
    bridge_sync::run();
    let hub = Hub::new();
    let server = ipc::Server::bind(&reui_protocol::pipe_name())?;

    let mut enabled: Vec<Box<dyn Module>> = Vec::new();
    // First, so a new user's visibility default is written before anything reads it.
    if !has("--no-onboarding") {
        enabled.push(Box::new(modules::onboarding::Onboarding::new(!has("--no-agents"), has("--first-run"))));
    }
    if !has("--no-agents") {
        for profile in [&modules::agent::CLAUDE, &modules::agent::CODEX, &modules::agent::OPENCODE] {
            enabled.push(Box::new(modules::agent::Agent(profile)));
        }
    }
    if !has("--no-clipboard") && feature_on("clipboard.json") {
        enabled.push(Box::new(modules::clipboard::ClipboardHistory { link_previews: !has("--no-link-previews") }));
    }
    if !has("--no-media") && feature_on("media.json") {
        enabled.push(Box::new(modules::media::NowPlaying));
    }
    if has("--no-osd") {
        // Still put back Windows' flyout if an earlier run died with it hidden.
        tokio::task::spawn_blocking(modules::osd::recover_flyout);
    } else {
        enabled.push(Box::new(modules::osd::SystemOsd));
    }
    if !has("--no-devices") && feature_on("devices.json") {
        enabled.push(Box::new(modules::devices::Devices));
    }
    if !has("--no-timers") && feature_on("timers.json") {
        enabled.push(Box::new(modules::timers::TimersModule));
    }
    if !has("--no-calendar") && feature_on("calendar.json") {
        enabled.push(Box::new(modules::calendar::Calendar));
    }
    if !has("--no-privacy") && feature_on("privacy.json") {
        enabled.push(Box::new(modules::privacy::Privacy));
    }
    if !has("--no-stats") && feature_on("stats.json") {
        enabled.push(Box::new(modules::stats::SystemStats));
    }
    if !has("--no-shelf") && feature_on("shelf.json") {
        enabled.push(Box::new(modules::shelf::FileShelf));
    }
    if !has("--no-updates") {
        enabled.push(Box::new(modules::updates::Updates));
    }
    // The island's hide shortcut: the island is always there to hide.
    enabled.push(Box::new(modules::hide::HideIsland));
    if has("--demo") {
        enabled.push(Box::new(modules::demo::Demo));
    }
    modules::start(&hub, enabled);
    tokio::spawn(server.run(hub.clone()));
    println!("hub: listening on {}", reui_protocol::pipe_name());

    if has("--no-island") {
        tokio::signal::ctrl_c().await?;
        return Ok(());
    }
    tokio::select! {
        result = island::supervise() => result,
        result = tokio::signal::ctrl_c() => Ok(result?),
    }
}

/// A feature's `enabled` key (default on), read once at start: switching a feature on or
/// off in Settings applies when ReUI restarts.
fn feature_on(file: &str) -> bool {
    let text = std::fs::read_to_string(paths::config().join(file)).unwrap_or_default();
    serde_json::from_str::<serde_json::Value>(text.trim_start_matches('\u{feff}')).ok().and_then(|v| v.get("enabled").and_then(serde_json::Value::as_bool)).unwrap_or(true)
}

/// Panics also go to `%LOCALAPPDATA%\ReUI\hub.log`: the hub usually runs without a console,
/// and a module that dies quietly (clipboard capture, say) is otherwise invisible.
fn log_panics() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        use std::io::Write;
        let dir = crate::paths::local();
        _ = std::fs::create_dir_all(&dir);
        if let Ok(mut log) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("hub.log")) {
            let thread = std::thread::current().name().unwrap_or("?").to_string();
            _ = writeln!(log, "{} panic on {thread}: {info}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"));
        }
        default(info);
    }));
}
