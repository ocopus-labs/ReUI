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
| `reui-protocol` | Wire types: `Activity`, presentations (compact, minimal, expanded), rows, priorities, actions; postcard framing. Also `version` (semver comparison for update.json) and `install` (the lines `reui-settings --install-update` prints) |
| `reui-design` | `design/tokens.json` compiled into constants at build time (plus a `Palette` per theme); `look.rs` turns the Appearance settings into a `Look` (palette, radii, insets, the concentric maths) for the island and Settings' previews alike; `layout/` is the island's geometry over that `Look` (compact pill and bubble, the expanded header, every row, the card row, gauges and their sparklines, each card's width and height, the view switcher), pure and tested, which both lay out with; `icon.rs` turns `design/logo/reui.ico` into the icon resource every program's `build.rs` links in; `devices.rs` holds the device drawings' bytes (`design/assets/devices/`), one copy for the hub (which sends them) and Settings' Devices preview |
| `reui-hub` | Feature modules and the hub that orders their activities |
| `reui-island` | The island window: Windows.UI.Composition + Direct2D/DirectWrite |
| `reui-bridge` | Hook entry point for coding agents (`main.rs`; `parents.rs`: its parent process chain, sent for "Open terminal"), the hook installer (`install.rs`), and the OpenCode plugin it installs (`opencode-plugin.js`, tested by `tools/opencode-plugin-test.mjs`) |
| `reui-settings` | The Settings window: Direct2D/DirectWrite in the island's design language, every setting in one place |

## `reui-hub/src`

```
main.rs            flags (--no-agents, --no-media, --no-clipboard, --no-osd, --no-devices, --no-timers,
                   --no-privacy, --no-stats, --no-calendar, …), module list
hub.rs             activities, ordering, ModuleContext (publish, retract, focus, image store)
ipc.rs             the pipe: snapshots + images to surfaces, actions/commands/hooks back
island.rs          launches and supervises reui-island
bridge_sync.rs     keeps the installed reui-bridge (and an installed OpenCode plugin) in step
                   with this build
paths.rs           %APPDATA% / %LOCALAPPDATA% / home, or REUI_CONFIG_DIR's sandbox (own pipe and lock)
modules/
  mod.rs           Module trait, start, supervise (restart on error/panic)
  audio/           shared Core Audio: devices, volume, outputs, per-app volume, change events
  system_events/   shared hidden window: power settings, resume/unlock, volume and phone
                   arrivals, brightness keys (Raw Input), lock keys, Explorer restarts,
                   clock changes, module hotkeys
  hotkey.rs        hotkey settings ("Win+Alt+V") parsed into modifiers + virtual key
  hide.rs          the island's hide shortcut (settings.json `hide_shortcut`, Win+Alt+I) and
                   `toggle-hidden` (`reui-bridge hide`), relayed as HubMsg::ToggleHidden
  destinations.rs  the places the view switcher offers (module, glyph, name, open command),
                   from the running modules and Now Playing's card; the hub sends them as
                   HubMsg::Destinations when they change (pure, tested)
  demo/            --demo: music, a timer, and agent moments (moments.rs: a permission
                   with a body, a 3-step question, a plan) that answer nothing real
  onboarding/      first run (demo card, "Hover me", the welcome), one-time tips and offers
                   (state + queue, views, system: fullscreen/pause, agent processes, Connect);
                   other modules report moments with `onboarding::note`
  agent/           Claude Code / Codex / OpenCode: sessions tracked, published only when
                   they need you; events (a hook event into sessions and moments), queue
                   (the moments in line, paged as one card; the async and blocking
                   PreToolUse duplicates made one), moments (a permission, question, plan,
                   MCP form or link: steps, answers, the field; Codex questions read-only),
                   form (an MCP form being filled in), schema (an MCP requested_schema as
                   fields, or "can't show"), link (an MCP link opened, http(s) only),
                   answer (the reply JSON in each agent's documented shape), card (a
                   moment's card), plan (a plan's title, steps, prose), view (what shows,
                   the session strip, a failed turn's Open terminal), terminal (the
                   bridge's parent chain to the window to bring forward), describe (a
                   tool request: hero, facts, body; secrets masked), settings
                   (coding.json), token usage, status (agents.json, the test event)
  clipboard/       history: module loop, notch panel, toast, paste stack, view model,
                   settings, secrets, link previews, paste-as transforms, OCR
    os/            the OS side: window thread, hotkeys, target/caret, capture, formats,
                   worker, paste-back
    store/         SQLite: schema/migrations, items, queries (FTS), collections, db thread
  media/           Now Playing: module loop, GSMTC sessions, card, controls, cover/artwork,
                   ranking, lyrics (LRCLIB), player activation, settings
  osd/             volume/brightness HUDs, brightness (WMI, DDC/CI), Windows flyout hiding,
                   settings
  devices/         Bluetooth connects and batteries (watchers, containers, PnP battery and
                   Device ID reads, AirPods ads with colour and lid), anti-spam gate, low
                   battery, controllers, laptop power and time to full, drives and Eject,
                   phones, overview, OSD hand-off, settings; art.rs looks up ReUI's device
                   drawings by kind or family (design/assets/devices/, sent as Art::Logo)
    catalog/       exact models, offline: Device ID parsing (ids), Apple/Beats and
                   controller tables with a source per row, chip vendors to refuse, the
                   lookup order (resolve; pure, tested); families and kinds
  timers/          countdowns, focus sessions, stopwatch: launcher field parser, state
                   machines (fake-clock tests), timer-state.json, finish sound, Do Not
                   Disturb (WNF read, IQuietHoursSettings, checked), views, settings
  privacy/         mic, camera, screen and location in use: the consent store (consent: what's
                   live, pure; alive: which uses still have their process, pure; store: the
                   reui-privacy thread, RegNotifyChangeKeyValue + process waits), tracker (screen
                   settle, short uses linger, Briefs; fake-clock tests), views, mute via the
                   shared audio module, settings (privacy.json)
  stats/           system stats card (CPU, GPU, memory, disk, network, battery draw), sampled
                   only while open: card (open/closed, deadlines; pure, tested), sampler (its
                   own thread, 1 Hz), os (GetSystemTimes, GlobalMemoryStatusEx, GetIfTable2),
                   pdh (GPU engines and memory, disk; English counter names), maths (deltas,
                   counted interfaces, the busiest GPU engine, number text; tested), history
                   (the open card's last minute per gauge, sustained-high tone; tested), view,
                   settings (stats.json)
  calendar/        the next meeting: secret ICS links (fetch: conditional GET, backoff, the
                   DPAPI-encrypted last copy; network: connectivity hints) and Windows' own
                   store (windows_store, opt-in), read by ics/ (lines, time + VTIMEZONE,
                   rrule, zones via Windows' ICU; feeds into occurrences; tested), links
                   (meeting-link detection, Safe Links unwrapped; pure, tested), schedule
                   (countdown, start alert, the day's card, the next wake-up; fake clock),
                   views, settings (calendar.json, links DPAPI-sealed: dpapi.rs), status
                   (each source's last refresh, for Settings)
  updates/         daily update check, the update card (an Alert: Later / Install, then
                   download progress, installing or why it failed), "Updated" brief:
                   update.json + offer/snooze rules, card phases (pure, tested), views,
                   reui-settings runs (--check-updates, --install-update read line by line)
```

**Pattern:**
- Each feature is a folder. `mod.rs` holds the module loop and its state.
- OS adapters run on their own threads, and blocking calls (COM, SQLite, WMI) never run on the hub thread.
- Pure logic sits in small files with unit tests next to it.

## `reui-island/src`

```
main.rs            window class, message loop, mouse/foreground hooks
ipc.rs             pipe client
settings.rs        visibility modes, pause, the look's keys, watching settings.json
tray.rs            tray icon, short menu (Install update while one is known)
theme.rs           Windows' app mode (Auto) and accent, re-read on "ImmersiveColorSet"
order.rs           which activity is in the pill and which in the bubble
place.rs           which display (monitor setting) and the window rect at its DPI
fullscreen.rs      when to hide: D3D, presentation, busy, a window covering the display
desktop.rs         whether the desktop (not an app) is in front, for "Always show on the desktop"
drag_guard.rs      no hover intent while a drag from elsewhere passes over
images.rs          encoded images from the hub
pill/              the island's behaviour
  mod.rs           state machine, snapshot apply, render orchestration
  link.rs          the hub link: snapshots, images and focus requests as they arrive
  setup.rs         builds the composition tree for the Look (shape, hairline, notch ears);
                   a new look rebuilds it (WM_LOOK), like a DPI change
  input.rs         clicks, wheel, drag, hover
  summon.rs        hotkey-summoned cards, keyboard capture, search
  idle.rs          the idle pill: the time at rest (`idle_clock`), the date and time on a
                   peek, a click opening the row of places
  switcher.rs      the view switcher: places, when it shows, its tabs placed and drawn,
                   click / wheel / Ctrl+Tab to another place, a waiting decision's bubble beside it
  visibility.rs    size springs, show/hide, reveal, fullscreen, topmost
  timers.rs        tick, clocks, bars, scroll, lyric, level timers
  visuals.rs       level bars, logo
render/            drawing only (no behaviour state); where things go is reui_design::layout
  mod.rs           Renderer, surfaces, layouts, hit testing
  fonts.rs  text.rs  images.rs  elements.rs
  compact.rs  minimal.rs  expanded.rs (strip + header + row layout)
  notch.rs         the Notch style's ears
  idle.rs          the idle pill's faces: the time, the date and time, the row of places
  sheet.rs         test-only: the device-drawing contact sheet (target/devices-art/)
  switcher.rs      the view switcher's tabs and the waiting decision's dot
  rows/            timeline, controls, slider, lyrics, cards, search, tabs, note,
                   context (session strip), choices, block (scrolling text), gauges
                   (gauges with sparklines, facts: System stats), agenda (the calendar's
                   rest of the day)
features/          per-feature island logic
  clipboard.rs  clipboard_keys.rs   panel selection/scroll, keyboard hook
  keys.rs                           the key map of a card that has the keyboard
  choices.rs                        keys in a card that asks something or a moment
                                    (once clicked); the cursor scrolled into view
  music.rs                          scrub, slider, lyric and swipe maths
  visualizer.rs                     per-process audio loopback + FFT
  level.rs                          eased level values
  clock.rs                          how clocks read, and when they next change
  idle.rs                           the idle pill's face (time, date and time, places), the wall
                                    clock and when the minute turns
  switcher.rs                       when the view switcher shows, which tab is selected,
                                    which decision waits beside it
```

**Rules:**
- Rendering draws what it's given.
- Motion runs on short timers that stop once settled. There are no compositor animation loops (see DESIGN.md).

## `reui-settings/src`

```
main.rs            single instance, --page, --welcome, --check-updates and
                   --install-update (no window), REUI_CONFIG_DIR (test sandbox)
window.rs          the window: own caption (no system title bar), hit-testing, DWM corners,
                   per-monitor DPI v2
gpu.rs             D3D11 device, D2D device context, flip swap chain + frame-latency waitable
perf.rs            --perf frame timings
shots.rs           --shots <dir> page screenshots; --backdrop-color, --stages (the
                   README's pictures: tools/readme-images.ps1)
app/               state machine: page, scroll, hover, focus, menus, drags
  input.rs         pointer and keyboard (Tab order, arrows, Space/Enter, Esc)
  motion.rs        springs and eases stepped once per presented frame
  search.rs        sidebar search over every row's title and line
  shot.rs          offscreen rendering for --shots (pages, and stages alone)
ui/                pages as data: blocks, rows, parts (controls)
  layout.rs        where everything goes (pure, tested): rows, controls, scrolling, nav
  stage.rs         each page's island in the user's Look (its size, an open card's header and
                   rows, from reui_design::layout) and its stage's height (pure, tested)
  chrome.rs        sidebar and caption geometry      icons.rs   page icons and tile colours
render/            drawing only: Renderer/Painter primitives, fonts, text (layout cache),
                   shapes, icons, sidebar, frame (caption, scrollbar), page, controls, menu
  preview/         the hero card's island on the top of the screen: mod (stage, placement),
                   parts (shape and notch ears, tiles, rings, buttons, rows, placed by
                   reui_design::layout as the island places them), one file per moment: compact, music, clipboard (+ toast),
                   levels, devices, timers, stats, agents; switcher (under the timer card)
pages/             one file per page (calendar.rs: links sealed with model/calendar.rs): build its rows from State, handle its controls;
                   welcome.rs is the three-step welcome (first run, Replay the tour)
model/             settings files (read, change a key, atomic write, keep unknown keys,
                   never write an invalid file), hotkeys, coding agents (status, test)
updates/           GitHub releases: release.rs (JSON, checksums, notes; pure), net.rs
                   (WinHTTP GET/download, CNG SHA-256), mod.rs (check, download + verify,
                   run the setup, update.json, What's new), install.rs (--install-update:
                   the island's Install, one step per stdout line; REUI_UPDATE_SIMULATE)
wallpaper/         the previews' stage: mod.rs reads the wallpaper (IDesktopWallpaper, else the
                   registry) on a thread, composes the top-centre strip with WIC; fit.rs: where
                   each fit mode puts the image and what lands in the strip (pure, tested)
state.rs           what the pages show and change      hub.rs     pipe client (reload, clear)
keys.rs            hotkey recorder (low-level hook)     system.rs  startup entry, folders, apps
stats.rs           idle cost: CPU and private memory of ReUI's processes
../perf/           harness.ps1 / live.ps1: scripted scroll/hover runs against --perf
```

## Data on disk

| Path | What |
|---|---|
| `%APPDATA%\ReUI\settings.json` | island `visibility`, `show_on_desktop`, `hide_in_fullscreen`, `monitor`, `paused_until` while ReUI is paused, `hidden` while it's hidden with its shortcut (written by the island), `hide_shortcut` (read by the hub), `view_switcher`, `idle_clock`, and the look: `style` (island / notch), `theme` (auto / dark / light), `accent` (reui / windows), `roundness` (0..1), `top_gap` (0..24), `padding` (compact / regular / roomy), `scale` (0.9..1.25) |
| `%APPDATA%\ReUI\onboarding.json` | first run done (`welcomed`), tips shown, offers answered |
| `%APPDATA%\ReUI\clipboard.db`, `clipboard.json` | clipboard history and settings |
| `%APPDATA%\ReUI\media.json`, `osd.json`, `devices.json`, `timers.json`, `privacy.json`, `stats.json`, `coding.json` | music, OSD, Devices, Timers, privacy indicator, System stats and coding agent settings (`coding.json` is read by all three agent modules) |
| `%APPDATA%\ReUI\calendar.json` | Calendar: `lead_min`, `windows`, `sources` (each link DPAPI-sealed, shown by `host`) |
| `%LOCALAPPDATA%\ReUI\calendar\` | each calendar's last good copy (`<id>.cache`, DPAPI-encrypted) and `status.json` (each source's last refresh, read by Settings) |
| `%APPDATA%\ReUI\timer-state.json` | running and paused timers, a focus session, the stopwatch, and whether ReUI turned Do Not Disturb on |
| `%LOCALAPPDATA%\ReUI\lyrics\` | lyrics cache |
| `%LOCALAPPDATA%\ReUI\hub.log` | panics |
| `%LOCALAPPDATA%\ReUI\bin\reui-bridge.exe` | the installed hook bridge |
| `%LOCALAPPDATA%\ReUI\agents.json` | each coding agent's last hook event (hub's agent module, ≤ 1 write / 10 s per agent; Settings' status line) |
| `%LOCALAPPDATA%\ReUI\update.json` | the last update check and what it found (Settings), the last version run, `updated_from`, and the version "Later" put off with `snoozed_until` (hub's updates module); the island's tray reads it when its menu opens |
| `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` value `ReUI` | start with Windows (the Settings window's toggle) |

## Conventions

- One job per file, about 500 lines at most.
- Doc comments say why.
- No filler text in the UI.
- New features get a PRD in `docs/` first, then milestones, then a folder in `modules/`.
