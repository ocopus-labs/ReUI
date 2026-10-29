# PRD: Agent moments (the island only when an agent needs you)

Status: M0–M5 built (see §9); real-session checks pending (V1–V3, V5; V4 and V6 answered from source, §10) · Owner: Rohit · Revises the Agents feature in [PLAN.md](../PLAN.md) · Oct 29, 2026

**References** (checked Oct 29, 2026; sources in the appendix):
- **Claude Code hooks:**
  - `PermissionRequest` blocks and is answered with allow / deny (+ `message`, `interrupt`) / allow + `updatedPermissions` / allow + `updatedInput`.
  - **AskUserQuestion and ExitPlanMode are answered from a blocking `PreToolUse` hook:** `permissionDecision: "allow"` plus `updatedInput` (the original `questions` plus `answers`, keyed by question text, multi-select labels joined with ", "). "allow" alone isn't enough.
  - **MCP elicitation** has its own blocking `Elicitation` hook: `action` accept (with `content` keyed by the schema's properties) / decline / cancel; form and URL modes. Input: `mcp_server_name`, `message`, and optional `mode`, `url`, `elicitation_id`, `requested_schema`. No output leaves it to Claude's dialog. **A URL-mode `accept` skips the dialog and Claude never opens the URL** (checked Oct 30, 2026), so the island opens it itself before accepting.
  - `Notification` (`permission_prompt`, `idle_prompt`, `elicitation_dialog`, `agent_needs_input`, …) can't answer anything.
  - Command hooks time out at 600 s by default; a timed-out hook's output is dropped and Claude carries on with its own flow.
- **Codex:**
  - `PermissionRequest` works as today and a deny can carry `message`. `updatedInput` / `updatedPermissions` fail closed.
  - Its question tool (`request_user_input`: `questions[]` with `id`, `header`, `question`, `options`, `isOther`, `isSecret`) is visible through `PreToolUse` but **no hook can answer it**, and there's no hook for plan approval or elicitation (openai/codex#12524 closed as not planned). What `PreToolUse` sees is the model's raw arguments (`tool_name` `request_user_input`, `tool_input.questions[]` with `id`, `header`, `question`, `options`): `isOther` and `isSecret` are set by Codex only after the hooks ran, so they're usually absent. `PostToolUse` fires only when it was answered (a cancelled one ends with the turn).
- **OpenCode:**
  - **The `permission.ask` plugin hook has not been called since v1.3.0 (March 2026)** (still declared in v1.19.5, never triggered); the plugin was rewritten to the event path in M5.
  - The live path: the plugin's `event` handler gets `permission.asked` (`id`, `sessionID`, `permission`, `patterns`, `metadata`, `always`) and `question.asked` (`id`, `sessionID`, `questions[]` with `question`, `header`, `options`, `multiple`, `custom`), then `permission.replied` (`sessionID`, `requestID`, `reply`), `question.replied` (`requestID`, `answers`) or `question.rejected` (`requestID`). The handler is called without being awaited and only for the plugin's own directory. The v1 SDK's event *types* are stale (they still say `permission.updated`); the shapes above are the server's. Replies go through the server: `POST /permission/{id}/reply` `{reply: once|always|reject, message?}` (the v1 client's `postSessionIdPermissionsPermissionId` reaches the deprecated `POST /session/{id}/permissions/{permissionID}` `{response}`), `POST /question/{id}/reply` `{answers: [[label…]…]}` / `/reject`; an unknown or already answered id is a 404. A `reject` also rejects the session's other pending permissions.
  - OpenCode's own prompt stays up meanwhile; whichever answers first wins.
  - Plugins get the **v1** client (no question methods); without a server port it routes requests in-process (`Server.Default().app.fetch`), and its hey-api `_client` (protected in TypeScript, there at run time) posts to any route.

**Inputs:** the agent module today (`modules/agent`), the bridge and installer, and ReUI's existing building blocks:
- `Priority::Decision` already expands the island by itself;
- summoned cards capture typing without taking focus (`clipboard_keys.rs`), and the search field already takes free text and Enter (the timers launcher);
- the pager already walks alternatives in one activity ("1 of 2");
- every failure in the bridge already means "no decision".

## 1. Problem

Today every Claude Code, Codex and OpenCode session is an activity for its whole turn: "Thinking", "Running Bash", "Compacting", then "Done in 2m 14s". With two sessions running, the island is mostly agent status nobody asked for, and it pushes music and timers into the bubble. Meanwhile the moments that do need you are only half covered: a permission request can be answered, but a question with options ("Which framework?"), a plan to approve, or an MCP server's form only says "Has a question for you" and sends you to the terminal.

## 2. Goals

1. **Quiet while agents work.** No activity at all for a session that doesn't need you. The island stays with music, timers and the rest.
2. **Every human-in-the-loop moment answerable in the island**, where the agent allows it: permissions, questions (single, multi-select, typed answers, up to 4 in a row), plan approval with feedback, and MCP forms.
3. **Always know which session is asking.** Each moment shows its session (agent, project, what you asked it) in a slightly wider card, so two agents in two projects never get confused.
4. **Never worse than the terminal.** Fail open on every error and timeout; "Answer in terminal" is always one click; nothing is answered without you.
5. **Costs nothing while agents work.** Status hooks stay `async` and the hub publishes nothing for them, so a busy agent means no island redraws.

## 3. Non-goals (v1)

- Typing the agent's **next prompt** from the island. No agent documents a way to inject one (Claude's `asyncRewake` and Stop-hook `block` continue a turn; they don't deliver a user message).
- A session list or dashboard of what agents are doing (a future "Agents" panel, summoned by hotkey, could show it).
- Answering Codex questions, plans or elicitations: Codex has no hook for them. We show the question and send you to the terminal.
- Showing diffs in full. A permission shows a summary and the first lines.
- Token usage in the island. It moves to Settings → Coding agents (today's and per-session totals already exist in `usage.rs`).

## 4. Scenarios

| # | Scenario | v1 |
|---|---|---|
| S1 | Two sessions working, music playing | The pill shows the music. No agent activity at all |
| S2 | Claude wants to run `cargo test` | The island expands: session strip "ReUI · fix the build", eyebrow "Permission", hero `cargo test` (code chip), detail "Run the tests"; Deny / Always / Allow |
| S3 | Claude wants to edit a file | Hero `main.rs`, detail "Edit · src · +12 −3", a code block with the first changed lines; Deny / Always / Allow |
| S4 | Claude asks "Which framework?" with 3 options | Eyebrow "Question", hero the question, then the options as a list (label + description). Click one: answered, the card closes. "Type an answer" below the list for something else |
| S5 | Claude asks 3 questions at once, one multi-select | The card steps through them ("1 of 3"); multi-select shows checkmarks and a Next button; the last step's button is Send. Back with ‹ |
| S6 | Claude finishes a plan in plan mode | Eyebrow "Plan", hero the plan's title, a scrolling block with the plan; Keep planning / Approve. Keep planning opens a field: "What should change?" and Enter sends it as feedback |
| S7 | An MCP server asks for a project key | Eyebrow "Jira (MCP)", hero the server's message, one field per schema property (a list for enums, Yes/No for booleans); Decline / Submit |
| S8 | An MCP server asks you to open a URL | Hero the message, the host in the detail; Decline / Open link |
| S9 | Codex asks a question | Eyebrow "Question", the question and its options (not clickable), one button: Answer in terminal |
| S10 | Two sessions need you at once | One card with a pager, oldest first. The pill shows the count ("2") beside the logo |
| S11 | You answer in the terminal instead | The card disappears (the bridge went away, or OpenCode's `*.replied` event arrived) |
| S12 | You're typing in your editor when a request arrives | The island expands but doesn't take your keys. Clicking the card (or Win+Alt+A) gives it the keyboard |
| S13 | A turn fails (API error, usage limit) | Alert: session strip, eyebrow "Stopped", the reason; it goes when dismissed or after 15 s |
| S14 | You want the old behaviour | Settings → Coding agents → Show sessions: "While they work" brings back the status activities |

## 5. Experience

### 5.1 What shows, and when

| Moment | Priority | Default | Answerable from the island |
|---|---|---|---|
| Permission | Decision | on | Claude, Codex, OpenCode (M5) |
| Question | Decision | on | Claude, OpenCode (M5); Codex shows it read-only |
| Plan approval | Decision | on | Claude |
| MCP form / URL | Decision | on | Claude |
| Turn failed | Alert | on | Dismiss only |
| Turn finished ("Your turn") | Brief | **off** (Settings) | Open terminal |
| Working (thinking, tool, compacting) | Ongoing | **off** (Settings: "While they work") | — |

An agent's moments wait in one queue, oldest first; the one in front is the activity `moment-<id>` (one per moment, so the next one opens by itself and never inherits a keyboard a click gave the last) and the rest wait behind its pager. Each is gone the moment it's settled: answered here, answered elsewhere, or timed out.

### 5.2 The card: session strip, then three levels

Every moment uses the same anatomy, in the **roomy** width (wider than a timer card, narrower than the clipboard panel; a new token, *T1*):

```
┌─ inset ─────────────────────────────────────────────────────┐
│ (◉) ReUI · fix the build · 4m                    ‹ 1/2 ›    │  session strip: logo, project, prompt, time in turn
│                                                             │
│ PERMISSION                                                  │  eyebrow (accent): the kind of moment
│ ┌──────────────────────┐                                    │
│ │ cargo test --workspace│                                   │  hero: the command / file / question / plan title
│ └──────────────────────┘                                    │
│ Run the tests                                               │  detail: the agent's own description, or facts
│ ─────────────────────────────────────────────────────────── │
│ (    Deny    )  (   Always   )  (     Allow     )           │
└─────────────────────────────────────────────────────────────┘
```

- **Session strip** (quiet, one line): the agent's logo, the project folder, the first line of the prompt that started the turn, and how long the turn has run. It answers "which one?" before you read anything else. The pager sits on it when several moments wait.
- **Eyebrow:** "Permission", "Question", "Plan", "Jira (MCP)", "Stopped". The agent's name is in the strip, not repeated here.
- **Hero:** what you decide about, never a sentence written by us.
- **Compact** (when the card folds after the decision hold): the logo, then the kind in the accent ("Allow?", "Question", "Plan", "Input"). With several waiting, the count in its place ("2"). The pill is a little wider than for other activities only because the text is.

### 5.3 Permission

- Hero per tool, as `describe_tool` does today: commands as a code chip, files by name, hosts for fetches, MCP tool names.
- **Body (new):** a code block under the header for what the hero cuts off: the full command (multi-line scripts), the first changed lines of an edit with `+`/`−` lines, a patch's lines (its file headers only when it touches several). It scrolls with the wheel; it doesn't grow the card past max-height.
- **The hero never repeats the body** (M2 fix: the first build showed a script's first line twice). With the command in the body, the hero is the agent's description ("Test and publish the release") and the detail the shell and line count ("Bash · 4 lines"); without a description, the hero is the shell ("Bash") and the detail the line count. Edits and patches keep the file as hero and "Edit · src · +12 −3" / "Patch · 3 files · +4 −1" as detail; the body has only the changed lines.
- **Secrets are masked** in hero, detail and body with the clipboard's rules (`clipboard::secrets::mask`): known token shapes, long random tokens (also as `KEY=value`), a private key's body.
- Buttons: Deny (Secondary), Always (Secondary, only where the agent supports it and offered a suggestion), Allow (Accent, the tinted main action per §5.8).
- Deny sends a message the agent shows to the model ("Denied from the ReUI island."). **Alt+click Deny** (or Shift+Enter) opens a field: "Tell Claude what to do instead" (Cancel / Deny), sent as the deny `message`. On the wire: the island sends the button's action with the `alt:` prefix (`ACTION_ALT`), only on cards with a session strip.

### 5.4 Question

- One question per step. Hero: the question; detail: its header ("Framework"); then the **choice list**: each option a row with its label and the description in secondary. Single select: a click answers that step. Multi-select: rows toggle with a check, and a Next / Send button appears.
- A field under the list, "Type an answer", when the agent allows free text (Claude: always; OpenCode: `custom`, default true; Codex: `isOther`). Enter sends the text as the answer.
- Several questions: the strip's pager becomes the step ("1 of 3"), ‹ goes back with answers kept; the answer goes to the agent only after the last step. When other moments also wait, the pager pages them instead, the step moves into the detail ("Features · 2 of 3") and Back becomes a button.
- Long lists (E-A6): the list stops at `choice.max-height` (184, about three options with descriptions, which is what `roomy-max-height` 440 leaves after the strip, header, field and buttons; the earlier "four two-line choices" estimate didn't count the field) and scrolls; the keyboard cursor scrolls its option into view.
- **Answer in terminal** (quiet, at the end of the list): replies "no decision" right away, so the agent shows its own prompt.
- `isSecret` (Codex) and anything that looks like a password prompt: never shown, only "Answer in terminal". (Built: question text, labels and descriptions go through the secret mask; a Codex `isSecret` question shows only its header as the hero, no text or options.)
- Questions whose texts aren't distinct (E-A7), or that came without a waiting bridge (older hooks, E-A13): the options show without actions and the only button is "Answer in terminal".

### 5.5 Plan approval (Claude)

- Hero: the plan's first heading (or "Plan"); detail: the plan file's name and how many steps it lists.
- Body: the plan as quiet prose in a scrolling block (headings bold, list markers kept, no other Markdown).
- Buttons: Keep planning (Secondary) and Approve (Primary). Keep planning opens the field "What should change?"; Enter denies with that text as the reason, which Claude reads as feedback. Esc in the field approves nothing and goes back.
- What mode Claude continues in after Approve is Claude's own choice (*V3*); we don't pick auto-accept on the user's behalf.

### 5.6 MCP form and URL (Claude)

- Eyebrow: the server's name + "(MCP)"; hero: the server's `message`.
- Form fields, one step per field when there's more than one: `enum` → choice list, `boolean` → Yes / No choices, `string` / `number` / `integer` → the field (with the schema's `title` as placeholder). Required fields block Submit; a number that doesn't parse shows a quiet note.
- Schemas we can't show (nested objects, arrays, more than 6 fields): only "Answer in terminal".
- URL mode: the host in the detail; Decline / Open link (opens the default browser, http(s) only, and accepts at once: Claude doesn't open a URL a hook accepted, and the server's own completion follows without us).
- As built: a choice step's pick moves on, except on the last step, which waits for Submit; Next shows once the step may be left (filled, or optional); a number that doesn't parse says "Not a number" / "Not a whole number" in the detail; the compact pill says "Input". "cancel" is never sent: leaving a form to the terminal is no output (Claude's own dialog).

### 5.7 Keyboard

- A moment that arrives by itself **never takes the keyboard**: you may be typing in the terminal or editor, and a stray Enter must not allow a command.
- Clicking the card, or **Win+Alt+A** (configurable; "answer"), captures it like the clipboard panel: ↑/↓ move in a choice list, 1–9 pick, Space toggles, Enter is the primary button or the selected choice, Tab moves between the list, the field and the buttons, Esc releases the keyboard; the card stays open until answered or moved away from (once released, the island no longer sees keys).
- Enter never means Allow unless the keyboard was given to the card on purpose.

### 5.8 Visual reference: iOS 17 system alerts

The cards follow the system alerts in the [iOS 17 Dynamic Island Components](https://www.figma.com/design/Q5jcdhxYldk71bDnC5VzIE/iOS17-Dynamic-Island-Components--Community-) kit (Rohit, Oct 29, 2026): "Mobile Data is Turned Off" (`107:858`), "Turn Off Airplane Mode" (`69:398`), "Screen Mirroring" (`93:782`), "Prague Main Train Station" (`107:908`), Incoming Call (`1:2446`), Silent Mode (`43:1076`). What we take from them:

| Kit pattern | Agent card |
|---|---|
| Bare glyph in the accent on the left (no tile), title in primary, a **wrapped** secondary line under it (2 lines) | Permission / question / plan header: the tool's glyph bare in the agent's accent; hero; the description wraps to two lines instead of being cut |
| Two equal capsules under the text, inset 16 from the card's edge, 12 apart: neutral grey ("OK") + **accent-tinted with accent text** for the main action ("Settings") | Deny (grey) / Allow (agent-accent tint). The main action is the tinted one, not a solid white capsule |
| One full-width capsule ("Open Settings", grey; "Stop Mirroring", blue tint; "End Route", red tint with red text) | Plan: Keep planning / Approve as a pair; a lone "Answer in terminal" (Codex questions) as one grey full-width capsule; destructive actions red-tinted |
| Incoming call: two 52 circles inline at the right of an 86-tall card (red Decline, green Accept) | A short permission with no body and no "Always" (Codex, OpenCode): Deny / Allow as two inline circles (✕ red tint, ✓ accent tint), so the card stays 86 tall |
| Silent Mode: one small capsule inline at the right ("Unmute") | A single secondary action that belongs beside the text (Open terminal on a failed turn) |
| Compact: glyph left, short accent text right ("Silent", "On") | Compact: logo left, the kind in the accent right ("Allow?", "Question") |

Differences we keep: the session strip (iOS alerts don't need to say which app is asking; we have several sessions), and our concentric radii (button radius = card radius − inset where a button sits at the inset).

### 5.9 What it never does

- Answers anything by itself, or picks Always on its own.
- Shows a session that doesn't need you (unless Settings asks for it).
- Keeps an agent waiting past its own hook timeout: every wait ends with "no decision" before the agent would kill the hook.
- Shows secrets: `isSecret` questions, and command text matching the clipboard's secret patterns, are masked.

## 6. Platform details

| Need | How | Notes |
|---|---|---|
| Claude questions and plans | A **second, blocking** `PreToolUse` entry with matcher `AskUserQuestion\|ExitPlanMode`, timeout 300 s, next to the existing async `*` one, run as `reui-bridge claude ask`. The bridge waits when `hook_event_name` is `PermissionRequest` or `Elicitation`, or `PreToolUse` for those two tools **only when run with `ask`** (correction: the async `*` entry gets the same event, and without the flag its bridge would wait too) | The async `*` entry still fires for them; the hub makes the two one moment by `tool_use_id` (whichever comes first: the blocking call upgrades a read-only moment in place; an async duplicate of one waiting or just settled is ignored). A `PermissionRequest` for either tool gets "no decision" at once. While this hook waits, Claude's own dialog isn't shown yet (*V1*), hence "Answer in terminal" and a shorter wait than for permissions |
| Claude answers | `{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"allow","updatedInput":{"questions":[…as received…],"answers":{"<question>":"<label>, <label>"}}}}`. Plans: `allow` + `updatedInput` echoing `plan`, or `deny` + `permissionDecisionReason` | Typed answers go in as the answer text (*V2*) |
| Claude elicitation | Blocking `Elicitation` hook (matcher `*`, 300 s); reply `{"hookSpecificOutput":{"hookEventName":"Elicitation","action":"accept","content":{…}}}` or `decline` | `ElicitationResult` isn't needed. Installed since M2; until M4 the hub answers "no decision" at once (no output: Claude's own dialog, per the hooks reference) |
| Codex | `PermissionRequest` as today, plus `message` on deny. A `PreToolUse` with `tool_name` `request_user_input` publishes the read-only question; its `PostToolUse` (or `Stop`) retracts it | Matching `^request_user_input$` in the existing async `*` hook is enough; nothing new to install |
| OpenCode | The plugin drops `permission.ask` and listens to `permission.asked`, `question.asked`, `permission.replied`, `question.replied`, `question.rejected`. It forwards the first two as `PermissionRequest` / a new `Question` event and waits for the bridge; it answers with the plugin's `client` (`postSessionIdPermissionsPermissionId` with `once`/`always`/`reject`; a deny with the user's message through `_client.post` to `/permission/{id}/reply`; questions through `client._client.post` to `/question/{id}/reply`, *V4*). Without `_client`, `fetch` to the server's port when it has one; else the question shows read-only | The `*.replied` / `question.rejected` events stop the waiting bridge, so `HookGone` retracts the card. A reply that loses the race gets NotFound, which is ignored. The hub rewrites an older installed plugin (user scope) on start (`reui-plugin: 2` marker) |
| Waits | Hub deadline < bridge deadline < hook timeout, per kind: permissions 100 / 110 / 120 s (today), questions, plans and forms 280 / 290 / 300 s | Codex and OpenCode keep their current limits |
| Answer in terminal | The hub replies `None` at once; the card goes | |
| Open terminal (finished, failed, Codex questions) | The bridge sends its parent process chain with the event; the hub walks it to the first process with a visible top-level window (Windows Terminal, the VS Code window, conhost) and activates it with `AllowSetForegroundWindow` from the island's click | Can reach the window, not the tab (*V5*) |
| Session strip | The session's project, first prompt line and turn start, which the module already tracks for every session, now kept but not published | |

**Engineering:**
- `modules/agent/`:
  - `mod.rs`: the loop; sessions are still tracked (needed for the strip), only published per the "Show sessions" setting;
  - `moments.rs`: the pending moments (permission, question, plan, form), their steps and answers; pure, tested;
  - `answer.rs`: the reply JSON per agent and kind; pure, tested against the documented shapes;
  - `view.rs`: the card for each moment, the strip, compact;
  - `schema.rs`: an MCP `requested_schema` into fields, or "can't show";
  - `settings.rs`: `%APPDATA%\ReUI\coding.json` (`show_sessions`: `needed` | `working`, `show_finished`, `hotkey`).
- `reui-bridge`: wait for the new events; per-kind deadline; the parent process chain; the installer adds the blocking `PreToolUse` and `Elicitation` entries (and removes them).
- `opencode-plugin.js`: the event-based rewrite.
- `reui-island`: the components below, keyboard capture on click, Win+Alt+A.
- Settings → Coding agents: Show sessions, Show finished turns, the hotkey, and today's token usage.

## 7. Protocol (v18)

New pieces, all semantic:

- **`ExpandedView.context: Option<Context>`**, the session strip: `Context { mark: Element, title: String, facts: Vec<String> }` ("ReUI", ["fix the build", "4m"]). The surface draws it as one quiet line above the header, and the pager on it.
- **`ExpandedView.wide: bool` → `size: CardSize { Regular, Roomy, Wide }`.** Clipboard uses Wide, agent moments Roomy; the widths are tokens.
- **`Row::Choices { options: Vec<Choice>, multi: bool }`**, `Choice { label, detail: Option<String>, selected: bool, action }`. A click sends `action`; with `multi`, the module answers by re-publishing with `selected` flipped.
- **`Row::Block { text: String, kind: BlockKind { Prose, Code, Diff } }`**: a body of text that scrolls inside the card. `Diff` lines starting with `+` / `−` take the Positive / Negative tones.
- **The answer field reuses `Row::Search`** (placeholder, glyph, `search:<text>` as you type, `submit` on Enter), as the timer launcher does.
- **`Button.role` gains nothing**; "Answer in terminal" is a `Plain` text button (`Secondary` when it's the only one).
- **`ACTION_ALT` (`"alt:"`)** (added in M2, no wire change): a button pressed for its second meaning, Alt+click or Shift+Enter (the first non-`Plain` button), only on cards with a `context`.
- Keyboard capture for any card with `Choices` or a field, or a moment (a `context`) with buttons, granted on click or by the hub's `Focus` (Win+Alt+A). Enter presses the `Primary` button, else the `Accent` one.

## 8. Edge cases

| ID | Case | Required behaviour |
|---|---|---|
| E-A1 | Hub not running, crashes or restarts mid-question | The bridge times out or the pipe breaks: no decision, the agent asks itself |
| E-A2 | Answered in the terminal (Claude) | The hook gets cancelled or exits; `HookGone` retracts the card |
| E-A3 | Answered in OpenCode's own prompt | `permission.replied` / `question.replied` retracts it; our late reply's NotFound is ignored |
| E-A4 | Two moments from one session (permission while a question waits) | Both wait in the pager, oldest first |
| E-A5 | Session ends while a moment waits | The bridge goes away with it; the card goes |
| E-A6 | Question with more than 4 options, or long labels | The list scrolls inside the card; labels are cut with an ellipsis, descriptions wrap to one more line |
| E-A7 | Questions identical in text (answers are keyed by text in Claude) | Allowed by Claude's schema only if distinct; if not, "Answer in terminal" only |
| E-A8 | Plan longer than the card | The block scrolls; the wheel scrolls it, not the pager |
| E-A9 | A fullscreen game is in front | Hidden as usual; the moment waits, and its timeout still applies (the agent then asks in the terminal) |
| E-A10 | ReUI paused or the agent module off | No hooks wait: the hub answers `None` at once |
| E-A11 | The island is in "Only when it needs me" mode | Moments are exactly what it needs you for: they show |
| E-A12 | `--no-agents` | The module doesn't run; the bridge finds no handler and gets `None` |
| E-A13 | Upgrade from a build whose hooks are installed without the new entries | Settings → Coding agents reads the old hook set (no blocking `AskUserQuestion` entry) and offers "Update hooks"; until then, questions say "Answer in terminal". (Built differently from the draft: the hub needs no check of its own, since a question that arrives without a waiting bridge is read-only by itself.) |

## 9. Milestones

| # | What | Done when | State |
|---|---|---|---|
| M0 | Protocol v18 and island components: the strip, roomy size, `Choices`, `Block`, keyboard on click; the demo module shows a permission, a 3-step question and a plan | Demo cards render and answer with mouse and keyboard; the design review sheet (`tools/claude-states.ps1`) updated | Built |
| M1 | Quiet sessions: nothing published while working; Show sessions / Show finished settings; the permission card rebuilt with the strip and the body block; deny with a message | Two real Claude sessions in this repo: nothing shows until a permission | Built; deny with a message, several moments paged (S10), long choice lists and secret masking added with M2 |
| M2 | Claude questions: the blocking `PreToolUse` hook, steps, multi-select, typed answers, Answer in terminal; installer and bridge sync | *V1*, *V2* answered; a real AskUserQuestion answered from the island lands in Claude | Built; the bridge's output checked against the documented shape with a recorded-shape event through a sandboxed hub. Pending: V1, V2 and a real session |
| M3 | Plan approval with feedback | A real plan approved, and one sent back with feedback, from the island | Built (Approve echoes the input; Keep planning denies with the reason; no permission mode set). Pending: a real plan, V3 |
| M4 | MCP forms and URLs | A test MCP server's form (enum, boolean, string) and URL request answered | Built (`schema.rs`, `form.rs`, `link.rs`); a form (enum, boolean, string, integer) and a URL request in the documented shapes answered through a sandboxed hub and island, the bridge's output exactly the hooks reference's. Pending: a real MCP server's elicitation in Claude Code, Open link in a real session |
| M5 | OpenCode rewrite (permissions work again, questions), Codex read-only questions, Open terminal | Real OpenCode 1.18 permission and question answered; Codex question shown and retracted | Built. The real plugin ran against a fake OpenCode client through the real bridge, sandboxed hub and island (permission → `always` on the v1 route, question → `/question/{id}/reply`, `permission.replied` retracts); Codex's question shown, paged and retracted by its `PostToolUse` with recorded-shape events. Pending: real OpenCode 1.18+ and Codex sessions, Open terminal in Windows Terminal / VS Code / conhost (*V5*) |

## 10. To verify

| ID | Question | How | Found |
|---|---|---|---|
| V1 | Does Claude show its own question / plan dialog while a blocking `PreToolUse` hook waits, or only after? Same for `PermissionRequest` (open in PLAN.md) | `claude --debug` with a hook that sleeps 30 s | Open |
| V2 | How a typed ("Other") answer is passed in `answers`: as plain text? | Try it; read the transcript entry Claude writes | Open |
| V3 | What permission mode Claude continues in after a hook approves ExitPlanMode; whether `updatedPermissions` `setMode` is honoured there | Try it, compare `permission_mode` on the next event | Open |
| V4 | Whether the OpenCode v1 plugin client reaches `/question/{id}/reply` through `_client.post` without a server port | Try in the TUI; fall back to `fetch` when a server port is up, else read-only | From v1.19.5's source (Oct 30): yes, by reading. The plugin's client is the v1 SDK built with `fetch: Server.Default().app.fetch` when no port is up, so every request runs in-process; `_client` is hey-api's client (`post` fills `{requestID}` from `path`, JSON body). Still to see in a real TUI |
| V5 | Finding the terminal window from the bridge's parent chain for Windows Terminal, VS Code's terminal and conhost | Spike | Built as: nearest ancestor with a visible, unowned top-level window, Explorer skipped; else a visible console the bridge shares. Untried with real terminals |
| V6 | Whether Codex's `PreToolUse` fires for `request_user_input` in the released build (it does on `main`) | Real Codex session in plan mode | On `main` (Oct 30): yes, `tool_name` `request_user_input`, the model's raw arguments as `tool_input` (no `isOther` / `isSecret`); the handler only runs in plan mode for the root agent. Still to see in a released build |

## 11. Open questions

- Should "Turn finished" be on by default? This draft says off, per "only when it needs you".
- Win+Alt+A as the answer hotkey, or reuse the island's click only?
- Is "Stopped" (a failed turn) a need? The agent can't go on without you, so this draft shows it as an Alert.

## Appendix: sources

- Claude Code hooks reference: https://code.claude.com/docs/en/hooks (PermissionRequest, PreToolUse "Tools that require user interaction", Elicitation, Notification, timeouts); MCP elicitation: https://code.claude.com/docs/en/mcp; permission modes: https://code.claude.com/docs/en/permission-modes
- Codex hooks: https://learn.chatgpt.com/docs/hooks; `codex-rs/hooks/src/events/permission_request.rs`, `codex-rs/protocol/src/request_user_input.rs`, `codex-rs/core/src/tools/handlers/request_user_input_spec.rs`, `codex-rs/core/src/tools/registry.rs` (main @184c9c5, read Oct 30, 2026); openai/codex#12524
- OpenCode (anomalyco/opencode v1.19.5, commit 810c0eba; sst/opencode redirects there): `packages/schema/src/v1/question.ts`, `packages/schema/src/v1/permission.ts`, `packages/opencode/src/server/routes/instance/httpapi/groups/{question,permission,session}.ts`, `packages/opencode/src/plugin/index.ts`, `packages/opencode/src/tool/edit.ts` (edit permissions: `metadata.filepath`, `metadata.diff`), `packages/sdk/js/src/gen/sdk.gen.ts` (v1 client); issues #47654, #47674
