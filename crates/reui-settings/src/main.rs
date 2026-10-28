//! ReUI Settings: one window for every setting, drawn in the island's own design language
//! (black material, concentric capsules, eyebrow → hero → detail) with Direct2D and
//! DirectWrite. It reads and writes the same JSON files the island and the hub modules
//! read, then tells the running hub to reload (the island watches its file itself).
//!
//! Single instance: launching it again brings the open window to the front. Flags:
//! `--page <name>` opens on a page (general, now, clipboard, osd, devices, coding, privacy,
//! about). `--welcome` opens the three welcome steps instead (first run, or "Replay the
//! tour"). `--check-updates` opens no window: it asks GitHub for the latest release, notes
//! what it found in `update.json` and exits (the hub runs it once a day).
//! `REUI_CONFIG_DIR=<dir>` keeps every file it touches in that folder and talks to no
//! running ReUI (for testing). `--perf` logs frame times to `settings-perf.log` in the local
//! folder; `--shots <dir>` saves every page as a PNG without showing the window.
//!
//! Files: `window.rs` (frame, caption, DPI, the message loop), `gpu.rs` (D3D11, Direct2D,
//! the flip-model swap chain), `app/` (state machine, input, motion, search), `render/`
//! (drawing, island previews), `ui/` (blocks, rows, layout maths), `pages/` (one per page), `model/`
//! (settings files, hotkeys, agents), `state.rs`, `hub.rs` (pipe client), `keys.rs`
//! (hotkey recorder), `system.rs` (startup entry, folders, apps, displays), `stats.rs`
//! (idle cost), `updates/` (GitHub releases: check, download, verify, What's new).

#![windows_subsystem = "windows"]

mod app;
mod hub;
mod keys;
mod model;
mod gpu;
mod pages;
mod paths;
mod perf;
mod render;
mod shots;
mod state;
mod stats;
mod system;
mod ui;
mod updates;
mod window;

use windows::core::HSTRING;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};

use crate::pages::Page;
use crate::paths::Paths;
use crate::state::State;

fn main() {
    let paths = Paths::from_env();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--check-updates") {
        updates::check_quietly(&paths);
        return;
    }
    let page = args.iter().position(|a| a == "--page").and_then(|i| args.get(i + 1)).and_then(|p| Page::from_arg(p)).unwrap_or(Page::General);
    let page = if args.iter().any(|a| a == "--welcome") { Page::WelcomeFeatures } else { page };

    if args.iter().any(|a| a == "--perf") {
        perf::init(paths.local.join("settings-perf.log"));
    }
    unsafe { _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    // Held for the life of the process; a second launch finds it and hands over.
    let name = if paths.sandbox { "Local\\ReUI.Settings.Sandbox" } else { "Local\\ReUI.Settings" };
    let _mutex = unsafe { CreateMutexW(None, true, &HSTRING::from(name)) };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS && window::activate_existing(paths.sandbox) {
        return;
    }

    let state = State::new(paths);
    if let Some(dir) = args.iter().position(|a| a == "--shots").and_then(|i| args.get(i + 1)) {
        if let Err(e) = window::shots(state, std::path::Path::new(dir)) {
            eprintln!("settings: screenshots failed: {e}");
        }
        return;
    }
    if let Err(e) = window::create(state, page, true) {
        eprintln!("settings: can't open the window: {e}");
        return;
    }
    window::run();
}
