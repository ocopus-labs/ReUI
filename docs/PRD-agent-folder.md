# PRD: The working folder on an agent's card

Status: built · Owner: Rohit · From [research-features.md](research-features.md) §5d #1 · Nov 1, 2026

**Inputs:**
- [PRD-agent-moments.md](PRD-agent-moments.md): every moment (permission, question, plan, MCP form) is one card with a *session strip* above its header: the agent's mark, a title, then facts (what you asked, how long the turn has run). `Context { mark, title, facts }` in the protocol; the island draws the title in `label.secondary` and cuts what doesn't fit.
- Every hook event from Claude Code, Codex and OpenCode carries the session's `cwd` (Codex as `\\?\D:\…`, OpenCode's plugin with forward slashes).
- Today the title is `project_name(cwd)`, the folder's last part only, fixed when the session is first seen (`modules/agent/events.rs`).

## 1. Problem

A permission card names the project, not where it is. Two sessions in `D:\work\api` and `D:\old\api` both say **api**, and a command that is harmless in a scratch folder isn't in a production checkout. The one person who asked (r/SideProject, Oct 31, 2026) put it as: "Approving from a small pill makes it easy to say yes to something I didn't read." The card already shows the whole command and an edit's diff; what's missing is *where*.

A second, quieter gap: the title is taken from the first event, but Claude Code reports the session's current `cwd` on every event, and it moves when the agent runs `cd`. A command runs in today's folder, not the one the session started in.

## 2. Goals

1. **Where, on every card that asks.** The strip's title is the session's working folder as a path, so two sessions with one project name read differently.
2. **Readable at a glance.** The home folder is `~`. A long path is shortened from the middle; the drive (or share) and the project's own folder always stay.
3. **Current.** The folder follows the last `cwd` the agent reported.
4. **Nothing new in the protocol.** The path is the strip's title, a string the island already draws. No new element, no new row, no change in the island.

## 3. Non-goals

- The path in the compact pill. The pill says *that* something is waiting; the card says what and where.
- A clickable path (open in Explorer). "Open terminal" already brings the agent's window forward.
- Showing it on session activities that don't ask anything (Thinking, Done). They have no strip.
- Telling the user the folder changed since the last card. Rare, and the card shows the folder as it is now anyway.

## 4. Scenarios

| # | Scenario | Strip |
|---|---|---|
| S1 | Claude in `C:\Users\me\code\api` asks to run `cargo test` | `~\code\api · fix the build · 4m` |
| S2 | A second Claude session in `D:\old\api` asks at the same time | its page of the pager says `D:\old\api · …`; the first says `~\code\api · …` |
| S3 | Codex in `\\?\D:\ocopus` asks | `D:\ocopus · …` |
| S4 | `D:\work\clients\northwind\2026\backend\services\billing-api` | `D:\…\backend\services\billing-api` |
| S5 | A share, `\\build-01\src\teams\payments\gateway\service` | `\\build-01\…\gateway\service` |
| S6 | The agent ran `cd packages\web` and then asks | the strip says `~\code\api\packages\web` |
| S7 | An event with no `cwd` (a test event) | the agent's name, as today |

## 5. Rules

### 5.1 Which path

- The last non-empty `cwd` from any of the session's hook events. Until one arrives, the project name (the folder's last part) or the agent's name, as today.
- Cleaned: `\\?\` dropped, `/` made `\`, no trailing separator (a drive root keeps its `\`: `D:\`).

### 5.2 Home as `~`

- When the path is the user's home folder or inside it (`USERPROFILE`; the sandbox's `home\` under `REUI_CONFIG_DIR`), that part becomes `~`. Compared without case, as Windows does, and only at a folder boundary: `C:\Users\Rohitk` isn't inside `C:\Users\Rohit`.

### 5.3 Shortened from the middle

- At most **36 characters** (`FOLDER_CHARS`). The strip also holds what you asked, so the path takes about half of a roomy card and the prompt still shows. A character count, not a width: the island decides widths and still cuts whatever doesn't fit.
- Over that: keep the first part (`D:`, `~`, `\\server`) and as many of the last folders as fit, joined by one `…`. The last folder is always whole, even when it alone is longer: it's the name you know the project by.
- A path with nothing to drop (`D:\one-very-long-name`) is left whole for the island to cut.

### 5.4 Where it shows

- The strip of every agent card: permissions, questions, plans, MCP forms and links, Codex's read-only questions, a failed turn's card.
- `--demo`'s moments show `~\code\ReUI`.

## 6. Design

- **Hub, `modules/agent/folder.rs`** (new, pure, tested): `shown_folder(cwd, home) -> Option<String>`: clean, `~`, shorten.
- **`Session.folder`**: set from every event that carries a `cwd` (`events.rs`).
- **The strip** (`view.rs` `context`): title = `session.folder`, else `session.project`.
- **Protocol, island, tokens:** unchanged.

## 7. Milestones

- **M1 (this change):** the folder module and its tests, the session's folder, the strip, the demo, the guide.

## 8. Tests

- Unit (`folder.rs`): home as `~` (any case; `~` alone; a sibling folder that only starts the same isn't home); Codex's `\\?\`, forward slashes, trailing separators, a drive root, empty input; long paths keep their drive and last folders within the cap; a long last folder stays whole; shares keep their server.
- By hand: `tools/claude-states.ps1` (cwd `D:\ReUI`), and two Claude sessions in folders with the same name answering at once (the pager's pages differ by their strip).
