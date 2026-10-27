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
| `crates/reui-bridge` | The program coding agents run as a hook; forwards events to the hub, answers permission requests, and installs/uninstalls the hooks |
| `spikes/composition-pill` | Phase 0a renderer spike and its measurements |

## Run

```
cargo build --release
target\release\reui-hub.exe --demo
```

The hub starts the island plus the coding-agent and Now Playing modules (`--no-agents`, `--no-media` to skip them). `--demo` enables a module that shows now-playing, a 5-minute timer and, every 25 s, an agent permission request that takes over the pill (it times out and fails open after 20 s). Hover to peek, click to expand, right-click the pill to quit ReUI. `--no-island` runs the hub alone so you can start `reui-island.exe` yourself.

## Coding agents (Claude Code, Codex, OpenCode)

One `agent` module per agent shows each session (thinking, the current tool, waiting on you, done with time and tokens, failed), lets you answer permission requests from the pill, and counts token usage. Claude Code offers Deny / Always / Allow; Codex and OpenCode take Deny / Allow (their hooks don't accept "always").

```
%LOCALAPPDATA%\ReUI\bin\reui-bridge.exe install claude   [--scope user|project|local] [--project DIR]
%LOCALAPPDATA%\ReUI\bin\reui-bridge.exe install codex    [--scope user|project]   then trust the hooks in Codex: /hooks
%LOCALAPPDATA%\ReUI\bin\reui-bridge.exe install opencode [--scope user|project]   loaded on OpenCode's next start
%LOCALAPPDATA%\ReUI\bin\reui-bridge.exe uninstall <agent> [same options]
```

| Agent | What gets installed | Usage from |
| --- | --- | --- |
| Claude Code | Hooks in `.claude/settings(.local).json` (exec form) | Session transcripts |
| Codex | Hooks in `~/.codex/hooks.json` (command line; Codex has no `args`) | `~/.codex/state_N.sqlite` → `threads.tokens_used` |
| OpenCode | Plugin `~/.config/opencode/plugins/reui.js` | `~/.local/share/opencode/opencode.db` → `session` tokens |

The installer merges into existing settings (other tools' hooks are untouched), keeps the original as `*.reui-backup`, and uninstall removes exactly its own entries. The hub keeps the copy in `%LOCALAPPDATA%\ReUI\bin` in step with itself when it starts, so a protocol change never silently disconnects agents.

Safety: status events are `async` (agents never wait for them). Only permission requests wait, and every failure (hub not running, timeout after 100 s, bridge killed) means "no decision", so the agent asks you itself as usual. The bridge prints nothing except a decision and always exits 0.

Usage numbers are **fresh tokens** (cache reads, usually ~97 % of all tokens at ~10 % of the price, are left out). Codex records one running total per thread, so its "today" counts whole threads active today.

## Now Playing

The hub's `media` module follows Windows' media sessions (GSMTC), so any player that shows up in the Windows media flyout works: Spotify, browsers, VLC, Media Player. It shows the artwork (its most vivid colour becomes the accent), title, artist, the player's name, a timeline the island advances itself, and only the transport controls the player enables. Playing is ongoing; paused steps back; nothing playing removes it. `--no-media` turns it off.

## Clipboard

The hub's `clipboard` module keeps everything you copy (text with its formatting, images with transparency, files) in `%APPDATA%\ReUI\clipboard.db` and gives it back from the island. The full spec is [docs/PRD-clipboard.md](docs/PRD-clipboard.md).

- **Win+Alt+V** opens the panel (press again to close): a search field, type tabs with counts (All, Pinned, Links, Images, Text, Code, Colors, Files) and a scrolling row of rich cards. Each card shows the thing itself plus the source app's icon, its age and one fact (size, line count, site, "Cut", "Missing").
- Keys while it's open: type to search, ←/→ select, Enter paste, Shift+Enter paste as plain text, Ctrl+Enter copy only, Alt+1–9 paste the Nth card, Ctrl+P pin, Delete (Ctrl+Z undoes), Tab switches tabs, Esc clears or closes. The app you were in keeps focus, so the paste lands there; if it runs elevated, or you switched apps, the item is left on the clipboard and the island says "Press Ctrl+V". Terminals get Shift+Insert.
- Mouse: click pastes, Shift+click pastes plain text, right-click a card for its menu, the wheel scrolls the row.
- Privacy: copies that apps mark private (password managers) are never read; copies from ignored apps and text that looks like a secret (private keys, API tokens, JWTs) are never stored; history older than 30 days is removed except pinned items; up to 1 000 items / 500 MB. Pause for an hour or clear history from the tray. Nothing leaves the machine except link previews (public sites only, fetched when the card is shown; links that look like reset/sign-in/signed URLs only fetch the site's name).
- Settings: `%APPDATA%\ReUI\clipboard.json` (`hotkey`, `retention_days`, `ignored_apps`, `link_previews`). `--no-link-previews` and `--no-clipboard` turn previews / the module off.

## Adding a module

Implement `modules::Module` in `crates/reui-hub/src/modules/`. In `run`, publish activities with `ctx.publish(key, priority, View { accent, compact, minimal, expanded })`, retract them with `ctx.retract(key)`, and handle inputs (button presses, agent hook calls) from `ctx.next()`. Register it in `main.rs`. Modules stay event-driven (no polling where an event exists) and describe meaning only: never sizes, fonts or theme colours.
