# PRD: Hidden from screen shares

Status: built · Owner: Rohit · From [research-features.md](research-features.md) §5e #1 · Nov 1, 2026

**References:**
- **`SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE)`** (Windows 10 version 2004 and later): "The window is displayed only on a monitor. Everywhere else, the window does not appear at all." It applies to a top-level window of the calling process and can be switched at any time. Before 2004 the value is rejected; the older `WDA_MONITOR` leaves a black rectangle in captures instead ([SetWindowDisplayAffinity](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowdisplayaffinity)).
- What it covers: anything that captures through DWM: Windows.Graphics.Capture (Teams, the browser's tab and screen pickers, Snipping Tool), Desktop Duplication (Zoom, OBS's display capture), GDI `BitBlt` of the screen (PrintScreen, older tools). Not a camera pointed at the screen, not a remote session drawn by the remote machine.
- **macOS** apps do the same with `NSWindow.sharingType = .none`; password managers and some notch apps use it so their panels never appear in shares.
- **[PRD-privacy-indicators.md](PRD-privacy-indicators.md) §6:** ReUI can tell when the screen is captured through Windows.Graphics.Capture, but Desktop Duplication and GDI leave no record. So "while sharing" can't be detected reliably, and this feature doesn't try.

## 1. Problem

The island is on top of everything, so it's in every screen share and recording. A permission card shows the command and the project's folder, the clipboard panel shows what you copied (a password, a customer's email), a notification shows a message. In a Teams call or a recorded demo, all of that goes out to whoever is watching. Today the only way to avoid it is to pause ReUI before sharing and remember to resume it.

## 2. Goals

1. **Private cards never reach a capture,** by default, with nothing to remember: while a card that may hold private text is in front, the island is left out of every capture Windows makes.
2. **You still see everything.** Exclusion only changes what captures get; the island on your screen is unchanged and works as usual.
3. **Only then.** The rest of the time (music, a timer, the resting pill) the island is captured like any window, so a recording of ReUI itself, or a screenshot of the island, still works.
4. **Your choice.** Settings → General: hide private cards (default), hide the whole island, or hide nothing.
5. **Costs nothing.** No watching, no polling: decided when the card in front changes.

## 3. Non-goals

- Detecting a share or recording and changing what the island shows to *you* (closing cards while sharing). The privacy module's screen indicator already tells you something is capturing.
- Blurring or masking parts of a card in captures. Windows excludes whole windows only.
- Settings' own window. It shows settings, not content.
- A camera filming the screen, or a remote-desktop viewer.

## 4. Scenarios

| # | Scenario | Your screen | What the call or recording gets |
|---|---|---|---|
| S1 | Sharing in Teams, you press Win+Alt+V | the clipboard panel | what's under the island; the island isn't there |
| S2 | Sharing, Claude asks to run a command | the permission card | nothing of the island |
| S3 | Sharing, music playing | the music pill | the music pill |
| S4 | Recording a demo with OBS, a copy toast shows | the toast | no island for the toast's seconds, then the pill again |
| S5 | Snipping Tool screenshot of an open clipboard panel | — | the screenshot doesn't have it |
| S6 | Setting *Island* | everything as usual | never the island |
| S7 | Setting *Nothing* | everything as usual | everything, as before this change |
| S8 | `--demo` recording for the website | the demo's agent card | the card: the demo module isn't private, so recordings of ReUI still work |
| S9 | Windows 10 1909 | as usual | as usual (the call fails; logged once per change) |

## 5. Rules

### 5.1 What's private

A module declares it once, for all its activities (`ModuleContext::declare_private`); the hub stamps `Activity::private`. Modules describe meaning, so the module says *what* it is (it may hold private text), and the surface decides what to do with it.

| Module | Why |
|---|---|
| Coding agents | commands, diffs, prompts, folders, an MCP form's fields |
| Clipboard | whatever was copied, in the panel and the toast |
| Notifications | other apps' messages |
| Shelf | held text, links and file names |
| Terminal commands | a command line can hold a host, a path or a token |

Not private: Now Playing (the song is on screen anyway), timers, reminders (short, typed by you; revisit if asked), calendar (a meeting's title, revisit if asked), devices, volume, stats, privacy indicators, downloads, screenshots, updates, demo, onboarding.

### 5.2 When the island is left out

- *Private cards* (default): while the **front** activity is private: its compact pill, its peek or its open card. The bubble beside it shows only a glyph and doesn't count.
- *Island*: always.
- *Nothing*: never.
- Decided whenever the front activity changes and when the setting changes; the window's affinity is set only when the answer changes.
- A rebuilt pill (a new look, another display) applies it again.

### 5.3 The setting

- `captures` in `settings.json`: `"private"` (default) | `"island"` | `"everything"`. Unknown or missing: `"private"`.
- Settings → General → **Screen sharing** → *Hide from screen shares*: **Private cards** / **Island** / **Nothing**; detail: "Clipboard, agents, notifications and the shelf" / "Only you see the island" / "Others see the island as you do"; footer: "Also screenshots and recordings. You still see everything."

## 6. Design

- **Protocol:** `Activity::private: bool`.
- **Hub:** `ModuleContext { private }`, `declare_private()`, stamped in `publish`. Called by `agent`, `clipboard`, `notifications`, `shelf`.
- **Island:** `capture.rs` (the setting's values, `excluded(front_private)`, pure and tested; `set_excluded(hwnd, bool)`), `Settings::captures`, `Pill::update_share()` after each render and on a settings change.
- **Settings:** `CAPTURES`, `Files::captures()`, the row and its test.

## 7. Milestones

- **M1 (this change):** all of the above.
- **Later, if asked:** reminders and calendar as private; a per-module choice.

## 8. Tests

- Unit: the setting's values and default; `excluded` for each mode; Settings writes each value.
- Checked on this machine (Windows 11 26200, sandboxed hub): the island's `GetWindowDisplayAffinity` is `0` at rest, `0x11` (`WDA_EXCLUDEFROMCAPTURE`) while the copy toast is in front, `0` again after; switching `captures` to `island` sets `0x11` at once, `everything` keeps `0` through a copy toast.
- By hand, still to do: a Teams or Zoom share and an OBS recording while the clipboard panel is open (the island absent from the far side), and a Snipping Tool capture.
