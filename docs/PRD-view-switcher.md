# PRD: View switcher (tabs under an open card)

Status: draft for review · Owner: Rohit · Oct 30, 2026

**Ask (Rohit):** a Mac notch app shows a small pill of icons (home, tray, music, grid) under the open notch to switch views; plan the same for ReUI. "We need to give better UX with UI with our standards, we can't just do whatever we want": so this is not a copy of the reference, it's what fits ReUI.

**References:**
- **Mac notch apps.** NotchNook has two tabs, Nook (media, widgets) and Tray (a file shelf); boring.notch has Home and Shelf; Alcove and Atoll add a few more. They're all small glyph tabs drawn by the open notch, usually in its top row, and the notch is an app with pages: you open it and then choose a page.
- **What ReUI is instead.** The island is a priority surface. Activities come forward by themselves (decision > alert > ongoing > passive), moments (a permission, a finished timer) expand themselves, and the user opens the rest. Some of those are **places** you go to on purpose: clipboard history, the timer launcher, System stats, Devices, the Now Playing card. Tabs fit places; they don't fit moments.
- **iOS tab bars** (HIG): a tab bar switches between top-level sections, never performs actions, keeps the same tabs in the same order, and shows which one you're in. Badges are for something that needs you.

**Inputs (what ReUI already has):**
- Each place opens its own way: Clipboard with Win+Alt+V or the tray (`clipboard` / `open`), Timers with Win+Ctrl+T or the tray (`timers` / `launcher`), System stats with Win+Alt+S, the tray or `reui-bridge stats` (`stats` / `open`), Devices from the tray (`devices` / `overview`), Now Playing by clicking the pill while music is in front.
- An opened place is a **summoned** card (`pill/summon.rs`, `HubMsg::Focus`): pinned in front over anything less than a decision, open until you move on, and given the keyboard without taking focus when it has a field or a list (`clipboard_keys.rs`).
- The minimal bubble is already a second, detached shape in the island's material (`pill/setup.rs`); its geometry is in `reui-design/src/layout/compact.rs`.
- The card's own pager ("‹ 1/2 ›") steps between alternatives inside one place (players, timers, moments in line).

## 1. Problem

Once a place is open, the only way to another is to close it and remember a hotkey or open the tray. Nothing on the island says what else ReUI can show. People who never read the guide never find System stats or the Devices overview, and people who do still go out to the tray and back.

## 2. Goals

