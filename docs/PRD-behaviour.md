# PRD: A considerate island (when it appears, opens and interrupts)

Status: planned · Owner: Rohit · Oct 30, 2026

**Ask (Rohit):** make ReUI the most *considerate* island: give people control over how and when it appears and interrupts.

**Inputs:**
- **What Windows users complain about** (r/Windows11, r/windowsapps, Windhawk issues #4738, #4424, #4194) is behaviour, not looks: the top-edge hover steals clicks meant for browser tabs; the island opens when the pointer merely brushes it; media keeps it on screen; idle widgets can't be turned off; there's no per-app control.
- **What ReUI already does** (`DESIGN.md` rules 5 and 7, "never steal focus", hover intent): it never activates, is click-through outside the pill, waits for the pointer to rest before taking clicks in the band above the pill, has three visibility modes, hides for fullscreen and has a hide shortcut. §1 is the exact audit.

## 1. Audit: what it did before (Oct 30, 2026)

| Behaviour | Exactly |
|---|---|
| **Hover → peek** | The pointer entering the pill's area (its peek size, from the screen's top edge down: the `top_gap` band above the pill counts), the bubble or the switcher starts `motion.delay.hover-intent` (250 ms). If it is still there, the pill becomes clickable and a compact or idle pill **peeks** (+6 DIP). Leaving a peek folds it at once |
| **When it takes clicks** | On the **drawn shape** (pill, bubble, switcher): at once, on the first pointer move over it ("a quick click there is clearly aimed at it"). In the **band above** the pill (11 DIP by default, where a maximised browser's tabs are): only after the 250 ms rest. Everywhere else: never (`WS_EX_TRANSPARENT`) |
| **Click** | Compact → opens the card (or sends `open` when there is none). Idle → the row of places. Hover never opens anything |
| **Collapse** | Leaving an open card: `delay.collapse` (450 ms); a summoned card (hotkey): `delay.panel-idle` (60 s) untouched, or a press elsewhere, or another app coming forward. A decision opens itself and folds back after `decision-hold` (8 s) unless hovered |
| **Visibility modes** | *Always*: the resting pill is always there. *When active*: while any activity exists, so music playing keeps it on screen. *When needed*: decisions, alerts and briefs (a volume change, a copy) only |
| **Reveal (hidden modes)** | Pointer at the **top 2 px** of the island's window (its top is the work area's top), anywhere across the **window's width**: the widest card plus 24 DIP each side, **608 DIP** at 100 % (912 px at 150 %), about 5× the resting pill. It shows at once, and takes clicks at once on the shape like a visible pill (in the Notch style, where the shape touches the edge, the first click there was the island's). It hides `reveal-hold` (3 s) after the pointer leaves |
| **Fullscreen** | `hide_in_fullscreen` (on): hidden while an exclusive Direct3D app or a presentation runs (anywhere), the shell says "busy" for an app on the island's display, or the foreground window covers that display. Rechecked on foreground changes, the foreground app's window moves, and 300 ms later. Hides everything, decisions too |
| **Desktop** | `show_on_desktop` (on): with the wallpaper in front, a hidden mode shows as if *Always*; the taskbar, Start and Alt+Tab keep the answer |
| **Win+Alt+I** | Hidden over any mode until pressed again (`hidden` in settings.json, survives restarts); only decisions and a card summoned with its shortcut show; the top edge doesn't reveal it |
| **Pause** | An hour: like hidden, plus clipboard recording stops; decisions show |
| **Per-app control** | None. Per-moment control: each module's page in Settings (`osd.json` volume, `devices.json` connect_cards, ...); the clipboard's copy toast had no setting |

## 2. Goals

1. **Opening is the user's choice.** Open on click (today) or on hover, and how long the pointer must rest.
2. **The top edge is the browser's.** The reveal zone is explicit (Off / Pill / Wide), and a hidden island never takes a click it wasn't shown for: clicks always wait for the rest.
3. **Per-app rules.** While a chosen app is in front: hide the island, or show only decisions.
4. **Quiet a kind of moment from the moment.** Right-click a brief or an alert: "Don't show these" or "Snooze for 1 hour". Never for decisions.
5. **Published cost.** CPU and memory at rest, with music, with the stats card open, in the README.

## 3. Non-goals

