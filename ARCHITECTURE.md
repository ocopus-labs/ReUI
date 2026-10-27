# Architecture

Where things live. [PLAN.md](PLAN.md) explains why, [DESIGN.md](DESIGN.md) covers the look, and the PRDs in [docs/](docs/) cover each feature.

## Processes

```
reui-hub.exe     owns all state; feature modules; serves surfaces over a named pipe
  └ reui-island.exe   the dynamic island (a surface); restarted by the hub if it crashes
reui-bridge.exe  run by coding agents' hooks (Claude Code, Codex, OpenCode); forwards to the hub
```

- **Hub → island:** a semantic view model (`reui-protocol`). The island decides how it looks.
- **Island → hub:** actions (clicks, keys) and module commands (tray).

## Crates

| Crate | Job |
|---|---|
| `reui-protocol` | Wire types: `Activity`, presentations (compact, minimal, expanded), rows, priorities, actions; postcard framing |
| `reui-design` | `design/tokens.json` compiled into constants at build time |
| `reui-hub` | Feature modules and the hub that orders their activities |
| `reui-island` | The island window: Windows.UI.Composition + Direct2D/DirectWrite |
| `reui-bridge` | Hook entry point for coding agents, and the hook installer |

## `reui-hub/src`

```
main.rs            flags (--no-agents, --no-media, --no-clipboard, --no-osd, …), module list
hub.rs             activities, ordering, ModuleContext (publish, retract, focus, image store)
ipc.rs             the pipe: snapshots + images to surfaces, actions/commands/hooks back
island.rs          launches and supervises reui-island
bridge_sync.rs     keeps the installed reui-bridge in step with this build
modules/
  mod.rs           Module trait, start, supervise (restart on error/panic)
  audio/           shared Core Audio: devices, volume, outputs, per-app volume, change events
  demo.rs
  agent/           Claude Code / Codex / OpenCode sessions; view; token usage
  clipboard/       history: module loop, notch panel, toast, paste stack, view model,
                   settings, secrets, link previews, paste-as transforms, OCR
    os/            the OS side: window thread, hotkeys, target/caret, capture, formats,
                   worker, paste-back
    store/         SQLite: schema/migrations, items, queries (FTS), collections, db thread
  media/           Now Playing: module loop, GSMTC sessions, card, controls, cover/artwork,
                   ranking, lyrics (LRCLIB), player activation, settings
  osd/             volume/brightness HUDs, OS window events, brightness (WMI, DDC/CI),
                   Windows flyout hiding, settings
```

**Pattern:**
- Each feature is a folder. `mod.rs` holds the module loop and its state.
- OS adapters run on their own threads, and blocking calls (COM, SQLite, WMI) never run on the hub thread.
- Pure logic sits in small files with unit tests next to it.

## `reui-island/src`

```
main.rs            window class, message loop, mouse/foreground hooks
ipc.rs             pipe client
settings.rs        visibility modes        tray.rs   tray icon and menu
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

## Data on disk

| Path | What |
|---|---|
| `%APPDATA%\ReUI\settings.json` | island visibility |
| `%APPDATA%\ReUI\clipboard.db`, `clipboard.json` | clipboard history and settings |
| `%APPDATA%\ReUI\media.json`, `osd.json` | music and OSD settings |
| `%LOCALAPPDATA%\ReUI\lyrics\` | lyrics cache |
| `%LOCALAPPDATA%\ReUI\hub.log` | panics |
| `%LOCALAPPDATA%\ReUI\bin\reui-bridge.exe` | the installed hook bridge |

## Conventions

- One job per file, about 500 lines at most.
- Doc comments say why.
- No filler text in the UI.
- New features get a PRD in `docs/` first, then milestones, then a folder in `modules/`.
