# PRD: The island on all displays

Status: proposed · Owner: Rohit · Extends Settings → General → Display ([PRD-behaviour.md](PRD-behaviour.md)) · Nov 1, 2026

**References:**
- **macOS notch apps** (NotchNook, boring.notch, Atoll) offer "show on all displays" or "show on the display with the menu bar" / "the active display". Users with an external monitor and a closed or open laptop lid ask for the island on the screen they look at; users with two equal monitors ask for it on both (boring.notch's multi-display setting).
- **The Windows taskbar** (Settings → Personalization → Taskbar → "Show my taskbar on all displays") puts one taskbar per display. The clock and the system tray stay on the primary display only, and the other taskbars show only app buttons. That's the model to borrow: everything visible everywhere, one owner for the things that must be single.
- **ReUI today:**
  - `monitor` in `settings.json`: `primary`, `cursor` (follow the pointer's display, in every state since Nov 1, 2026), or a display's device name.
  - One `reui-island.exe`, launched and supervised by the hub (`island.rs`), with one window, one `Pill` (`thread_local PILL`), one hub link, one tray icon, and one low-level mouse hook. A keyboard hook exists only while a card has the keys (`clipboard_keys.rs`).
  - Measured on Nov 1, 2026, at idle: the island takes 59 MB private memory, the hub 10 MB. Most of the island's share is the D3D/D2D device, DirectWrite fonts and composition surfaces.
  - Fullscreen hiding, app rules, the reveal edge and screen-share privacy all work on the island's window and display.

## 1. Problem

With two or more displays, the island is on one of them. "Where the pointer is" helps when you move around, but a glance at the other screen (music, a timer, a permission waiting for you) finds nothing there. People who keep the island always visible ask for it on every display, as the taskbar can be.

## 2. Goals

1. **One more choice** in Settings → General → Display: **All displays**, beside Main display, Where the pointer is, and each display by name.
2. **The same island on every display:** the same activities in the same order, each display's island showing them at its own scale.
3. **One owner for what must be single:**
   - **Keyboard and summoned cards** open on the pointer's display, and only one card has the keyboard at a time.
   - **The tray icon** exists once.
   - **Modules** hear "open" and "closed" once, however many displays show the card.
4. **Per display where it means per display:** a fullscreen game on one display hides only that display's island. Hover, peek, the reveal edge and drops work on each island by itself.
5. **Cheap:** an extra display costs a window and its surfaces, not another process. Target: under 15 MB more private memory per extra display, and no CPU at rest.
6. **Plug and unplug:** a display that arrives gets an island, and one that leaves takes its island with it. No restart.

## 3. Non-goals

- Different content per display, or choosing which activities show where.
- A different look per display. The Appearance settings apply to all of them.
- Spanning one card across displays.
- Mirroring a card's scroll or hover to the other displays. Each island is used by itself.

## 4. Scenarios

| # | Scenario | Result |
|---|---|---|
| S1 | Two displays, All displays, music playing | the music pill on both, bars moving on the pointer's display (§5.6) |
| S2 | A Claude permission arrives | the card opens by itself on both displays. Answer on either: both close (the hub retracts the moment) |
| S3 | Win+Alt+V with the pointer on display 2 | clipboard history opens on display 2 only, with the keyboard; display 1 keeps the pill |
| S4 | Then the pointer moves to display 1 and Win+Alt+V again | display 2's panel closes, display 1's opens: one panel with the keyboard at a time |
| S5 | Win+Alt+N (open what's waiting) | acts on the pointer's display's island (PRD keyboard island) |
| S6 | A game fullscreen on display 1 | display 1's island hides; display 2's keeps showing |
| S7 | When needed, nothing happening, pointer pushed against display 2's top edge | display 2's island reveals; display 1's stays hidden |
| S8 | A file dragged toward display 1's island | display 1 becomes the drop target; drops work on any display's island |
| S9 | Laptop lid closed, external display only | one island, on the external display |
| S10 | A third display plugged in | an island appears on it within a second |
| S11 | Right-click the tray icon → Quit | every island closes, and ReUI quits |
| S12 | Win+Alt+I (hide) or Pause | hides every island; decisions still show on all |
| S13 | Sharing display 2 in Teams with a clipboard card open on display 1 | display 1's island is excluded from capture as usual (per window), and display 2 shows nothing private |
| S14 | Displays at 100 % and 150 % | each island at its own scale: sharp text, right size |

## 5. Rules

### 5.1 The setting

- `monitor: "all"` in `settings.json`. Settings → General → Display adds **All displays** to its list (after Where the pointer is). Unknown values still mean the primary display.
- Switching to or from All displays applies at once: islands are added or removed, with no restart.

### 5.2 One process, one window per display

- The island process keeps one window and one `Pill` per display. It doesn't launch one process per display, for these reasons:
  - **Cost:** a second process would duplicate the 59 MB, the device, the fonts and the images. Windows in one process share them.
  - **Coordination:** the keyboard owner, the tray and open/closed are decided in one place, with no new hub messages or races between processes.
  - **One hub link:** one connection, and each snapshot decoded once.
- The hub keeps launching and supervising one island process (`island.rs` unchanged). A crash takes all the islands down and the hub restarts them together, as today.

### 5.3 What's shared and what's per display

| Shared (process) | Per display (window) |
|---|---|
| hub link, snapshot, image store (`images.rs`), places | the `Pill`: state (resting, peek, open), springs, surfaces, hits |
| D3D/D2D device and DirectWrite fonts (`gpu`) | scale (the display's DPI) and the window rectangle |
| settings (`settings.json`), the look | fullscreen hiding and app rules (§5.5), revealed, hovered |
| tray icon | screen-share exclusion (each window's affinity) |
| the low-level mouse hook (dispatches to the island under or nearest the pointer) | drops (each window is its own drop target) |
| the keyboard hook and its owner (§5.4) | |

### 5.4 Single owners

- **The keyboard:** at most one island has it.
  - A summoned card (a module's hotkey, the hub's `Focus`) and the open shortcut (`OpenIsland`) go to the island on the pointer's display.
  - A click gives the keys to the clicked island.
  - When one island takes the keyboard, the island that had it lets go: a summoned card closes, an engaged decision is released.
- **Open and closed to modules:** a module hears `expanded` when the first island opens its card, and `collapsed` when the last one closes it. Example: System stats samples while its card is open anywhere. The process combines the islands' open cards before telling the hub, so the hub and modules don't change.
- **Pinned and summoned:** these are per island. Bringing an activity forward on display 1 (the bubble, the wheel) doesn't change display 2.
- **The tray icon:** one, owned by the process. Its menu works on all islands (Pause, Hide, Quit).

### 5.5 Per display

- **Fullscreen:** each island checks whether a fullscreen app covers *its* display (`fullscreen.rs` already takes the display).
- **App rules:** "Hide the island" and "Only decisions" apply to the island on the display that holds the app in front. Other islands are unaffected.
- **Visibility and reveal:** each island hides and reveals by itself, from its own top edge.
- **Desktop in front:** "Always show on the desktop" applies to all islands (the desktop spans every display).

### 5.6 Costs at rest

- **No timers or polling per island beyond today's.** Animations are compositor-driven per window and stop when settled.
- **Audio-reactive bars:** only one island captures audio, the one on the pointer's display. The others draw their bars at rest, as with Windows' "animation effects" off. One capture instead of N.
- **The idle clock** (when on) redraws once a minute on each island.

### 5.7 Displays changing

- On `WM_DISPLAYCHANGE` and `WM_DPICHANGED`, after the existing settle delay, the process lists the displays. It adds a window for each new one, removes the window of each one that's gone, and re-places and rescales the rest. An island moving to another scale is rebuilt as today, keeping its summoned card (`reopen_after_move`).
- A display that's turned off but still attached keeps its island (Windows still lists it).

### 5.8 Other modes are unchanged

- **Main display, a named display, Where the pointer is:** one window, as today. These are the same code with a list of one.

## 6. Design

- **Island:**
  - `main.rs`: the single `PILL` becomes `Islands`, the pills keyed by window, with the shared link, settings, tray and GPU resources. `wndproc` looks up the pill for its `hwnd`. `with_pill` becomes `with_island(hwnd, f)` plus `each_island(f)`.
  - **New file `islands.rs`, pure and tested:** which displays get an island for a `MonitorChoice`, which island owns the keyboard, combining open cards into one expanded/collapsed per activity, and where the mouse hook sends a point.
  - **`pill/link.rs`:** `drain_hub` drains the link once and applies the snapshot, focus and places to every island (focus only to the owner, §5.4).
  - **`place.rs`:** `MonitorChoice::All`; `placements(choice, look)` returns one placement per display.
  - **`clipboard_keys.rs`:** the hook's window becomes the owner's. Taking ownership releases the previous owner.
  - **`tray.rs`:** added once, by the first window.
  - **The visualizer:** live only on the pointer's island.
- **Hub:** no change, beyond the island module reloading the setting as today.
- **Settings:** `MONITOR_ALL = "all"`, the label "All displays", and a test.
- **Protocol:** no change.

## 7. Milestones

- **M1, one process, many windows:** `Islands` replaces the single pill, and every existing mode works with a list of one. No behaviour change; the tests and the sandboxed island runs are the same as today.
- **M2, All displays:** the setting, a window per display, shared snapshot, focus to the pointer's island, one keyboard owner, combined open and closed, per-display fullscreen and app rules, one tray.
- **M3, displays changing and costs:** plug and unplug, mixed scaling, the visualizer on one island, and measured memory and CPU (§8).

## 8. Tests

- **Unit (`islands.rs`):** displays for each choice; the keyboard owner (summon, click, open shortcut, hand-over); combining open cards (first open is `expanded`, last close is `collapsed`); the mouse hook's target island.
- **On two real displays** (this PC: 1920×1080 primary, 1280×768 secondary; sandboxed hub, synthetic pointer moves as in the "Where the pointer is" fix):
  - S1–S8, S11 and S12, each checked by the island windows' rectangles and screenshots of both displays.
  - Plug and unplug (S10) by disabling and enabling the second display.
  - Mixed scaling (S14), by setting the second display to 150 %.
- **Cost:** idle private memory with one display, then two, in All displays. Target: under 15 MB more per extra display. CPU at rest: 0 % over a minute, with music paused and with it playing.
- **Regression:** the existing island tests, and one display in every mode behaves exactly as before.

## 9. Open questions

1. **Decisions on every display, or on the pointer's only?** Proposed: every display, since you might be looking at either; answering on one closes both.
2. **Should "Where the pointer is" become "All displays, opening on the pointer's"?** No: some people want one island, and the setting keeps both.
3. **Per-display app rules:** is the app's display the right scope, or should a rule hide every island? Proposed: the app's display.
