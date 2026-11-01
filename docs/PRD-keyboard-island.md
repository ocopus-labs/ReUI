# PRD: The island from the keyboard (a round trip)

Status: built · Owner: Rohit · From [research-features.md](research-features.md) §5d #4 · Nov 1, 2026

**References:**
- **The ask** (r/SideProject, Oct 31, 2026): open the island, read an item, act on it, close it, and be back in the exact app and control you were in, without the mouse. "Predictable focus and an obvious Escape path would matter more to me than another indicator."
- **macOS:** notch apps open from a global shortcut (NotchNook, boring.notch); the panels take key focus, and Esc closes them and gives focus back to the previous app. That works on macOS because a non-activating panel can still be key; on Windows, taking focus would move the caret.
- **ReUI today:**
  - Cards opened by their own shortcut (Win+Alt+V, Win+Ctrl+T, Win+Alt+S) are *summoned*. A low-level keyboard hook reads the keys they use, and the app in front never loses focus, so a paste lands where the caret was (`clipboard_keys.rs`).
  - A coding agent's card takes the keyboard only after a click (*engaged*), so a stray Enter in an editor can't allow anything (PRD agent moments §5.7). Enter then presses the tinted button, Shift+Enter denies with a message, Esc gives the keys back.
  - The idle pill's row of places opens on a click and reads only Esc.
  - A summoned card with nothing to type into or pick (System stats) reads no keys, so Esc doesn't close it.

## 1. Problem

Three gaps stop a keyboard-only round trip:
1. **No way in.** Nothing opens the island itself: each card has its own shortcut, and a coding agent's request needs a click before it takes the keyboard.
2. **No way around.** The row of places can't be walked or opened from the keyboard.
3. **No sure way out.** Esc closes some summoned cards but not others.

## 2. Goals

1. **One shortcut in** (Win+Alt+N by default, Settings → General → Open the island) that opens *what's waiting* with the keyboard on it:
   1. a decision (a permission, a question, a plan), as if it had been clicked;
   2. else the card in front (music, a timer, a device), summoned as its own shortcut would;
   3. else, with nothing happening, the row of places, with the first one under a keyboard cursor.
