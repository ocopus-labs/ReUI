# ReUI

A unified dynamic island for Windows, and later our own Windows shell. See `PLAN.md` for the plan and `DESIGN.md` for the design system.

## Layout

| Path | What |
| --- | --- |
| `design/tokens.json` | Design tokens, the source of truth for the look (see `DESIGN.md`) |
| `crates/reui-design` | Tokens compiled into typed Rust constants at build time |
| `crates/reui-protocol` | Wire protocol between the hub and surfaces: a semantic view model after Apple's Live Activity presentations (compact, minimal, expanded), postcard frames |
| `crates/reui-hub` | Background process. Owns every module's state, decides which activity is in front, serves surfaces over a per-user named pipe, supervises the island |
| `crates/reui-island` | The pill, rendered natively with Windows.UI.Composition; renders whatever the hub puts in front and sends actions back |
| `crates/reui-setup` | Per-user installer and uninstaller (`ReUI-Setup-<version>.exe`), with the programs embedded at build time |
| `crates/reui-bridge` | The program coding agents run as a hook; forwards events to the hub, answers permission requests and Claude's questions and plans, and installs/uninstalls the hooks |

## Install

1. Download `ReUI-Setup-<version>.exe` from the repository's **Releases** page.
2. Run it. It installs for your user only (no admin) into `%LOCALAPPDATA%\Programs\ReUI`, adds ReUI to the Start menu, starts it with Windows, and launches it.
3. Releases aren't code-signed yet, so Windows SmartScreen may say "Windows protected your PC". Choose **More info → Run anyway**.

Running the setup again, or a newer one, upgrades in place; your settings and clipboard history (`%APPDATA%\ReUI`) are kept. Launching ReUI from the Start menu while it's running opens Settings. Prefer no installer? `ReUI-<version>-x64.zip` holds the same programs: unzip and run `reui-setup.exe` (it installs the files next to it), or run `reui-hub.exe` in place.

Setup options: `--no-autostart` (don't start with Windows), `--no-launch`, `--quiet` (no message boxes).

## Guide

What each feature does, and every shortcut and gesture: [docs/guide.md](docs/guide.md). Settings → About → Replay the tour shows the three welcome steps again.

## Updates

ReUI looks for a new GitHub release once a day (Settings → About → Check automatically, on by default; Privacy & data lists it): the hub runs `reui-settings.exe --check-updates`, which asks `api.github.com` for the latest release and notes it in `%LOCALAPPDATA%\ReUI\update.json`. A new version gets a card in the island that waits for an answer without taking over: **Later** puts that version off for a day (a newer one shows at once), **Install** runs `reui-settings.exe --install-update` (no window), which prints one line per step (`progress <0..1>`, `verifying`, `installing`, `error <message>`) that the card follows as a download bar, then "Installing", or "Couldn't update" with the reason and Try again. The tray menu's **Install update** does the same. Turning Check automatically off means no network and no card. **Install** in the island and in Settings → About both download `ReUI-Setup-<version>.exe` to `%TEMP%`, checks its size and its SHA-256 against the release's `sha256sums.txt`, and runs it with `--quiet` (it stops ReUI, upgrades and starts it again). On the first start of a new version, About shows that version's CHANGELOG entry once (Settings → About → What's new after updates). The repository is one constant, `REPOSITORY` in `crates/reui-settings/src/updates/mod.rs` (`ocopus-labs/ReUI`); checks and downloads are anonymous, so they find releases only once it is public. To try the island's flow, run a sandboxed hub (`REUI_CONFIG_DIR=<dir>`) with a hand-written `<dir>\local\update.json` (`available`, `setup_url`, `notes`) and `REUI_UPDATE_SIMULATE=ok` or `fail`: a sandbox never installs, and with that set `--install-update` plays the steps out without any network.

## Uninstall

**Settings → Apps → Installed apps → ReUI → Uninstall**, or run `%LOCALAPPDATA%\Programs\ReUI\reui-setup.exe --uninstall`. It stops ReUI, puts back Windows' volume pop-up if ReUI was replacing it, removes the coding-agent hooks it installed, start with Windows and the Start menu entry, then asks whether to delete your data too (clipboard history, settings, caches). `--quiet` uninstalls without asking and keeps your data.

## Build from source

```
cargo build --release
target\release\reui-hub.exe --demo
```

Release builds of the hub have no console window; debug builds (`cargo build`) keep one for the logs. `pwsh scripts/package.ps1 -OutDir dist` builds the release assets (the setup and the zip) the way `.github/workflows/release.yml` does on a `v*` tag.

The hub starts the island plus the coding-agent and Now Playing modules (`--no-agents`, `--no-media` to skip them). `--demo` enables a module that shows now-playing, a 5-minute timer and, 25 s after each one is settled, the next agent moment: a permission with a scrolling script, a three-step question (one step multi-select) and a plan to approve. They take over the pill, answer nothing real, and time out and fail open (20 s for the permission, 60 s for the others). Hover to peek, click to expand, right-click the pill to quit ReUI. `--no-island` runs the hub alone so you can start `reui-island.exe` yourself.

## Coding agents (Claude Code, Codex, OpenCode)

