# ReUI Plan: A Unified Dynamic Island for Windows

Oct 27, 2026 · revised from "Music and AI Integration Plan"

ReUI is one unified surface for everything happening on the machine. Music, agents and clipboard are only the first modules. Every feature gets deep integration, and they all share one hub, one island and one performance budget.

## What changed from the previous draft

- **Unified, not a music app.** Every feature is a module that plugs into the same hub and island. The architecture, budgets and island behaviour are defined once for all modules, not per feature.
- **AI means the user's agents, not ours.** We don't run models. The island watches the coding agents the user already runs (Claude Code, Codex CLI, OpenCode, more later). It shows their prompts and permission requests, lets the user answer them from the island, and shows token usage across all of them. The whole on-device model stack (llama.cpp, whisper, sqlite-vec, the model router) is dropped from v1.
- **Clipboard is a v1 feature.** History with rich previews, pinned items and paste-back from the island. UI direction comes from Rohit's mockups.
- **First target: a Windows dynamic island.** Linux adapters stay in the design but ship later. Music keeps the media-session approach from the old plan.
- **Long-term: our own Windows shell.** The island is the first surface of a future explorer.exe replacement, so the architecture keeps the hub separate from the surfaces now (see the last section).

## Architecture

One hub process owns all state. Each surface (island now; dock, taskbar and desktop later) is only a subscriber, and each source is only an adapter.

```
 sources (adapters)                 hub (Rust, one process)            surfaces
 ───────────────────               ─────────────────────────           ────────
 GSMTC media sessions  ──┐        ┌ now-playing service ┐            ┌ island (v1)
 Claude Code hooks     ──┤        │ agent service       │  events    │ dock (later)
 Codex notify/rollouts ──┼──────► │ usage ledger        │ ─────────► │ taskbar (shell)
 OpenCode server/SSE   ──┤        │ clipboard service   │ ◄───────── │ desktop (shell)
 Clipboard listener    ──┘        └ settings, storage   ┘  commands  └
```

