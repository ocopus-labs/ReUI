# Developing ReUI

Building from source, the programs' flags, where files live, and how the coding-agent hooks work. What each feature does for users is in the [guide](guide.md); where the code lives is in [ARCHITECTURE.md](../ARCHITECTURE.md), why in [PLAN.md](../PLAN.md), the look in [DESIGN.md](../DESIGN.md), and each feature's spec in `docs/PRD-*.md`.

## Layout

| Path | What |
| --- | --- |
| `design/tokens.json` | Design tokens, the source of truth for the look (see `DESIGN.md`) |
| `crates/reui-design` | Tokens compiled into typed Rust constants at build time, and the island's layout maths (`src/layout/`, shared with Settings' previews) |
| `crates/reui-protocol` | Wire protocol between the hub and surfaces: a semantic view model after Apple's Live Activity presentations (compact, minimal, expanded), postcard frames |
| `crates/reui-hub` | Background process. Owns every module's state, decides which activity is in front, serves surfaces over a per-user named pipe, supervises the island |
| `crates/reui-island` | The pill, rendered natively with Windows.UI.Composition; renders whatever the hub puts in front and sends actions back |
| `crates/reui-settings` | The Settings window (Direct2D), the welcome, and the update check and install |
| `crates/reui-setup` | Per-user installer and uninstaller (`ReUI-Setup-<version>.exe`), with the programs embedded at build time |
| `crates/reui-bridge` | The program coding agents run as a hook; forwards events to the hub, answers permission requests and Claude's questions and plans, and installs/uninstalls the hooks |

## Build from source

Windows only, MSVC toolchain.

```
cargo build --release
target\release\reui-hub.exe --demo
cargo test --workspace
```

Release builds of the hub have no console window; debug builds (`cargo build`) keep one for the logs. `pwsh scripts/package.ps1 -OutDir dist` builds the release assets (the setup and the zip) the way `.github/workflows/release.yml` does on a `v*` tag. A tagged release needs a `## <version>` entry in `CHANGELOG.md`, and the tag must match the `reui-hub` crate version.

**Run test instances sandboxed:** `REUI_CONFIG_DIR=<dir>` gives the hub, island and Settings their own files, named pipe and single-instance lock, so they run beside an installed ReUI without touching its data.

To drive a sandboxed island without touching the real mouse or keyboard, post to its own window (class `ReUI.Island`, found by its process): `WM_LBUTTONDOWN` / `WM_LBUTTONUP` at client pixels click it, and in debug builds `WM_APP + 20` with screen pixels in `lparam` (as `WM_MOUSEMOVE` packs them) is the pointer resting there, for hover and peek; `WM_APP + 21` with a process id in `wparam` is that process coming to the front, for app rules (the island reads its executable name as it does for a real foreground change); `WM_APP + 22` with an entry index in `wparam` and a client point in `lparam` picks that entry of the quiet menu for the moment there (Don't show these, Snooze for 1 hour), so no menu opens at the user's pointer. Never post a right-click to a sandboxed island while someone is using the machine: its menu opens at the real pointer. Module commands go through the pipe (`reui-bridge stats`, `reui-bridge hide`).

## The hub's flags

The hub starts the island plus every module. `--no-agents`, `--no-media`, `--no-clipboard`, `--no-osd`, `--no-devices`, `--no-timers`, `--no-privacy`, `--no-stats`, `--no-shelf`, `--no-downloads`, `--no-screenshots` skip one; `--no-link-previews` keeps clipboard links offline; `--no-updates` never looks for updates or mentions them. `--no-island` runs the hub alone so you can start `reui-island.exe` yourself (under a debugger, say). `--no-onboarding` skips the first run and one-time tips; `--first-run` replays the demo and welcome. The full list is the doc comment at the top of `crates/reui-hub/src/main.rs`.

`--demo` enables a module that shows now-playing, a 5-minute timer and, 25 s after each one is settled, the next agent moment: a permission with a scrolling script, a three-step question (one step multi-select) and a plan to approve. They take over the pill, answer nothing real, and time out and fail open (20 s for the permission, 60 s for the others). Hover to peek, click to expand, right-click the pill to quit ReUI. `--demo-music` runs only its music (no timer, no moments): what `tools/perf-sample.ps1` measures for "music playing".

`pwsh -File tools/perf-sample.ps1 [-Seconds 60] [-NoBuild]` measures the README's Performance table: it builds release, runs the hub and island sandboxed (`REUI_CONFIG_DIR` in `%TEMP%`, the modules that react to the user's own input off) through three scenes (at rest, music playing with the card closed, the System stats card open) and prints CPU (share of the whole machine), mean private memory, and how busy the rest of the machine was meanwhile.