One `agent` module per agent tracks each session and shows it only when it needs you (see [docs/PRD-agent-moments.md](docs/PRD-agent-moments.md)): permission requests and Claude Code's questions and plans, answered from the island, and a failed turn. Each such card starts with the session strip (agent, project, what you asked, time in turn), so two sessions never get confused; several waiting share one card, oldest first, with a pager on the strip and the count in the pill. A permission shows the full command or an edit's changed lines in a scrolling block (introduced by what it's for, never repeated in the hero), with secrets masked by the clipboard's rules. Claude Code offers Deny / Always / Allow; Codex and OpenCode take Deny / Allow (their hooks don't accept "always"); Alt+click Deny (or Shift+Enter) denies with a message. Claude's questions (AskUserQuestion: steps, multi-select, a typed answer) and plans (ExitPlanMode: Approve, or Keep planning with feedback) are answered through a blocking `PreToolUse` hook; "Answer in terminal" hands any of them back at once. Sessions at work (thinking, running a tool) and finished turns stay out of the island unless `%APPDATA%\ReUI\coding.json` asks for them (`show_sessions`: `needed` | `working`, `show_finished`), which Settings → Coding agents sets. Token usage is still counted.

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
| Claude Code | Hooks in `.claude/settings(.local).json` (exec form). Blocking: `PermissionRequest` (120 s), `PreToolUse` for `AskUserQuestion\|ExitPlanMode` (run as `reui-bridge claude ask`, 300 s) and `Elicitation` (300 s, answered "no decision" at once until MCP forms are supported) | Session transcripts |
| Codex | Hooks in `~/.codex/hooks.json` (command line; Codex has no `args`) | `~/.codex/state_N.sqlite` → `threads.tokens_used` |
| OpenCode | Plugin `~/.config/opencode/plugins/reui.js` | `~/.local/share/opencode/opencode.db` → `session` tokens |

The installer merges into existing settings (other tools' hooks are untouched), keeps the original as `*.reui-backup`, and uninstall removes exactly its own entries. The hub keeps the copy in `%LOCALAPPDATA%\ReUI\bin` in step with itself when it starts, so a protocol change never silently disconnects agents.

Safety: status events are `async` (agents never wait for them). Only what needs you waits: permission requests (the hub gives up after 100 s, the bridge after 110 s, the hook after 120 s) and Claude's questions and plans (280 / 290 / 300 s). Every failure (hub not running, timeout, bridge killed) means "no decision", so the agent asks you itself as usual. The bridge prints nothing except a decision and always exits 0. Hooks installed by a build before agent moments M2 lack the blocking question entry: those questions show read-only ("Answer in terminal") and Settings → Coding agents offers **Update hooks**.

Usage numbers are **fresh tokens** (cache reads, usually ~97 % of all tokens at ~10 % of the price, are left out). Codex records one running total per thread, so its "today" counts whole threads active today.

## Now Playing

The hub's `media` module follows Windows' media sessions (GSMTC), so any player that shows up in the Windows media flyout works: Spotify, browsers, VLC, Media Player. It shows the artwork (its most vivid colour becomes the accent), title, artist, the player's name, a timeline the island advances itself, and only the transport controls the player enables. Playing is ongoing; paused steps back; nothing playing removes it. `--no-media` turns it off.

## Clipboard

The hub's `clipboard` module keeps everything you copy (text with its formatting, images with transparency, files) in `%APPDATA%\ReUI\clipboard.db` and gives it back from the island. The full spec is [docs/PRD-clipboard.md](docs/PRD-clipboard.md).

- **Win+Alt+V** opens the panel (press again to close): a search field, type tabs with counts (All, Pinned, Links, Images, Text, Code, Colors, Files) and a scrolling row of rich cards. Each card shows the thing itself plus the source app's icon, its age and one fact (size, line count, site, "Cut", "Missing").
- Keys while it's open: type to search, ←/→ select, Enter paste, Shift+Enter paste as plain text, Ctrl+Enter copy only, Alt+1–9 paste the Nth card, Ctrl+P pin, Delete (Ctrl+Z undoes), Tab switches tabs, Esc clears or closes. The app you were in keeps focus, so the paste lands there; if it runs elevated, or you switched apps, the item is left on the clipboard and the island says "Press Ctrl+V". Terminals get Shift+Insert.
- Mouse: click pastes, Shift+click pastes plain text, the star and bin on the card under the pointer pin and delete it (the panel stays open), right-click a card for its menu, the wheel scrolls the row.
- Privacy: copies that apps mark private (password managers) are never read; copies from ignored apps and text that looks like a secret (private keys, API tokens, JWTs) are never stored; history older than 30 days is removed except pinned items; up to 1 000 items / 500 MB. Pause for an hour or clear history from the tray. Nothing leaves the machine except link previews (public sites only, fetched when the card is shown; links that look like reset/sign-in/signed URLs only fetch the site's name).
- Settings: `%APPDATA%\ReUI\clipboard.json` (`hotkey`, `retention_days`, `ignored_apps`, `link_previews`). `--no-link-previews` and `--no-clipboard` turn previews / the module off.

## System stats

The hub's `stats` module shows CPU, GPU, memory, disk and network in a card: **Win+Alt+S**, Tray → System stats, or `reui-bridge stats`. Stats have no OS events, so they're polled, but only while the card is open: opening it starts a sampler thread that reads once a second (like Task Manager), and the island's "collapsed" ends it. With the card closed the module costs nothing (measured). CPU matches Windows 11's Task Manager (`GetSystemTimes`, i.e. `% Processor Time`); GPU is the busiest engine with its processes summed, from PDH's English counter names. Settings: `%APPDATA%\ReUI\stats.json` (`shortcut`, `gpu`, `disk`, `network`, `power`). `--no-stats` turns it off. The full spec and the costs are in [docs/PRD-system-stats.md](docs/PRD-system-stats.md).

## Adding a module

Implement `modules::Module` in `crates/reui-hub/src/modules/`. In `run`, publish activities with `ctx.publish(key, priority, View { accent, compact, minimal, expanded })`, retract them with `ctx.retract(key)`, and handle inputs (button presses, agent hook calls) from `ctx.next()`. Register it in `main.rs`. Modules stay event-driven (no polling where an event exists) and describe meaning only: never sizes, fonts or theme colours.