- **Hub ↔ surfaces:** one local IPC channel (named pipe on Windows, Unix socket on Linux) carrying typed events and commands. Surfaces never call Windows APIs for data themselves.
- **Hub ↔ agent bridges:** a tiny `reui-bridge` executable the agents call as a hook. It forwards the event over the same pipe and, for permission requests, waits for the user's answer.
- **Storage:** one SQLite file for clipboard history, the usage ledger and settings.
- **Core language:** Rust, as the old plan assumed. It is still an open question; the shell work later makes Rust (or C#) a better fit than Electron.

## Module model

Every feature is a module with the same shape, so adding one never changes the island or the hub core.

- **Adapter(s):** talk to the OS or the external app (GSMTC, hooks, clipboard listener, WinRT APIs).
- **State:** one typed record the module owns in the hub (Player, AgentSession, ClipItem, ...).
- **Activities:** what the module asks the island to show: compact line, expanded card, alert, with a priority.
- **Actions:** commands the island can send back (play/pause, allow/deny, paste).
- **Settings and permissions:** each module can be turned off completely; turned off means its adapters don't run at all, not just hidden.

The island arbitrates between modules: one activity in front, the rest as small indicators, ordered by priority (anything needing a decision > alerts > ongoing activity > passive info). Modules never draw directly.

## Performance architecture

Many modules with deep integration only stay cheap if the cost rules are shared and enforced in the core.

- **Event-driven only.** Adapters subscribe to OS events (GSMTC changed, `WM_CLIPBOARDUPDATE`, hook calls, WinRT watchers). Polling is allowed only where no event exists, with a stated interval and backoff when idle.
- **Lazy modules.** A module's adapter starts on first use or when enabled, and heavy parts (transcript parsing, image thumbnails, lyrics lookups) run off the main thread and only when their card is shown.
- **One hub process, one surface process.** No process per module. Heavy optional work (e.g. a full usage re-scan) runs in a short-lived worker that exits.
- **Coalesced updates.** The hub batches state changes per frame and sends diffs, so a burst of agent events or timeline ticks never floods the island.
- **No perpetual animations.** Every running composition animation makes DWM recompose each frame: measured at 15–30 % of a core on a 199 Hz display (spike 0a). Morphs are short; looping indicators (now-playing bars, "agent working" shimmer) run only while expanded or hovered, otherwise they show a static state.
- **The island idles at zero.** No animation timers, no repaints and no IPC traffic when nothing changes; timeline progress is interpolated in the surface from a start time, not pushed every second.
- **Fullscreen and battery modes.** Pause non-essential modules when a fullscreen app is in front or on battery saver.
- **Measured budgets per module.** Each module has an idle CPU, memory and latency budget set in its spike and checked in CI with a soak test; a module that breaks its budget doesn't ship.

## Feature catalogue

v1 ships the first three; the rest are candidates that fit the same module model. Order after v1 follows Rohit's priorities.

| Module | Integration | Windows source |
| --- | --- | --- |
| **Music / media** (v1) | Now playing for any player, controls, artwork, lyrics | GSMTC |
| **Agents** (v1) | Claude Code, Codex, OpenCode: live status, permission answers, global token usage | Hooks, JSONL transcripts, local servers |
| **Clipboard** (v1) | History, rich previews, pins, paste-back | Clipboard format listener |
| Notifications | Mirror toasts, quick actions, focus-mode summary | `UserNotificationListener` (needs package identity) |
| Calendar and tasks | Next meeting countdown, join button, quick capture | Google/Microsoft/CalDAV sync |
| System OSD | Replace volume, brightness and media-key popups | Core Audio, WMI/monitor APIs |
| Devices | Bluetooth headphones connect with battery level, USB drives, charging | WinRT device watchers |
| Mic, camera and screen-share indicators | Show which app is using them | Capability access manager registry/consent store |
| Timers and focus | Timer, stopwatch, pomodoro as live activities | Internal |
| Downloads and file shelf | Drop files onto the island to hold them, finished-download alerts | Drag-drop, folder watchers |
| System stats | CPU, GPU, RAM, network in the expanded view | PDH counters, sampled only while visible |
| Calls | Teams/Discord/Zoom call state and mute | Per-app integration, spike needed |

## The island on Windows

The island is a top-most, always-on window at the top centre of the screen that morphs between states. The OS-level behaviour matters more than the graphics, so the first spike is about the window, not the content.

| Concern | Approach | Watch out |
| --- | --- | --- |
| Window | Borderless, top-most, `WS_EX_TOOLWINDOW` (no taskbar/Alt-Tab entry), `WS_EX_NOACTIVATE` so clicking it never steals focus from the editor or terminal | Text input in the island (answering an agent) needs focus temporarily; switch styles only while expanded |
| Click-through | Transparent areas pass clicks through; only the pill shape hit-tests | With a web renderer this means updating the window region or toggling `WS_EX_TRANSPARENT` as the shape morphs |
| Placement | Primary monitor by default, per-monitor DPI aware, follows the monitor with the active window as an option | Taskbar on top, multiple monitors with different scaling |
| Fullscreen | Hide or shrink when a fullscreen app or game is in the foreground (`SHQueryUserNotificationState`, plus a foreground-rect check) | Borderless-windowed games don't report as fullscreen |
| Renderer | **Native, Windows.UI.Composition (the Visual Layer) from Rust**, hosted in the Win32 window via `DesktopWindowTarget`. Springs, expression animations, clips and opacity all run inside DWM, so morphs cost our process ~0 CPU and stay smooth when our thread is busy. Content (text, artwork, icons) is drawn with Direct2D/DirectWrite into composition surfaces; layout with `taffy` when it gets complex. **Safety valve:** heavy expanded panels (clipboard history, usage dashboard) may use an on-demand WebView2 that loads when opened and is destroyed when closed, never at idle | We build our own small widget layer (text, buttons, lists, text input). Design iteration is slower than CSS, so keep visual constants in one place |
| Startup | Per-user Run key or Task Scheduler at logon | Later replaced by being part of the shell itself |

**Island states** (final look comes from the UI mockups):

1. **Idle:** small pill, optional clock or status dot.
2. **Compact live activity:** one line, such as now-playing with artwork, "Claude Code is waiting for you", or "Copied image".
3. **Expanded:** hover or click opens the full card: transport controls and lyrics, the agent's permission request with Allow/Deny, clipboard history.
4. **Alert:** a short attention animation when an agent needs permission or finishes a task.

When several activities are live, priority decides what shows: permission request > agent finished > music > clipboard toast. The rest sit behind a small indicator.

## Music

This part is unchanged from the previous plan except for scope: Windows first.

- **Source:** GSMTC (`Windows.Media.Control`) through the `windows` crate. This covers Spotify desktop, browsers, VLC and anything else that registers with system media controls. No accounts and no API quotas.
- **One Player record:** app, title, artist, album, artwork, status, position, length, enabled controls. The island only sees this record.
- **Timeline:** Windows updates position at its own pace, so the hub interpolates.
- **Controls:** show only what the session enables.
- **Lyrics:** LRCLIB synced lyrics. Its terms are still an open question.
- **Later:** Linux MPRIS adapter; optional "Connect Spotify" (PKCE, bring-your-own developer app) for like/save. The Spotify constraints table from the old plan still applies.

## Agent integration

The island becomes a control surface for the coding agents the user already runs. It shows what each agent is doing, lets the user answer permission prompts without switching to the terminal, and adds up token usage across all of them.

### What we show

- **Sessions:** one entry per running agent session: agent, project folder, current state (working, waiting for permission, waiting for input, done, error) and the last tool call.
- **Permission requests:** the tool and its input (command, file and diff summary), with Allow once, Deny, and "Open terminal". Answering from the island returns the decision to the agent.
- **Turn finished / needs input:** an alert when an agent stops or is idle waiting for the user, with a jump to its terminal window.
- **Usage:** tokens in and out, cache reads and estimated cost per session, per agent, per day; plan limits when the agent exposes them.

### How each agent connects

| Agent | Live events and permissions | Token usage | Confidence |
| --- | --- | --- | --- |
| **Claude Code** | Hooks in user settings: `SessionStart`, `UserPromptSubmit`, `PermissionRequest`, `PostToolUse`, `Notification` (`idle_prompt`, `permission_prompt`), `Stop`, `SessionEnd`. `PermissionRequest` answers with `hookSpecificOutput.decision.behavior = allow \| deny`; echoing one of its `permission_suggestions` back as `updatedPermissions` gives "always allow". Handlers can be `command` (our bridge) or `http` (POST straight to the hub). Default timeout 600 s; a timeout means no decision and the terminal prompt shows (fail open for free). | Transcript JSONL `~/.claude/projects/<cwd>/<session>.jsonl`, `message.usage` per assistant line (dedupe on message id + request id). **Plan limits and cost** only come from the statusline JSON (`rate_limits.five_hour/seven_day.used_percentage`, `cost.total_cost_usd`, `context_window.*`), so we install a statusline forwarder that chains the user's existing one. OTLP metrics are an optional extra. | Verified against docs, Oct 2026 |
| **Codex CLI** | Hooks now exist and are on by default (`~/.codex/hooks.json`): `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PermissionRequest`, `Stop`, `SessionEnd`. `PermissionRequest` covers normal terminal sessions, allow/deny only (no "always" yet). The user has to trust new hooks once via `/hooks`, so the installer guides them. `app-server` only for sessions ReUI launches itself. | Rollouts `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`: `token_count` events with `total_token_usage` and `rate_limits.primary/secondary.used_percent`. Duplicate lines exist; count deltas (ccusage documents the traps). | Hooks verified; rollout fields from third-party sources |
| **OpenCode** | A small JS plugin in `~/.config/opencode/plugins/` forwards `permission.asked`, `session.status`, `session.idle` to the hub and replies through the SDK. Works with the TUI's random port. `opencode serve` + `/event` SSE is the alternative. Permission reply routes changed in 1.18 and again in 2.0, so feature-detect. | Read session storage the way ccusage does | Events verified; reply route needs a spike |
| **Gemini CLI** | Hooks: `BeforeTool` can deny (no dedicated permission event), `Notification`, `AfterAgent` for done | Per agent | Later |
| **Cursor** | `~/.cursor/hooks.json`: `beforeShellExecution` returns allow/deny/ask, `stop`, `afterAgentResponse` | Per agent | Later; confirm the CLI runs hooks |

### Bridge rules

- **Fail open to the agent.** If the hub isn't running or the island doesn't answer before a timeout, `reui-bridge` exits without a decision and the agent shows its own terminal prompt as usual. We never block an agent indefinitely.
- **No silent approvals.** The island only allows a tool when the user clicks. No "auto-approve" rules in v1.
- **Only the user can answer.** The pipe is restricted to the current user's SID, and every request carries a one-time ID so another process can't inject an approval.
- **Installing is explicit and reversible.** A "Connect Claude Code / Codex / OpenCode" toggle merges our hook entries into the agent's config (never overwrites), keeps a backup, and removes exactly what it added when turned off.
- **Usage is read-only and local.** Transcripts and rollouts are parsed on the device; nothing is uploaded. Cost is an estimate from a local price table the user can see.

### Prior art to read first

- **coucou** (louis-cfm/coucou): Tauri 2, runs on Windows as a top-centre island, Claude/Codex/Cursor/Gemini with Allow/Deny/Always. Closest existing thing to this module; study its hook installer and protocol.
- **Vibe Notch** (farouqaldori/vibe-notch, formerly Claude Island, Apache-2.0): the original hooks + socket Allow/Deny pattern on the Mac notch.
- **ccusage** (now Rust): parsers for Claude, Codex, OpenCode, Gemini and more, LiteLLM + models.dev pricing, offline mode. Port or reuse its parsers rather than writing our own.
- Also: vitaans/windows-notch, wxtsky/CodeIsland, MANISH007700/notch-island.
- **Cost:** prefer agent-reported cost (Claude statusline). Otherwise a bundled LiteLLM price snapshot refreshed daily, labelled "API-equivalent" since subscription users aren't billed per token.

## Clipboard

History with rich previews and paste-back from the island.

| Concern | Approach | Watch out |
| --- | --- | --- |
| Capture | `AddClipboardFormatListener` → `WM_CLIPBOARD­UPDATE` on a hidden hub window | Some apps write several formats in quick succession; debounce |
| Formats | Text, HTML, RTF, images (`PNG`, `CF_DIB`), files (`CF_HDROP`) | Store the original formats so paste-back is faithful, plus a preview |
| Previews | Image thumbnails, colour swatches for hex/rgb values, link cards (title, site, og:image, fetched once per link for public hosts only), code, file names, source app icons | Link previews make network calls: public hosts only, capped, `--no-link-previews` turns them off |
| Privacy | Skip items marked `ExcludeClipboardContentFromMonitorProcessing` or `CanIncludeInClipboardHistory = 0`, which password managers set; per-app exclude list; pause capture; auto-expire | Never sync history off the device in v1 |
| Paste-back | Put the item on the clipboard, restore focus to the previous window, send Ctrl+V with `SendInput` | Elevated windows reject input from a non-elevated process |
| Storage | SQLite with blobs, size cap and image downscaling for thumbnails | Large images and file lists |
| Linux (later) | `wlr`/`ext-data-control` protocols on wlroots and KDE | GNOME doesn't expose them |

## Libraries

Researched Oct 27, 2026 (versions from crates.io, licence and activity from GitHub). Rule: **GPL/AGPL/source-available projects are reference only; we don't copy their code.** Wrap every third-party crate behind our own trait and pin versions.

### Core, window and IPC

| Need | Pick | Notes |
| --- | --- | --- |
| Win32/WinRT | `windows` 0.62 | Base for nearly every module |
| Island renderer | windows-rs **Windows.UI.Composition** (`SpringVector2NaturalMotionAnimation`, `ExpressionAnimation`, `CompositionGeometricClip`) + **Direct2D/DirectWrite** into `CompositionDrawingSurface` | Chosen Oct 27, 2026. Same compositor the Windows shell itself uses, so it carries over to the taskbar and desktop. robmikh/minesweeper-rs is a Rust sample of this stack |
| Layout | `taffy` (flexbox/grid without a browser) | Add when cards outgrow hand-placed layout |
| Optional heavy panels | Tauri 2 / WebView2 + Motion, loaded on demand | ~100–150 MB while open, so never resident at idle. Fallbacks for the whole UI if native stalls: iced, egui |
| Click-through | Whole window click-through, toggled when the cursor enters the pill rect; `WS_EX_NOACTIVATE` + `WM_MOUSEACTIVATE → MA_NOACTIVATE` | No framework does per-pixel hit testing (Tauri issues #13070, #6164 still open) |
| Tray, hotkeys | `tray-icon`, `global-hotkey` | Tauri-maintained |
| Single instance | Named mutex via `windows` | `single-instance` crate is stale (2021) |
| Fullscreen / appbar | `SHQueryUserNotificationState`, `ABN_FULLSCREENAPP`, `SHAppBarMessage` directly | Cairo's ManagedShell is the AppBar reference |
| IPC | tokio named pipes or `interprocess`, framed with **postcard** | `bincode` 3.0 is a tombstone, don't use. `serde_json` only into the webview |
| Focus tracking | `SetWinEventHook` directly or `wineventhook` | Event-driven |

### Modules

| Module | Picks | Notes |
| --- | --- | --- |
| Media | windows-rs GSMTC; `wasapi` loopback + `realfft` for a live waveform; thin `reqwest` LRCLIB client; `musicbrainz_rs` | Read current-song2 (Nerixyz) for GSMTC handling. `mpris` for Linux later |
| Agents | `reui-bridge` (command hook) or Claude `http` hooks to the hub; OpenCode JS plugin; transcript/rollout tailers ported from ccusage; LiteLLM price snapshot | See Agent integration above |
| Clipboard | `clipboard-master` (listener) + `clipboard-win` (all formats, exclusion checks); `SendInput` via windows-rs for paste-back; `rusqlite` with FTS5; `nucleo-matcher` or `frizbee` for fuzzy search; `image` + `fast_image_resize` for thumbnails | Skip items carrying `ExcludeClipboardContentFromMonitorProcessing`, `CanIncludeInClipboardHistory = 0`, `CanUploadToCloudClipboard`, `Clipboard Viewer Ignore`. UX references: EcoPaste (Apache, Tauri), clippi (MIT), Maccy |
| Notifications | windows-rs `UserNotificationListener` to read; `tauri-winrt-notification` to send | Live change events likely need package identity, so plan a sparse MSIX package early |
| System OSD | Core Audio callbacks (`IAudioEndpointVolumeCallback`, `IMMNotificationClient`); `brightness` + `wmi` event subscription | Hiding the native flyout (HideVolumeOSD trick) may fail on XAML flyouts in 24H2/25H2; fallback is a keyboard hook. FluentFlyout is the living reference (GPL) |
| Devices | WinRT `DeviceWatcher` + `BluetoothLEDevice` + PnP battery property; `starship-battery` + power notifications | BlueGauge (MIT, Rust) is the reference for Bluetooth battery |
| Mic/camera in use | ConsentStore registry watched with `RegNotifyChangeKeyValue` | No crate; build in-house. No public API for screen share |
| System stats | `sysinfo`, `nvml-wrapper`, PDH for non-NVIDIA GPUs | Polled by nature, so only while the card is visible |
| Calendar (later) | `google-calendar3`, Graph via `reqwest`, `libdav` | |

### Reference projects

| Project | Why read it | Licence |
| --- | --- | --- |
| WinIsland | Rust island for Windows, almost our exact stack (GSMTC, notifications, Core Audio, DComp, WASAPI spectrum, lyrics) | GPL-3, reference only |
| Nectar | Tauri 2 + React notch with per-property springs, visibility modes, taskbar-replacing dock | GPL-3, reference only |
| Pillar | Tauri 2 + React island | MIT, reusable |
| coucou | Tauri island for coding agents on Windows | check before reuse |
| Boring Notch, Atoll, NotchDrop | macOS UX and state references | GPL / GPL / MIT |
| Seelen UI | Proof that Tauri can carry a full shell | AGPL, reference only |
| Cairo + ManagedShell | Tray hosting (`Shell_TrayWnd`, `TaskbarCreated`, `Shell_NotifyIcon` via `WM_COPYDATA`), AppBar; port the logic to Rust | Apache-2.0 |
| Zebar / GlazeWM, komorebi, ReactOS explorer | Widget bars from a Rust provider, WinEvent handling, shell internals | GPL / source-available / GPL |

## Feature order

Agreed Oct 27, 2026. One feature at a time, each end to end (real source → hub module → island → actions back → tests → measured idle cost) before the next starts.

| # | Feature | Status |
| --- | --- | --- |
| 1 | Agents: Claude Code (bridge, installer, sessions, permissions, token usage) | Working end to end Oct 27, 2026; dogfooded in this repo (project-local hooks). Open: plan rate limits and cost via the statusline, "open terminal" action, confirm terminal-dialog behaviour while the hook waits |
| 2 | Now Playing (GSMTC, artwork, transport, lyrics) | Spec: [docs/PRD-now-playing.md](docs/PRD-now-playing.md). v1 built (M0–M5, Oct 28, 2026): robust sessions (all subscribed, own ranking, ‹ › switcher, friendly names, ads, letterbox trim), tinted card with drag-scrub, rate, LIVE, shuffle/repeat, open player, per-app volume + output picker (IPolicyConfig with Sound-settings fallback), real per-process audio visualizer (WASAPI process loopback + FFT, synthetic fallback), opt-in LRCLIB lyrics, media.json settings and tray toggles. Open: hands-on matrix, card keyboard shortcuts, tint look |
| 3 | Agents: Codex, OpenCode | Built Oct 27, 2026: generic agent module with per-agent profiles, Codex hooks installer, OpenCode plugin, SQLite usage readers; hub/island side verified with simulated events. Open: install on this machine and verify with real sessions (needs the user to run the installers and trust Codex's hooks), Codex logo |
| 4 | Clipboard (history, previews, paste-back, panel) | Spec: [docs/PRD-clipboard.md](docs/PRD-clipboard.md). v1 built (M0–M4, Oct 28, 2026): faithful capture (HTML/RTF, PNG alpha, cut files), SQLite v1 schema on its own thread, search + type tabs + scrolling card row, keyboard without stealing focus, card menu, pins/delete/undo, safe paste-back (elevation/focus check, terminal chord), privacy (ignored apps, secret detection, pause, retention, clear), safe link previews. Open: hands-on paste matrix, v2 (OCR, categories, drag-out) |
| 5 | System OSD (volume, brightness) | Spec: [docs/PRD-system-osd.md](docs/PRD-system-osd.md) (M0–M4). Built Oct 28: volume HUD (level bar, bubble ring while music plays, no echo, device moment), brightness (panel events + WMI, DDC/CI set), opt-in Windows flyout hiding with crash-safe restore, optional lock keys / battery saver / mic mute |
| 6 | Devices (Bluetooth battery, USB, charging) | Spec: [docs/PRD-devices.md](docs/PRD-devices.md) (M0–M4), draft for review |
| 6.5 | **Product shell** (install, first run, Settings window, trust) | Spec: [docs/PRD-product-shell.md](docs/PRD-product-shell.md) (P0–P5), draft for review. Comes before feature #7 |
| 7 | Timers and focus | Spec: [docs/PRD-timers-focus.md](docs/PRD-timers-focus.md). Built Oct 28 (M0–M4): island-run clocks (protocol v17), typed timers (`10m tea`) from Win+Ctrl+T / tray / `reui-bridge timer`, several at once, a finish that opens and rings, late finishes after sleep or restart, focus sessions (ask or auto-start, optional Do Not Disturb, checked), stopwatch with laps, Settings → Timers. Open: hands-on typing and clicks, sleep/resume with a running timer |
| 8 | Notifications (needs package identity) | |
| 9 | Mic / camera in use | Spec: [docs/PRD-privacy-indicators.md](docs/PRD-privacy-indicators.md). Built Oct 30 (M0–M1): the consent store watched with `RegNotifyChangeKeyValue`, uses checked against their processes, mic, camera, screen capture (`graphicsCaptureProgrammatic`) and location, mic mute, Settings → Mic & camera. Open: Core Audio capture sessions (M2), HKLM for elevated apps |
| 10 | Calendar | |
| 11 | Stats, downloads, file shelf | |

## Roadmap

Each phase ends with a gate measured against the budgets, not with a date.

| Phase | Scope | Gate |
| --- | --- | --- |
| 0a. Composition spike ✅ | Native pill: spring morph compact ↔ expanded, Direct2D text and artwork, click-through toggling, no focus steal, fullscreen hide; measure CPU of our process **and dwm.exe**, plus memory | Done Oct 27, 2026: pill process ≈ 0 CPU, ~57 MB private; idle pill adds nothing to DWM; animations cost DWM while running. The spike itself was removed once `reui-island` replaced it |
| 0b. Island + core 🟡 | Hub + IPC skeleton, module interface, activity arbitration, multi-monitor and DPI changes, soak-test harness | Oct 27, 2026: workspace, protocol, hub (modules, arbitration, user-only pipe, island supervision in a kill-on-close job), island rendering any activity, demo module; end-to-end permission round trip verified. Idle: hub 1 MB private, island ~58 MB. **Still open:** multi-monitor and DPI changes, soak-test harness | Idle CPU near zero, memory within budget, morph animation smooth at 60 fps |
| 1. Music | GSMTC adapter, Player record, compact and expanded music states, lyrics | Track change reaches the island within budget |
| 2. Agents | `reui-bridge`, Claude Code first (events, permission answer, usage), then Codex and OpenCode, usage dashboard | Permission round-trip reliable; fail-open verified by killing the hub mid-request |
| 3. Clipboard | Capture, previews, history UI, paste-back, privacy rules | Password-manager items never stored |
| 4. More modules | Next modules from the catalogue, in Rohit's order | Each meets its own budget; total idle cost still within the island budget |
| 5. Shell groundwork | See below | |

Phases 1 to 3 can reorder depending on which UI mockups arrive first; they only depend on phase 0.

## Toward our own Windows shell

The island is the first surface of a future explorer.exe replacement. Nothing here is v1 work, but these choices keep the door open:

- **Hub and surfaces stay separate processes.** The future taskbar, desktop and tray host are just more surfaces on the same hub.
- **Prototype path:** a per-user `Shell` value under `HKCU\Software\Microsoft\Windows NT\CurrentVersion\Winlogon` on a second account or VM (Shell Launcher is the official route but only on Enterprise/Education/IoT editions). Recovery is Ctrl+Shift+Esc → Run new task → `explorer.exe`.
- **What the shell must add:** taskbar via `RegisterShellHookWindow` (documented as possibly changing in future Windows versions), a tray host window with class `Shell_TrayWnd`, desktop icons and file operations through the shell namespace COM interfaces, and app-bar space reservation (`SHAppBarMessage`) so maximised windows avoid our bars.
- **Hardest parts last:** Start menu (`StartMenuExperienceHost.exe`) and search can't simply be replaced; taskbar pins live in undocumented caches; Windows updates break shell customisers, so expect upkeep.
- **References:** ReactOS Explorer source (taskbar via shell hooks), Cairo Desktop (maintained full shell), Seelen UI (Rust + Tauri overlay), Shelled (web-stack shell), and the UFO² paper for an agent-native desktop.

## Risks

| Risk | Effect | Mitigation |
| --- | --- | --- |
| Agent hook formats change between releases | Events or approvals break | One adapter per agent behind a trait; version check at connect; fail open |
| A bug in the bridge blocks an agent | User loses trust fast | Hard timeout, fail open, kill-the-hub test in CI |
| Editing users' agent config files | Broken config | Merge, back up, remove exactly what we added |
| WebView2 memory cost | Island too heavy for an always-on surface | Measure in phase 0; native renderer as fallback |
| Clipboard holds secrets | Privacy incident | Respect exclusion formats, per-app excludes, local only, auto-expire |
| Spotify / LRCLIB terms | Features pulled | Media session is the default; review terms before shipping lyrics |
| Windows updates break shell work | Later shell unstable | Keep the island working without shell replacement |

## Open questions

- [x] Renderer: native Windows.UI.Composition, decided Oct 27, 2026; confirm with the phase 0a measurements.
- [ ] Pill look: solid black like Apple's island, or frosted glass (host backdrop brush, needs testing)?
- [ ] Core in Rust (assumed) or C#?
- [ ] Which agent comes first after Claude Code: Codex or OpenCode?
- [ ] Offer "always allow"? Claude Code and OpenCode support it, Codex doesn't yet.
- [ ] Claude Code: our own `reui-bridge` exe, or `http` hooks posting straight to the hub (no exe, but needs a localhost port)?
- [ ] Do LRCLIB's terms allow lyrics in a shipped app?
- [ ] UI mockups for island states, agent cards and clipboard previews (from Rohit).