## Settings' flags

`reui-settings.exe --page <name>` opens on a page (general, now, clipboard, osd, devices, downloads, coding, privacy, about); `--welcome` opens the three welcome steps; `--perf` logs frame times. `--shots <dir>` saves every page as a PNG without showing the window; add `--backdrop-color #RRGGBB` to put that colour on the island stages instead of your wallpaper, and `--stages` to save only the stages, each alone at 200 % (`stage-<moment>[-light|-notch].png`). `pwsh -File tools\readme-images.ps1` renders the README's pictures that way, sandboxed, into `docs/images`.

## Install, by hand

Setup options: `--no-autostart` (don't start with Windows), `--no-launch`, `--quiet` (no message boxes). It installs into `%LOCALAPPDATA%\Programs\ReUI`. `ReUI-<version>-x64.zip` holds the same programs: unzip and run `reui-setup.exe` (it installs the files next to it), or run `reui-hub.exe` in place. `reui-setup.exe --uninstall` removes it; `--quiet` uninstalls without asking and keeps your data.

## Files on disk

| Path | What |
| --- | --- |
| `%LOCALAPPDATA%\Programs\ReUI` | The programs |
| `%APPDATA%\ReUI\*.json` | Settings, one file per module (`settings.json` is the island's), written by Settings and read by the modules |
| `%APPDATA%\ReUI\clipboard.db` | Clipboard history |
| `%APPDATA%\ReUI\coding.json` | Coding agents: `show_sessions` (`needed` \| `working`), `show_finished` |
| `%APPDATA%\ReUI\clipboard.json` | `hotkey`, `retention_days`, `ignored_apps`, `link_previews` |
| `%APPDATA%\ReUI\calendar.json` | `lead_min`, `windows`, `sources` (`id`, `host`, `url` DPAPI-sealed, `on`) |
| `%LOCALAPPDATA%\ReUI\calendar\` | Each calendar's last good copy (`<id>.cache`, DPAPI-encrypted) and `status.json` (each source's last refresh) |
| `%APPDATA%\ReUI\downloads.json`, `screenshots.json` | Downloads: `enabled`, `folders` (watched besides Downloads); screenshots: `enabled` |
| `%APPDATA%\ReUI\privacy.json` | The indicator's kinds (`microphone`, `camera`, `screen`, `location`), `calls` and each call app (`call_teams`, `call_zoom`, `call_discord`, `call_slack`, `call_webex`, `call_messengers`, `call_browsers`) |
| `%APPDATA%\ReUI\osd.json` | The HUDs and states, `scroll_step`, and `mixer` (each app's level in the open volume card; its button there flips it) |
| `%LOCALAPPDATA%\ReUI` | Caches and logs |
| `%LOCALAPPDATA%\ReUI\update.json` | What the last update check found |
| `%APPDATA%\ReUI\shelf.json` | File shelf: `keep` (`removed` \| `restart` \| `day` \| `week`), `saved_cap_mb` |
| `%LOCALAPPDATA%\ReUI\shelf\` | What the file shelf holds (`held.json`) and the copies it made (`files\<card>\`) |
| `%LOCALAPPDATA%\ReUI\agents.json` | Each coding agent's last event (at most one write per 10 s) |
| `%LOCALAPPDATA%\ReUI\bin\reui-bridge.exe` | The bridge the hooks run; the hub keeps it in step with itself when it starts |

## Updates

ReUI looks for a new GitHub release once a day (Settings → About → Check automatically, on by default; Privacy & data lists it): the hub runs `reui-settings.exe --check-updates`, which asks `api.github.com` for the latest release and notes it in `%LOCALAPPDATA%\ReUI\update.json`. A new version gets a card in the island that waits for an answer without taking over: **Later** puts that version off for a day (a newer one shows at once), **Install** runs `reui-settings.exe --install-update` (no window), which prints one line per step (`progress <0..1>`, `verifying`, `installing`, `error <message>`) that the card follows as a download bar, then "Installing", or "Couldn't update" with the reason and Try again. The tray menu's **Install update** does the same. Turning Check automatically off means no network and no card. **Install** in the island and in Settings → About both download `ReUI-Setup-<version>.exe` to `%TEMP%`, check its size and its SHA-256 against the release's `sha256sums.txt`, and run it with `--quiet` (it stops ReUI, upgrades and starts it again). On the first start of a new version, About shows that version's CHANGELOG entry once (Settings → About → What's new after updates).

The repository is one constant, `REPOSITORY` in `crates/reui-settings/src/updates/mod.rs` (`ocopus-labs/ReUI`); checks and downloads are anonymous, so they find releases only once it is public. To try the island's flow, run a sandboxed hub (`REUI_CONFIG_DIR=<dir>`) with a hand-written `<dir>\local\update.json` (`available`, `setup_url`, `notes`) and `REUI_UPDATE_SIMULATE=ok` or `fail`: a sandbox never installs, and with that set `--install-update` plays the steps out without any network.

## Coding agents (Claude Code, Codex, OpenCode)

One `agent` module per agent tracks each session and shows it only when it needs you (see [PRD-agent-moments.md](PRD-agent-moments.md)): permission requests, Claude Code's and OpenCode's questions, Claude Code's plans and MCP forms and links, all answered from the island; Codex's questions (`request_user_input`, which no hook can answer) shown read-only with "Answer in terminal"; and a failed turn, with "Open terminal" beside it. Each such card starts with the session strip (agent, project, what you asked, time in turn), so two sessions never get confused; several waiting share one card, oldest first, with a pager on the strip and the count in the pill. A permission shows the full command or an edit's changed lines in a scrolling block (introduced by what it's for, never repeated in the hero), with secrets masked by the clipboard's rules. Claude Code and OpenCode offer Deny / Always / Allow; Codex takes Deny / Allow (its hooks don't accept "always"); Alt+click Deny (or Shift+Enter) denies with a message. Claude's questions (AskUserQuestion: steps, multi-select, a typed answer) and plans (ExitPlanMode: Approve, or Keep planning with feedback) are answered through a blocking `PreToolUse` hook, its MCP elicitations through the blocking `Elicitation` hook (a form one field per step: enum list, Yes / No, text or number; or a link: Open link / Decline). OpenCode's permissions and questions arrive on its event bus (`permission.asked`, `question.asked`) and the plugin posts the island's answer back through OpenCode's server; answering in OpenCode instead (`*.replied`) takes the card away. "Answer in terminal" hands any of them back at once and, like "Open terminal", brings forward the window the agent runs in (the bridge sends its parent process chain; the hub picks the nearest ancestor with a window). Sessions at work (thinking, running a tool) and finished turns stay out of the island unless `%APPDATA%\ReUI\coding.json` asks for them (`show_sessions`: `needed` | `working`, `show_finished`), which Settings → Coding agents sets. Token usage is still counted.

```
%LOCALAPPDATA%\ReUI\bin\reui-bridge.exe install claude   [--scope user|project|local] [--project DIR]
%LOCALAPPDATA%\ReUI\bin\reui-bridge.exe install codex    [--scope user|project]   then trust the hooks in Codex: /hooks
%LOCALAPPDATA%\ReUI\bin\reui-bridge.exe install opencode [--scope user|project]   loaded on OpenCode's next start
%LOCALAPPDATA%\ReUI\bin\reui-bridge.exe uninstall <agent> [same options]
%LOCALAPPDATA%\ReUI\bin\reui-bridge.exe test <agent>      the island answers "Connected" (exits 1 if ReUI isn't running)
```

Settings → Coding agents does the same with Connect / Disconnect / Send test, and shows each agent's status ("Connected · last event 2 min ago", "Agent not found on this PC"); the hub notes each agent's last event in `%LOCALAPPDATA%\ReUI\agents.json` (at most one write per 10 s).

| Agent | What gets installed | Usage from |
| --- | --- | --- |
| Claude Code | Hooks in `.claude/settings(.local).json` (exec form). Blocking: `PermissionRequest` (120 s), `PreToolUse` for `AskUserQuestion\|ExitPlanMode` (run as `reui-bridge claude ask`, 300 s) and `Elicitation` (300 s, MCP forms and links) | Session transcripts |
| Codex | Hooks in `~/.codex/hooks.json` (command line; Codex has no `args`) | `~/.codex/state_N.sqlite` → `threads.tokens_used` |
| OpenCode | Plugin `~/.config/opencode/plugins/reui.js` (OpenCode 1.3 or later; the hub rewrites an older copy of it on start) | `~/.local/share/opencode/opencode.db` → `session` tokens |

The installer merges into existing settings (other tools' hooks are untouched), keeps the original as `*.reui-backup`, and uninstall removes exactly its own entries. The hub keeps the copy in `%LOCALAPPDATA%\ReUI\bin` in step with itself when it starts, so a protocol change never silently disconnects agents.

Safety: status events are `async` (agents never wait for them). Only what needs you waits: permission requests (the hub gives up after 100 s, the bridge after 110 s, the hook after 120 s) and questions, plans and MCP forms (280 / 290 / 300 s; OpenCode's own prompt stays up meanwhile, so it has no hook timeout). Every failure (hub not running, timeout, bridge killed) means "no decision", so the agent asks you itself as usual. The bridge prints nothing except a decision and always exits 0. Hooks installed by a build before agent moments M2 lack the blocking question entry: those questions show read-only ("Answer in terminal") and Settings → Coding agents offers **Update hooks**.

Usage numbers are **fresh tokens** (cache reads, usually ~97 % of all tokens at ~10 % of the price, are left out). Codex records one running total per thread, so its "today" counts whole threads active today.

## Now Playing

The hub's `media` module follows Windows' media sessions (GSMTC), so any player that shows up in the Windows media flyout works: Spotify, browsers, VLC, Media Player. It shows the artwork (its most vivid colour becomes the accent), title, artist, the player's name, a timeline the island advances itself, and only the transport controls the player enables. Playing is ongoing; paused steps back; nothing playing removes it. `--no-media` turns it off.

## Clipboard

The hub's `clipboard` module keeps everything you copy (text with its formatting, images with transparency, files) in `%APPDATA%\ReUI\clipboard.db` and gives it back from the island. The full spec is [PRD-clipboard.md](PRD-clipboard.md); every key and gesture is in the [guide](guide.md#clipboard-history).

- The app you were in keeps focus, so the paste lands there; if it runs elevated, or you switched apps, the item is left on the clipboard and the island says "Press Ctrl+V". Terminals get Shift+Insert.
- Privacy: copies that apps mark private (password managers) are never read; copies from ignored apps and text that looks like a secret (private keys, API tokens, JWTs) are never stored; history older than 30 days is removed except pinned items; up to 1 000 items / 500 MB. Pause for an hour or clear history from the tray. Nothing leaves the machine except link previews (public sites only, fetched when the card is shown; links that look like reset/sign-in/signed URLs only fetch the site's name).
- Settings: `%APPDATA%\ReUI\clipboard.json` (`hotkey`, `retention_days`, `ignored_apps`, `link_previews`). `--no-link-previews` and `--no-clipboard` turn previews / the module off.

## Privacy indicators internals

The hub's `privacy` module shows which app is using the microphone, the camera or capturing the screen (location too, off by default), like iOS's orange and green dots: the app's icon and the kind's glyph in the pill, a dot in the bubble, and in the open card every app, since when, a mute for the microphone and **Privacy settings**. It reads Windows' own record of who uses what (the Capability Access Manager's consent store, watched for changes, no polling) and drops a use once no process from before it is running, so a crashed app never leaves a dot. ReUI's own programs never count. Settings: `%APPDATA%\ReUI\privacy.json` (Settings → Mic & camera); `--no-privacy` turns it off. Spec: [docs/PRD-privacy-indicators.md](docs/PRD-privacy-indicators.md).

Calls are the same module (`calls.rs`, `call_view.rs`): a known call app's microphone use that lasts 5 s becomes `call:<exe>` (Ongoing, a `Clock` from the microphone's start), the indicator leaves that app out, and the call ends 3 s after the microphone is released. No call app's own API is used (Teams retired its local one in 2026). Spec: [docs/PRD-calls.md](docs/PRD-calls.md).

The volume card's mixer (`osd/apps.rs`, `osd/mixer.rs`) runs `audio::watch_sessions` (`IAudioSessionNotification` + `IAudioSessionEvents` on the default output, its own thread) only while the card is open with the mixer on, so a closed card watches nothing. To try it without changing a real app's level, play silence from a script (a `System.Media.SoundPlayer` on a zeroed WAV); note that Windows remembers an app's level and mute per exe across runs.


## System stats internals

The hub's `island` module (`modules/hide.rs`) registers the island's hide shortcut (`hide_shortcut` in `settings.json`, Win+Alt+I by default; Win+Alt+H is Windows' voice typing) and relays a press as `HubMsg::ToggleHidden`; the island keeps the state (`hidden` in `settings.json`). `reui-bridge hide` sends the same request through the pipe, which is how scripts test it without pressing keys.

The hub's `stats` module shows CPU, GPU, memory, disk and network in a card: **Win+Alt+S**, Tray → System stats, or `reui-bridge stats`. Stats have no OS events, so they're polled, but only while the card is open: opening it starts a sampler thread that reads once a second (like Task Manager), and the island's "collapsed" ends it. With the card closed the module costs nothing (measured). CPU matches Windows 11's Task Manager (`GetSystemTimes`, i.e. `% Processor Time`); GPU is the busiest engine with its processes summed, from PDH's English counter names. Settings: `%APPDATA%\ReUI\stats.json` (`shortcut`, `gpu`, `disk`, `network`, `power`). `--no-stats` turns it off. The full spec and the costs are in [docs/PRD-system-stats.md](docs/PRD-system-stats.md).

## Calendar internals

The hub's `calendar` module reads secret ICS links (Google's, Outlook's, any provider's) and, opt-in, Windows' own appointment store. Feeds are fetched with `If-None-Match` / `If-Modified-Since` every 15 minutes, at once on resume and when connectivity hints say the internet is back, parsed off the hub thread by its own RFC 5545 reader (`ics/`: VTIMEZONE, RRULE in the event's wall time, exceptions; zones without a VTIMEZONE from Windows' ICU), and expanded for yesterday to the day after tomorrow. The module then sleeps until the next countdown, start, lapse, end or midnight; the island runs the ring. Links are DPAPI-encrypted in `%APPDATA%\ReUI\calendar.json` (`sources[].url` is `dpapi:` + hex; a link typed in by hand is sealed on first read) and the last good copy of each feed in `%LOCALAPPDATA%\ReUI\calendar\<id>.cache`; `calendar\status.json` is each source's last refresh for Settings. Logs never name a link or an event. To try it without a real calendar, serve an `.ics` on `http://127.0.0.1` (plain http is accepted only for loopback) and add that link. `--no-calendar` turns it off. Spec and research: [docs/PRD-calendar.md](PRD-calendar.md).

## File shelf internals

The island is the drop target and the drag source; the hub's `shelf` module holds the list. A drag from elsewhere near the island (`DragGuard` says a button that went down elsewhere is held, within `island.shelf.reach` of the shape) makes the window hit-testable for it, which is the only way OLE's `DoDragDrop` finds a `WS_EX_TRANSPARENT` layered window; away from it the island is click-through again at once. `dragdrop/target.rs` answers Copy, else Link, never Move, reads the formats once on the drop (`CF_HDROP`, `FileGroupDescriptorW`/`FileContents`, `UniformResourceLocatorW`, `PNG`, `CF_UNICODETEXT`) and sends `ClientMsg::Drop` to the destination with `drops`. Out: `SHDoDragDrop` of the shell's data object (`SHCreateDataObject` over absolute IDLists, so files from several folders go together, with `CF_HDROP`), or text and URL formats; marked `ReUI.Drag` so the island never takes its own drag back. Debug builds: `WM_APP + 21` posted to the island (wparam 1 over, 0 leave, 2 let go of `REUI_TEST_DROP`'s `;`-separated paths) shows the target and checks island → hub → shelf without a real drag. Real drags need a person: drag a file from Explorer to the island and back into another folder (it must copy, never move). `--no-shelf` turns it off. Spec: [docs/PRD-file-shelf.md](PRD-file-shelf.md).

## Downloads and screenshots internals

The hub's `downloads` and `screenshots` modules share `modules/folders/`: an overlapped `ReadDirectoryChangesW` per folder on one thread per module, so nothing runs between downloads or shots. Downloads are recognised by the browsers' partial files (`.crdownload`, `.opdownload`, `.part`, `.partial`, `.!qB`, `.!ut`), sized at most once a second after a write, and done when renamed to their final name; no browser records the total, so there's no percentage. Screenshots come from `Pictures\Screenshots` and from the clipboard module, which hands Snipping Tool's images over (`screenshots::claim`) instead of showing its own toast. Spec: [PRD-downloads-screenshots.md](PRD-downloads-screenshots.md). `--no-downloads` / `--no-screenshots` turn them off.

To try them without touching your own folders, a sandboxed hub (`REUI_CONFIG_DIR` set) also reads `REUI_DOWNLOADS_DIR` and `REUI_SCREENSHOTS_DIR` and watches those instead; outside a sandbox they're ignored. Grow a file named `<name>.crdownload` there from a script (append and flush every 250 ms), rename it to `<name>`, or save a PNG into the screenshots folder.

## Adding a module

Implement `modules::Module` in `crates/reui-hub/src/modules/`. In `run`, publish activities with `ctx.publish(key, priority, View { accent, compact, minimal, expanded })`, retract them with `ctx.retract(key)`, and handle inputs (button presses, agent hook calls) from `ctx.next()`. Register it in `main.rs`. Modules stay event-driven (no polling where an event exists) and describe meaning only: never sizes, fonts or theme colours. New features get a PRD in `docs/` first.
