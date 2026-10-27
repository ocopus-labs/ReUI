# Architecture

Where things live. [PLAN.md](PLAN.md) explains why, [DESIGN.md](DESIGN.md) covers the look, and the PRDs in [docs/](docs/) cover each feature.

## Processes

```
reui-hub.exe     owns all state; feature modules; serves surfaces over a named pipe
  └ reui-island.exe   the dynamic island (a surface); restarted by the hub if it crashes
reui-bridge.exe  run by coding agents' hooks (Claude Code, Codex, OpenCode); forwards to the hub
reui-settings.exe  the Settings window, opened from the tray; single instance
```

- **Hub → island:** a semantic view model (`reui-protocol`). The island decides how it looks.
- **Island → hub:** actions (clicks, keys) and module commands (tray).
- **Settings → files → modules:** the Settings window writes the same JSON files the
  modules read, then connects as a `Settings` surface and sends the module `reload`. The
  island watches its own folder for `settings.json` (visibility, pause) instead.

## Crates

| Crate | Job |
|---|---|
| `reui-protocol` | Wire types: `Activity`, presentations (compact, minimal, expanded), rows, priorities, actions; postcard framing |
| `reui-design` | `design/tokens.json` compiled into constants at build time |
| `reui-hub` | Feature modules and the hub that orders their activities |
| `reui-island` | The island window: Windows.UI.Composition + Direct2D/DirectWrite |
| `reui-bridge` | Hook entry point for coding agents, and the hook installer |
| `reui-settings` | The Settings window: Direct2D/DirectWrite in the island's design language, every setting in one place |

## `reui-hub/src`

```
main.rs            flags (--no-agents, --no-media, --no-clipboard, --no-osd, --no-devices, …), module list
hub.rs             activities, ordering, ModuleContext (publish, retract, focus, image store)
ipc.rs             the pipe: snapshots + images to surfaces, actions/commands/hooks back
island.rs          launches and supervises reui-island
bridge_sync.rs     keeps the installed reui-bridge in step with this build
modules/
  mod.rs           Module trait, start, supervise (restart on error/panic)
  audio/           shared Core Audio: devices, volume, outputs, per-app volume, change events
  system_events/   shared hidden window: power settings, resume/unlock, volume and phone
                   arrivals, brightness keys (Raw Input), lock keys, Explorer restarts
  demo.rs
  agent/           Claude Code / Codex / OpenCode sessions; view; token usage
  clipboard/       history: module loop, notch panel, toast, paste stack, view model,
                   settings, secrets, link previews, paste-as transforms, OCR
    os/            the OS side: window thread, hotkeys, target/caret, capture, formats,
                   worker, paste-back
    store/         SQLite: schema/migrations, items, queries (FTS), collections, db thread
  media/           Now Playing: module loop, GSMTC sessions, card, controls, cover/artwork,
                   ranking, lyrics (LRCLIB), player activation, settings
  osd/             volume/brightness HUDs, brightness (WMI, DDC/CI), Windows flyout hiding,
                   settings
  devices/         Bluetooth connects and batteries (watchers, containers, PnP battery,
                   AirPods ads), anti-spam gate, low battery, controllers, laptop power
                   and time to full, drives and Eject, phones, overview, OSD hand-off,
                   settings
```

**Pattern:**
- Each feature is a folder. `mod.rs` holds the module loop and its state.
- OS adapters run on their own threads, and blocking calls (COM, SQLite, WMI) never run on the hub thread.
- Pure logic sits in small files with unit tests next to it.

## `reui-island/src`

```
main.rs            window class, message loop, mouse/foreground hooks
ipc.rs             pipe client
settings.rs        visibility modes, pause, watching settings.json   tray.rs   tray icon, short menu
order.rs           which activity is in the pill and which in the bubble
images.rs          encoded images from the hub
pill/              the island's behaviour
  mod.rs           state machine, snapshot apply, render orchestration
  setup.rs         builds the composition tree
  input.rs         clicks, wheel, drag, hover
  summon.rs        hotkey-summoned cards, keyboard capture, search
  visibility.rs    size springs, show/hide, reveal, fullscreen, topmost
  timers.rs        tick, bars, scroll, lyric, level timers
  visuals.rs       level bars, logo
render/            drawing only (no behaviour state)
  mod.rs           Renderer, surfaces, layouts, hit testing
  fonts.rs  text.rs  images.rs  elements.rs
  compact.rs  minimal.rs  expanded.rs (header + row layout)
  rows/            timeline, controls, slider, lyrics, cards, search, tabs, note
features/          per-feature island logic
  clipboard.rs  clipboard_keys.rs   panel selection/scroll, keyboard hook
  music.rs                          scrub, slider, lyric and swipe maths
  visualizer.rs                     per-process audio loopback + FFT
  level.rs                          eased level values
```

**Rules:**
- Rendering draws what it's given.
- Motion runs on short timers that stop once settled. There are no compositor animation loops (see DESIGN.md).

## `reui-settings/src`

```
main.rs            single instance, --page, REUI_CONFIG_DIR (test sandbox)
window.rs          the window: own caption (no system title bar), hit-testing, DWM corners,
                   per-monitor DPI v2
app/               state machine: page, scroll, hover, focus, menus, drags, animations
  input.rs         pointer and keyboard (Tab order, arrows, Space/Enter, Esc)
ui/                pages as data: blocks, rows, parts (controls)
  layout.rs        where everything goes (pure, tested): rows, controls, scrolling, nav
render/            drawing only: Renderer/Painter primitives, fonts, frame (nav, caption,
                   scrollbar), page, controls, menu
pages/             one file per page: build its rows from State, handle its controls
model/             settings files (read, change a key, atomic write, keep unknown keys,
                   never write an invalid file), hotkeys, coding agents
state.rs           what the pages show and change      hub.rs     pipe client (reload, clear)
keys.rs            hotkey recorder (low-level hook)     system.rs  startup entry, folders, apps
stats.rs           idle cost: CPU and private memory of ReUI's processes
```

## Data on disk

| Path | What |
|---|---|
| `%APPDATA%\ReUI\settings.json` | island visibility, and `paused_until` while ReUI is paused |
| `%APPDATA%\ReUI\clipboard.db`, `clipboard.json` | clipboard history and settings |
| `%APPDATA%\ReUI\media.json`, `osd.json`, `devices.json` | music, OSD and Devices settings |
| `%LOCALAPPDATA%\ReUI\lyrics\` | lyrics cache |
| `%LOCALAPPDATA%\ReUI\hub.log` | panics |
| `%LOCALAPPDATA%\ReUI\bin\reui-bridge.exe` | the installed hook bridge |
| `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` value `ReUI` | start with Windows (the Settings window's toggle) |

## Conventions

- One job per file, about 500 lines at most.
- Doc comments say why.
- No filler text in the UI.
- New features get a PRD in `docs/` first, then milestones, then a folder in `modules/`.