1. **One click between places.** From any open place, every other place is one click away, and the card morphs in place rather than closing and reopening.
2. **Discoverable.** The places ReUI has (and only those that are on) are visible whenever you're in one.
3. **Still ReUI.** The card stays exactly as designed (eyebrow → hero → detail, content first, no chrome inside it). Never steals focus; click-through everywhere outside the shapes; nothing runs at rest.
4. **Modules unchanged.** The switcher opens a place through the path its module already has (the tray's command); modules describe what they are, not how a tab looks.

## 3. Non-goals (v1)

- A **Home** or overview page (see §5.1).
- Tabs for **moments** (permissions, questions, finished timers, device alerts). They come to you; they aren't somewhere you go.
- Showing the switcher on the compact pill, on hover of the compact pill, or replacing the pager.
- Per-user order or hiding single tabs (v1 is on/off), labels under the glyphs, counts or badges on tabs (no filler; see M2 for the one signal worth adding).
- A sessions view for coding agents (needs its own PRD; M3).

## 4. Scenarios

| # | Scenario | v1 |
|---|---|---|
| S1 | Clipboard to a timer | Win+Alt+V opens the panel; under it a small pill of glyphs, Clipboard filled. Click the timer glyph: the panel morphs into the timer launcher, whose field now has the keyboard. Type `10m`, Enter |
| S2 | Music to stats and back | Music is in front; click the pill. Under the Now Playing card, the switcher. Click System stats, then the music glyph: back to the card |
| S3 | Keyboard only | In the clipboard panel (it has the keyboard), Ctrl+Tab: the timer launcher. Ctrl+Shift+Tab: back |
| S4 | Wheel | Over the switcher, a wheel notch steps to the next place, as the wheel over the pager steps players |
| S5 | A permission arrives meanwhile | The place stays (you asked for it, as with any summoned card); the request waits and comes forward when you leave. M2 adds a sign of it on the switcher |
| S6 | Nothing playing | No music glyph. The others never move: Now Playing is the last tab |
| S7 | Only one place is on | No switcher: one tab switches nothing |
| S8 | Agent asks for permission | Its card expands by itself, with no switcher: a moment, not a place |

## 5. Experience

### 5.1 Which views

A place is a view the user opens on purpose, that has something of its own to show at any time, and whose module already has an open path.

| Tab | Module | Available | Opens with (existing path) | Glyph |
|---|---|---|---|---|
| Clipboard | `clipboard` | While the module runs | `open` | Paste `E77F` |
| Timers | `timers` | While the module runs | `launcher` (a running timer's card counts as Timers too) | Timer `E916` |
| Devices | `devices` | While the module runs | `overview` | Devices `E772` |
| System stats | `stats` | While the module runs | `open` | Diagnostic `E9D9` |
| Now Playing | `media` | While a player has a card | `show` (new: brings its card forward; the card itself is unchanged) | Music `E8D6` |

**Rejected:**
- **Home / overview.** The resting island already is the overview: what's in front in the pill, the next thing in the bubble, the wheel through the rest. A Home page would either repeat that or fill itself with filler ("Nothing playing", tiles that open the other tabs). ReUI shows content or nothing.
- **Coding agents.** They publish only when they need you, and there is no sessions view to go to yet. When one exists (M3) it joins with one row in the hub's list.
- **Privacy indicators, volume and brightness, updates, onboarding.** Moments and confirmations, not places.

**Declared by the hub, not hard-coded in the island.** The hub sends the list (`HubMsg::Destinations`) built from the modules that are running, so a feature turned off has no tab, and Now Playing's tab exists only while it has a card. The list itself is one table in `reui-hub/src/modules/destinations.rs` (module, glyph, name, open command, availability): the order is a product decision across modules, like `main.rs`'s module list, and keeping it in one place means no module changes to join. The island draws whatever the list says.

**Order is fixed.** The always-available places first in the table's order, the one that comes and goes (Now Playing) last, so no tab ever moves under the pointer or changes its Ctrl+Tab position while you use it.

### 5.2 When it shows

All of these, and only then:

- The island is **expanded** and shown (never idle, compact, peek; never while hidden for fullscreen or paused).
- The open card belongs to a module in the list (the **selected** tab is that module's: the clipboard panel is Clipboard, a running timer or the launcher is Timers).
- The card is a **place**, not a moment: priority Passive or Ongoing, or summoned (a hotkey, the tray, a tab). A decision, an alert or a brief card shows without it, so a permission request can't be mistaken for one tab among others and a stray click can't navigate away from it.
- The list has **two or more** entries, and Settings → General → View switcher is on (default).

It shows **with the card, not on hover**: a place opened by its hotkey usually has the pointer elsewhere, and discovery is the point. It goes the moment the card starts closing.

### 5.3 Form

```
              ╭──────────────────────────────────────────╮
              │  CLIPBOARD                               │   the open card, unchanged
              │  …                                       │
              ╰──────────────────────────────────────────╯
                          gap 8
                     ╭─(●)─ ◷ ─ ▣ ─ ♫ ─╮                 island height, inset 4
                     ╰─────────────────╯                 selected tab: soft fill
```

- **Under the card, detached, centred**, `island.switcher.gap` (8) below its bottom edge, riding the card's height spring as places of different heights morph into each other. Not inside the card, where the reference puts it: *content first, chrome never*. A tab row inside would be chrome in every card, a second header next to the pager, and room every layout, height cap and Settings preview would have to make. Detached, each card stays exactly as designed and the tabs read as navigation, not content.
- **The island's family.** The same material as the island (black, or white with the hairline in Light), the island's height (`island.height`, like the minimal bubble), a capsule at full roundness (`look.rounded`). Tabs are circles one `island.switcher.inset` (4) inside it: `concentric(rounded(h), inset)`, so at low roundness the pill and its tabs square off together.
- **Glyphs only.** No labels (the open card's eyebrow already says where you are, and the user's own reference has none), no counts. Glyphs at `icon.compact` (16), in `label.secondary`; the selected tab sits on a soft fill (`fill.secondary`) with its glyph in `label.primary`, as the clipboard's filter tabs do; the tab under the pointer gets `fill.quaternary`. **No accent:** the accent belongs to the activity in the card; navigation stays neutral.
- **Notch style:** the same floating capsule under the notch's card. It isn't on the screen's edge, so no ears and no flat top; the notch's in-notch second activity is a compact-only presentation and is hidden while a card is open anyway.
- **The pager stays.** Pager = alternatives within one place (in the card, top right). Switcher = between places (under the card). Two levels, two places on screen, and the wheel works the same on both.
- **The minimal bubble** is hidden while a card is open (unchanged), so the two never show together.

### 5.4 Motion

- Appears after the card starts opening: opacity and a small drop (`island.switcher.rise`, 4) on a settling spring (`motion.spring.expand`, critically damped: no overshoot). Leaves on `motion.spring.reduced` (short, critically damped) as soon as the card starts collapsing, so it never hangs under a shrinking card.
- Switching places is the existing summoned-card morph: width and height springs, the new content cross-fading in. The switcher follows the height spring through an expression in DWM, so it costs the process nothing.
- With Windows' animation effects off: no drop, `motion.spring.reduced` both ways.

### 5.5 Input

- **Click a tab**: the island sends that module's command (`ClientMsg::Command`), exactly what the tray sends; the module publishes its card and asks for focus; the island opens it as a summoned card. Clicking the selected tab does nothing (it doesn't close the place: Esc, the hotkey and clicking away do that).
- **The rest of the screen stays click-through.** Only the switcher's own shape takes clicks (and the 8 DIP gap above it counts as "on the island" for hover, so moving the pointer from the card to the tabs doesn't start the collapse). A press on the switcher isn't "outside" a summoned card.
- **Wheel over the switcher** steps to the next or previous place, a whole notch at a time.
- **Keyboard: Ctrl+Tab / Ctrl+Shift+Tab**, only while the open card already has the keyboard (the clipboard panel, the timer launcher, a card you clicked that asks something) and the switcher is showing; otherwise the keys go to the app in front as usual. The place switched to gets the keyboard if it can use it, so a typed `10m` lands in the timer field. **Not Ctrl+1–9:** those belong to the app in front (a browser's tabs) and Alt+1–9 already picks the clipboard's nth card. **No keyboard for cards that don't use it:** System stats, Devices and Now Playing don't capture keys just to offer Ctrl+Tab; reading every keystroke for that would cost more trust than it buys.
- **Never steals focus**: no activation on click (the window is `WS_EX_NOACTIVATE`, `MA_NOACTIVATE`), and keys come from the hook the panel already uses.

### 5.6 What it never does

- Show on the compact pill, take focus, move a tab while you use it, open a moment, wobble, or run anything while nothing changes.

## 6. Protocol (v19)

- `Destination { module, glyph, name, open }`: a place a surface can offer. `glyph` is a Segoe Fluent Icons code point (as `Element::Glyph` carries), `name` its accessible name ("Clipboard"), `open` the command to send to `module` to open it.
- `HubMsg::Destinations(Vec<Destination>)`: the places, in order. Sent to an island right after `Welcome` and again whenever the list changes (a module starts or stops, Now Playing's card comes or goes), never per snapshot. Added at the end of `HubMsg`, so nothing else in the protocol moves.
- Modules gain nothing new, except Now Playing's `show` command (bring its card forward).

## 7. Edge cases

| Case | Behaviour |
|---|---|
| A decision arrives while you're in a place | The place keeps the island (summoned rule, `order.rs`); the decision comes forward when you leave. M2: a sign of it on the switcher |
| A decision arrives between the click and the place opening | The place still opens (you asked for it) and the decision waits, as above |
| The module doesn't open (Now Playing's player just went) | Nothing happens; the open card stays and stays selected |
| A module is off, or stops | It isn't in the list (built from running modules), so no tab |
| Fullscreen app, pause, a hidden visibility mode | The switcher shows only with an open card, so it hides with the island; a place summoned in a hidden mode reveals the island as today |
| One place only | No switcher |
| A brief (copied) while a place is open | It goes to the bubble, which is hidden while a card is open (unchanged) |
| Drag from another app over the island | Nothing, as for the pill (`drag_guard`) |
| Display or scale changes, a new look | The pill is rebuilt (with the switcher) like today |

## 8. Performance budget

- At rest: nothing. No timers, no redraws, no messages. The switcher's surface is drawn when the list, the selected tab or the tab under the pointer changes; its motion is a spring in the pill's property set that settles and stops.
- The window grows by the switcher's room (`gap` + height, about 45 DIPs) below the tallest card; it's transparent and click-through there, so nothing underneath is blocked and DWM composes nothing extra.
- Hub: the list is rebuilt when activities change (a five-row table) and sent only when it differs.

## 9. Settings

`%APPDATA%\ReUI\settings.json` `view_switcher` (bool, default true), Settings → General → Island: "View switcher". The island watches the file, as for its other settings.

## 10. Accessibility

- Contrast: unselected glyphs `label.secondary` on the island's material (about 6:1 on black), selected `label.primary` on `fill.secondary`. Light uses the light palette.
- Targets: each tab takes clicks over the switcher's full height and its share of the gaps (≥ 32 × 36 DIPs at the default size), more than the glyph's circle.
- Reduced motion: §5.4.
- Screen readers: the island has no UI Automation tree yet (open item for the whole island). `Destination::name` is carried so the switcher can be exposed as a tab list when it gets one (M4).

## 11. Milestones

| # | What | Done when | Status |
|---|---|---|---|
| M0 | Protocol (`Destination`, `HubMsg::Destinations`, v19); the hub's list from running modules and Now Playing's card (pure, tested); Now Playing's `show`; tokens and `reui_design::layout::switcher` (pure, tested) | Tests | |
| M1 | The island draws it under a place's card, with its motion; click, wheel and Ctrl+Tab open places through their modules; hover and click-through rules; Settings → General toggle | A sandbox `--demo` hub: Clipboard → Timers → System stats by tab, screenshots in dark, light and Notch | |
| M2 | A waiting decision or alert shows on the switcher while you're in a place (its accent dot at the switcher's end; click to go to it); Settings' General preview shows the switcher | Permission arriving during a summoned place is visible within one frame | Planned |
| M3 | A coding agents sessions view (own PRD) as a destination | | Planned |
| M4 | UI Automation for the island; the switcher as a tab list | Narrator reads "Clipboard, tab, selected, 1 of 4" | Planned |

## 12. Open questions

- Should a waiting decision (M2) be a dot on the switcher, or should the switcher yield to it entirely (the decision opens, the place closes)? This draft: the dot, since you asked for the place.
- Should the timer tab open the running timer's card instead of the launcher when one runs? v1 keeps the module's existing path (`launcher`); the running timer is one wheel or bubble away.