- Per-app rules by window title or URL; rules that change the look; schedules ("quiet hours", Windows' Do Not Disturb already does that).
- Quieting decisions. A permission request is the one thing only the user can answer; pause, hide, every app rule and every snooze let it through (except "Hide the island" in an app and fullscreen, which hide everything, as today).

## 4. Experience

### 4.1 Opening style and hover delay (Settings → General → Island)

| Setting | Values | Default |
|---|---|---|
| **Open the island** (`open_on`) | *On click*: a rest peeks, a click opens. *On hover*: a rest opens the card (a pill without one peeks; the idle pill peeks with the date and time, its row of places still opens on click) | On click |
| **Hover delay** (`hover_delay`, ms) | Quick 150 · Normal 250 · Relaxed 500 (the file takes 50–2000) | Normal = `motion.delay.hover-intent` |

The delay is the one rest everywhere: the peek, a hover-open, and when the band above the pill (and a just-shown pill, §4.2) starts taking clicks. A card opened by hover folds `delay.collapse` after the pointer leaves, as one opened by a click.

### 4.2 Reveal zone and clicks

| `reveal_zone` | Pushing the pointer to the top edge in *When active* / *When needed* |
|---|---|
| **Off** | Never reveals; the island shows only when its mode says |
| **Pill** | Only across the pill's own width (the shape it would show) |
| **Wide** (default: today) | Across the island's window: the widest card's width |

**A just-shown pill doesn't take quick clicks.** The drawn shape takes clicks at once only once it has been on screen for the hover delay before the pointer got there. A pill that appears under the pointer (revealed from the edge, or shown by a brief while the pointer is at the top) is click-through until the pointer rests on it: the reveal only reveals. Pure rule: `intent::takes_clicks_at_once`.

### 4.3 Per-app rules (Settings → General → App rules)

A list of executables (`app_rules` in settings.json: `{ "game.exe": "hide", "obs64.exe": "decisions" }`), added from the running apps' menu or typed, as the clipboard's ignored apps are. Each has one rule:

| Rule | While that app is in front |
|---|---|
| **Hide the island** (`hide`) | Hidden, as for fullscreen |
| **Only decisions** (`decisions`) | Only permission requests and questions show, alone in the pill |

Driven by the foreground hook the island already has (no polling): each change reads the front process's executable name once. Shell surfaces (taskbar, Start, Alt+Tab) keep the last answer, so passing through them doesn't flash the island; the desktop clears it. Defaults: empty. Pure rule: `app_rules::rule_for`.

### 4.4 Quiet a kind of moment (right-click)

Right-click the pill (or the bubble) while a brief or an alert from a module that offers it is in front:

- **Don't show these**: turns off that module's own setting for this kind of moment (the volume pop-up: `osd.json` `volume`; a device connecting: `devices.json` `connect_cards`; the copy toast: `clipboard.json` `copy_toast`, new). The island writes the key (keeping every other key, never over an invalid file) and tells the module `reload`, exactly as Settings does, and the moment on screen goes at once. Settings shows it off on that module's page, where it turns back on.
- **Snooze for 1 hour**: that module's briefs and alerts stay out of the island for an hour (`snoozed` in settings.json: module → Unix ms). What the user is in (music, a running timer, a place they opened) isn't an interruption and stays. Settings → General → **Snoozed** lists each with **Undo**.

The rest of the island's menu follows, and the island hands the foreground back to the app that had it when the menu closes (before, the island's menu kept it). Never for a decision (a loud finished timer is one: it rings until stopped). The module declares the setting once (`ModuleContext::declare_quiet`, a function of the moment's key and priority); the hub attaches it to its briefs and alerts (`Activity::quiet`, protocol v22): `Quiet { setting: Option<QuietSetting { file, key }> }`, `None` meaning snooze only; a module that declares nothing gets no entries.

| Module | Moment | "Don't show these" turns off |
|---|---|---|
| osd | Volume / brightness levels; Caps/Num Lock; battery saver; mic mute | `volume` / `brightness`; `lock_keys`; `battery_saver`; `mic_mute` |
| devices | A device connecting; low battery; charging; a drive; a phone; a slow charger | `connect_cards`; `low_battery`; `charging`; `drives`; `phones`; `slow_charger` |
| clipboard | The copy toast | `copy_toast` (new, default on) |
| timers | A timer that finished quietly | Snooze only |
| claude, codex, opencode | A finished turn | `coding.json` `show_finished` (only the module whose moment it was reloads; the other two pick it up at their next reload) |

## 5. Settings (all in `settings.json` unless said; unknown keys kept)

`open_on` ("click" | "hover"), `hover_delay` (ms), `reveal_zone` ("off" | "pill" | "wide"), `app_rules` (object: exe → "hide" | "decisions"), `snoozed` (object: module → Unix ms; past entries ignored and dropped on the next write), `clipboard.json` `copy_toast` (bool, default true).

## 6. Performance

Nothing new runs at rest: the foreground hook existed; one snooze timer runs only while something is snoozed, set for the earliest end. README → Performance publishes the numbers, measured with `tools/perf-sample.ps1` on a sandboxed build.

## 7. Milestones

| # | What | Status |
|---|---|---|
| M1 | Opening style and hover delay; Settings rows | Planned |
| M2 | Reveal zone; a just-shown pill waits for the rest | Planned |
| M3 | Per-app rules; Settings → App rules | Planned |
| M4 | Quiet from the moment (protocol v22, osd / devices / clipboard / timers); Settings → Snoozed with Undo | Planned |
| M5 | Measured cost in the README; `tools/perf-sample.ps1` | Planned |
