# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

ReUI is a dynamic island for Windows (later, a Windows shell), written in Rust. Read `ARCHITECTURE.md` for where things live, `PLAN.md` for why, `DESIGN.md` for the look, and `docs/PRD-*.md` for each feature. `docs/guide.md` is the user-facing guide to every feature and shortcut; `README.md` is the product page; `docs/development.md` covers building, flags, files on disk and the coding-agent hooks.

## Commands

```
cargo build --release                       # all programs into target\release
cargo test --workspace                      # what CI runs before packaging
cargo test -p reui-hub timers               # one crate, tests whose path matches "timers"
cargo test -p reui-hub -- --exact <module::path::tests::name>   # a single test
target\release\reui-hub.exe --demo          # hub + island with demo activities
pwsh scripts/package.ps1 -OutDir dist       # setup exe + zip, as .github/workflows/release.yml does on a v* tag
```

- Windows only (MSVC toolchain). Debug builds of the hub keep a console for logs; release builds have none.
- **Run test instances sandboxed:** `REUI_CONFIG_DIR=<dir>` gives the hub, island and Settings their own files, named pipe and single-instance lock, so they run beside the user's installed ReUI without touching its data.
- Hub flags: `--demo`, `--no-island` (start `reui-island.exe` yourself, e.g. under a debugger), `--no-agents|--no-media|--no-clipboard|--no-osd|--no-devices|--no-timers|--no-privacy|--no-stats|--no-calendar`, `--no-updates`, `--no-onboarding`, `--first-run`. Full list in the doc comment at the top of `crates/reui-hub/src/main.rs`.
- Settings window: `reui-settings.exe --page <name>`, `--shots <dir>` (every page as PNG, no window; `--backdrop-color #RRGGBB` instead of the wallpaper, `--stages` only the island stages), `--perf`. `tools/readme-images.ps1` renders the README's pictures with them into `docs/images`. Perf harnesses: `crates/reui-settings/perf/harness.ps1`, `live.ps1`.
- `node tools/opencode-plugin-test.mjs` runs the OpenCode plugin against a fake OpenCode and bridge (no OpenCode needed).
- `tools/claude-states.ps1` drives a fake Claude Code session through every state via `reui-bridge` and captures the island into one review sheet (needs the hub running).
- A tagged release needs a `## <version>` entry in `CHANGELOG.md` and the tag must match the `reui-hub` crate version.

## Architecture

Separate processes talking over one per-user named pipe (postcard frames, types in `reui-protocol`):

- **`reui-hub`** owns all state. Each feature is a `Module` (`crates/reui-hub/src/modules/<feature>/`, registered in `main.rs`) that publishes activities with `ctx.publish(key, priority, View { accent, compact, minimal, expanded })`, retracts them, and handles inputs from `ctx.next()`. The hub orders activities by priority (decision > alert > ongoing > passive) and serves snapshots. Modules are supervised (restarted on error/panic).
- **`reui-island`** is a surface: it renders whatever the hub puts in front (Windows.UI.Composition + Direct2D/DirectWrite) and sends actions back. The hub launches and supervises it.
- **`reui-settings`** is the Settings window. It writes the same JSON files the modules read (`%APPDATA%\ReUI\*.json`), then connects as a `Settings` surface and tells the module to `reload`. The island watches its own `settings.json` instead.
- **`reui-bridge`** is run by coding agents' hooks (Claude Code, Codex, OpenCode); forwards events to the hub and waits only on permission requests. Every failure must mean "no decision" (fail open), it prints nothing but a decision and always exits 0. It also installs/uninstalls hooks; the hub keeps `%LOCALAPPDATA%\ReUI\bin\reui-bridge.exe` in step with its own build (`bridge_sync.rs`).
- **`reui-setup`** is the per-user installer; `build.rs` embeds the programs from `REUI_PAYLOAD_DIR` (set by `package.ps1`).
- **`reui-design`** compiles `design/tokens.json` into constants at build time (`tokens::color::label::PRIMARY`).

## Rules that span files

- **Modules describe meaning, never appearance.** The protocol carries priority, regions, element kinds, tones, button roles and an accent colour. Sizes, fonts and theme colours live only in `design/tokens.json`, the island's geometry in `reui-design/src/layout/` (shared with Settings' previews), and the drawing in the island's `render/`. A redesign should touch tokens and render code, never a module or the protocol.
- **Idle costs nothing.** Event-driven adapters only (polling only where no OS event exists). No compositor animation loops: motion runs on short timers that stop once settled; progress (timelines, clocks) is interpolated in the surface from a start time, not pushed every tick.
- Blocking calls (COM, SQLite, WMI) never run on the hub thread; OS adapters get their own threads. The hub runs a single-threaded tokio runtime.
- Pure logic goes in small files with unit tests beside it (state machines use fake clocks). One job per file, about 500 lines at most. Doc comments say why.
- The island never takes focus and is click-through outside the pill.
- No filler text in the UI (no "click to…", counts, hotkey hints).
- New features get a PRD in `docs/` first, then milestones, then a folder in `modules/`. Keep `ARCHITECTURE.md`'s file map, `docs/development.md` (flags, files on disk), `docs/guide.md` and the README's feature list current when adding modules, flags or files on disk.