2. **Walk the places:** ← / → (↑ / ↓) move, Home / End jump, Enter opens the place (its card then opens summoned, with its own keys).
3. **One way out:** Esc closes whatever a shortcut opened and gives the keys back; on a decision (which can't close unanswered) it gives the keys back and leaves the card. The shortcut pressed again does the same.
4. **Focus never moves.** The app in front keeps focus throughout (the existing hook), so leaving puts you back in the same control with the caret where it was.
5. **Costs nothing at rest.** A hotkey registered by the hub; the hook only while the island has the keyboard.

## 3. Non-goals

- Tab between controls inside a card (buttons by Tab). Enter, Shift+Enter, digits and arrows cover what cards ask.
- Opening a particular activity that isn't in front, by number. The wheel and the view switcher cover browsing.
- Giving the island real keyboard focus (it would take the caret, and break paste-back).

## 4. Scenarios

| # | Scenario | Keys | Result |
|---|---|---|---|
| S1 | Typing in VS Code, Claude asks to run `cargo test` | Win+Alt+N, Enter | allowed; the caret is still in VS Code where it was |
| S2 | Same, but you want to say why not | Win+Alt+N, Shift+Enter, type, Enter | denied with the message |
| S3 | A question with three options | Win+Alt+N, 2 | the second option is the answer |
| S4 | Changed your mind | Win+Alt+N, Esc | the card stays (still unanswered), the keys go back to VS Code |
| S5 | Music in front, nothing waiting | Win+Alt+N | the music card opens; Space/←/→ do nothing new; Esc closes it |
| S6 | Nothing happening | Win+Alt+N, →, →, Enter | the row of places opens on Clipboard, moves to Timers, opens the timer field (typing goes to it); Esc closes it |
| S7 | A view open (Clipboard) and a permission arrives (the dot) | Win+Alt+N | the permission comes forward with the keyboard, as a click on the dot would |
| S8 | Shortcut pressed twice | Win+Alt+N, Win+Alt+N | back where you were |
| S9 | Visibility *When needed*, nothing happening | Win+Alt+N | the island shows for the row, as for any summoned card |
| S10 | ReUI paused or hidden with Win+Alt+I, nothing waiting | Win+Alt+N | nothing (decisions still open, since they show anyway) |
| S11 | A fullscreen game | Win+Alt+N | nothing |
| S12 | An elevated app in front | Win+Alt+N, Enter | Windows doesn't let a medium-integrity hook see keys typed into an elevated window: the card opens, the key goes to the app. Documented, not fixed |

## 5. Rules

### 5.1 What opens

- **A decision first:** the first activity with priority Decision whose card can use the keyboard (a session strip and buttons, a list or a field). If it isn't in front (a summoned place is), it comes forward and the place stops being summoned, exactly as clicking the switcher's waiting dot does. It opens *engaged*: the same keys as after a click.
- **Else the front card**, when it has one: summoned (`focus`), as its own shortcut opens it.
- **Else, idle,** the row of places with the cursor on the first place, unless ReUI is paused or hidden by the user.
- A fullscreen app in front: nothing.

### 5.2 Keys

- Engaged decision: as today (agent moments §5.7). Esc gives the keys back, leaving the card.
- Summoned card with a field or a list: as today.
- **Summoned card with neither:** now reads Esc alone, which closes it. Every other key goes to the app. (Before, it read nothing.)
- **Row of places opened from the keyboard:** reads Esc, the arrows, Home, End and Enter; typing goes to the app. Opened with a click, it still reads Esc alone (so arrows in your editor aren't taken by a row you clicked open).
- **The shortcut again** while the island has the keyboard: as Esc.

### 5.3 The shortcut

- `open_shortcut` in `settings.json`, default `Win+Alt+N`; `""` turns it off. Registered by the hub's `island` module beside `hide_shortcut`; Settings records it like the others and refuses a chord another ReUI shortcut already uses.
- `reui-bridge open` sends the same request, for scripts and tests.

## 6. Design

- **Protocol:** `HubMsg::OpenIsland`.
- **Hub:** `modules/hide.rs` registers both shortcuts and relays `OpenIsland`; `Hub` relays island requests as `HubMsg` on one broadcast channel.
- **Island:** `pill/summon.rs` `open_from_shortcut` (the order in §5.1) and `let_go`; `update_capture` reads Esc for summoned cards without keys; `features/clipboard_keys.rs` `Limit` (All, Escape, Navigation; tested); `pill/idle.rs` opens the row from the keyboard and walks it (`places_step`, pure, tested).
- **Settings:** shortcuts are named (`Shortcut`: Clipboard, Timers, Stats, HideIsland, OpenIsland) rather than one per file, so the island's file can hold two; General shows *Open the island* above *Hide the island*.
- **Bridge:** `open`.

## 7. Milestones

- **M1 (this change):** all of the above.
- **Later, if asked:** Tab through a card's buttons; a number to open the nth activity.

## 8. Tests

- Unit: the shortcuts read from settings.json (both defaults, off, custom); `Limit` passes only its keys; walking the row (ends, wrap-less, Enter on the cursor or the first); Settings records each island shortcut on its own row and refuses a clash with the other.
- Checked on this machine with a sandboxed hub: idle, `reui-bridge open` opens the row of places with the first place marked, and again closes it; with a permission waiting, `reui-bridge open` keeps the card open.
- **By hand (the hook ignores injected keys, so this can't be scripted):** the S1–S8 round trips in VS Code, Windows Terminal, a browser field and Notepad, checking the caret is where it was after Esc; S12 in an elevated terminal; whether Win+Alt+N is free on a stock Windows 11 (Settings says so when recording if it isn't).
