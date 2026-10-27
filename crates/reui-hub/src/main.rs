//! ReUI hub: owns all module state, decides what's in front, and serves it to surfaces.
//!
//! Flags: `--demo` enables the demo module, `--no-agents` / `--no-media` disable the coding
//! agent (Claude Code, Codex, OpenCode) / Now Playing modules, `--no-link-previews` keeps
//! clipboard links offline, `--no-island` doesn't launch the island
//! (run `reui-island` yourself, e.g. under a debugger).

mod bridge_sync;
mod hub;
mod ipc;
mod island;
mod modules;

use anyhow::Result;

use crate::hub::Hub;
use crate::modules::Module;

// One thread is plenty: modules are event-driven and the hub mostly sleeps.
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    log_panics();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);

    // WinRT (media sessions) and WIC (artwork) need COM on this thread; the runtime is
    // single-threaded, so every module runs here.
    unsafe { windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED)? };
    bridge_sync::run();
    let hub = Hub::new();
    let server = ipc::Server::bind(&reui_protocol::pipe_name())?;

    let mut enabled: Vec<Box<dyn Module>> = Vec::new();
    if !has("--no-agents") {
        for profile in [&modules::agent::CLAUDE, &modules::agent::CODEX, &modules::agent::OPENCODE] {
            enabled.push(Box::new(modules::agent::Agent(profile)));
        }
    }
    if !has("--no-clipboard") {
        enabled.push(Box::new(modules::clipboard::ClipboardHistory { link_previews: !has("--no-link-previews") }));
    }
    if !has("--no-media") {
        enabled.push(Box::new(modules::media::NowPlaying));
    }
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

/// Panics also go to `%LOCALAPPDATA%\ReUI\hub.log`: the hub usually runs without a console,
/// and a module that dies quietly (clipboard capture, say) is otherwise invisible.
fn log_panics() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(dir) = std::env::var_os("LOCALAPPDATA").map(|d| std::path::PathBuf::from(d).join("ReUI")) {
            use std::io::Write;
            _ = std::fs::create_dir_all(&dir);
            if let Ok(mut log) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("hub.log")) {
                let thread = std::thread::current().name().unwrap_or("?").to_string();
                _ = writeln!(log, "{} panic on {thread}: {info}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"));
            }
        }
        default(info);
    }));
}
