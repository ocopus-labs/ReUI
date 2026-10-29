# ReUI Design System

The look and behaviour of every ReUI surface. The island is the first surface; the dock, taskbar and desktop of the future shell will use the same system.

**Where things live**

| What | Where |
| --- | --- |
| Tokens (source of truth) | `design/tokens.json`, W3C Design Tokens format |
| Tokens in code | `reui-design` crate, generated at build time: `tokens::color::label::PRIMARY` etc. |
| Layout (where every piece goes) | `crates/reui-design/src/layout/`: pure functions over the `Look` and measured text widths, shared by the island and Settings' previews |
| Components (drawing) | `crates/reui-island/src/render/` (one file per presentation and row) |
| Presentations and motion | `crates/reui-island/src/pill/` |
| Semantic view model (what modules may say) | `crates/reui-protocol` |
| Logo | `design/logo/reui.ico` (+ a PNG per size), drawn by `tools/logo.ps1` on each size's own pixel grid: a light screen with the black island at its top and its bubble in `color.system.blue`. Every program links it in (`reui_design::icon::embed()` in its `build.rs`); the tray, the Settings window and Apps & features load it; Settings' sidebar draws the same mark |
| References | `design/reference/` (Figma kit export, moodboard in `inspiration/`) |
| Figma | [Dynamic Island – Live Activities (Community)](https://www.figma.com/design/Oavznl8mcr41wrSfXdpCNs/Dynamic-Island---Live-Activities--Community-) by Jasper Cheyney; island geometry comes from here |
| Figma (system alerts) | [iOS 17 Dynamic Island Components (Community)](https://www.figma.com/design/Q5jcdhxYldk71bDnC5VzIE/iOS17-Dynamic-Island-Components--Community-); its alerts with buttons shape the agent cards (see `docs/PRD-agent-moments.md` §5.8) |

**The rule that keeps redesigns cheap:** modules describe *meaning* (priority, regions, element kinds, tones, button roles, an accent colour). Only the island decides *appearance*, and only from tokens. A redesign changes `tokens.json` and `render.rs`, never a module or the protocol. No hard-coded sizes, colours or fonts outside the token file.

## Principles

1. **Black and quiet.** The island is a pure black shape that blends into the top of the screen (the default look; see *Appearance* for Light and the Notch style). Colour comes from one accent per activity, used sparingly: the eyebrow, a ring, the bars, one tinted button.
2. **Three levels of hierarchy.** Every expanded view reads **eyebrow → hero → detail**: a small accent label saying where this comes from ("Timer", "Claude Code", "Now Playing"), one large thing to read (34 pt numerals, a 20 pt title, or a command chip), and a quieter secondary line. Never two heroes.
3. **Concentric corners.** Shapes inside the island follow its curve. In the compact capsule, round content is a circle inset so it sits concentric with the pill's end (24 circle, 6.33 inset). In the expanded island, anything one inset from the edge uses `radius.concentric` (44 − 20 = 24); buttons are capsules (radius ≈ 24) or circles. No small-radius squares or squircles next to a round edge.
4. **Content first, chrome never.** No borders, no separators, no shadows on the island. Hierarchy comes from size, weight and the label colours.
5. **Motion explains state.** The island only moves when its state changes, using springs that can be interrupted. **No compositor loops:** while any composition animation runs, DWM recomposes every display frame (measured 17–30 % of a core on a 199 Hz display, even for stepped keyframes). Things that must keep moving (level bars while music plays) are driven by the island at a fixed rate (`motion.bars.fps`, 30): DWM then only redraws when they change, measured at no cost above idle.
6. **One thing in front.** Priority decides the front activity (decision > alert > ongoing > passive). One more shows as the minimal bubble; the rest wait.
7. **Never steal focus.** The island never activates on click and is click-through everywhere except the pill itself.
8. **Crisp at every scale.** Geometry is pixel-snapped and layers sit on whole pixels; translucent strokes are not used for glow (they read as blur).

## Presentations

Following Apple's Live Activity model, from the Figma kit:

| Presentation | When | Anatomy |
| --- | --- | --- |
| **Idle** | No activities | Black capsule, `island.idle.width` × `island.height`, empty; with `idle_clock` on (Settings → General), the time in `label.secondary`, centred. **A peek** shows the date (secondary) and the time (primary) `joined-gap` apart, centred, the pill as wide as both plus `text-margin` each side (`layout::compact::clock`). **A click** opens it into the row of places: the view switcher's destinations as its tabs (circles one `island.switcher.inset` in, glyphs only, the hovered one on `fill.quaternary`), the pill as wide as the row at the island's height. No labels: the glyphs are the switcher's, which the user learns there, and a label under a 16 glyph at this height would be 8 pt filler. The hover clock never shows while something is happening |
| **Compact** | One activity in front | `leading` · `center-gap` · `trailing`. Both sides are the **same width** (the wider side, margin included). Round content (art, ring) is inset `side-margin` so it sits concentric in the capsule's end; text and bare glyphs keep `text-margin` clear of the curve. Text is capped at `max-side`. Apple reserves 126 in the middle for the camera; Windows has no cutout, so the gap is only 48. **A text label** (copied, Caps Lock on, a device, "Allow?") instead sits `joined-gap` after its mark and the pill is only as wide as the two (never narrower than the resting pill); **a bare glyph** (the privacy indicator's microphone, an agent's check) does the same, and the pill is just the two (a glyph has no length to fill the resting width, so a floor would only move the empty space to the end). Rings, clocks, bars, levels and art stay at the far end, where they read as the activity's state; a level fills the whole pill (see `Level` below) |
| **Peek** | Pointer over compact | Compact grown by `island.compact.peek-grow` |
| **Minimal** | A second activity exists | Detached circle `island.minimal.size`, `gap` to the right of the pill, showing the second activity's `minimal` element |
| **View switcher** | A place's card is open (clipboard, timers, calendar, devices, stats, Now Playing: views you go to, not moments that come to you) and there are two or more places | A capsule of glyph tabs as tall as the island, centred `island.switcher.gap` under the card and riding its height spring. Tabs are circles one `inset` in, concentric with its ends; glyphs `label.secondary`, the open place's tab on `fill.secondary` with a `label.primary` glyph, the hovered one on `fill.quaternary`. No accent, no labels. Notch style: the same floating capsule. Never compact, never with a decision, alert or brief in front. **A decision waiting** behind the open place shows as a bubble `waiting-gap` after the switcher's end (its height and corner) with the decision's accent dot (`component.dot`): not a tab, so the tabs never move and a moment never reads as a place; a click brings the decision forward. PRD: `docs/PRD-view-switcher.md` |
| **Expanded** | Click, or a decision or alert arrives | Width by the view's `size`: `island.expanded.width` (Regular), `roomy-width` (Roomy, agent moments) or `wide-width` (Wide, clipboard). An optional session strip, then the header: `leading` · eyebrow / hero / detail · `trailing` (or inline controls). Then full-width rows. Height fits content between `min-height` 84 and `max-height` 284 (`roomy-max-height` 440 for a roomy card) |

Expanded layout, all from tokens:

```
┌─ inset 20 ───────────────────────────────────────────────┐
│ ╭──────╮  EYEBROW (13 semibold, accent)        (⏸) (✕)   │  header: max(leading, text, trailing)
│ │ 56   │  Hero: 34 numerals / 20 title / chip            │  ← inline glyph controls when the
│ ╰──────╯  detail (15, secondary)                         │    only row is glyph buttons
│ ── section-gap 16 ─────────────────────────────────────  │
│ ▬▬▬▬▬▬▬▬▬▬▬▬▬▬▬▬▬▬───────────────────────────────────── │  progress 5, captions (tertiary)
│ 01:32                                           -02:31   │
│ ── row-gap 12 ─────────────────────────────────────────  │
│            ⏮          ( ⏸ )          ⏭                   │  transport circles 44, gap 28
│   or  (   Deny   )  (  Allow once  )                     │  text capsules 44, gap 10
└──────────────────────────────────────────────────────────┘
leading tile: radius.concentric (24) · trailing art: circle
```

Layout templates fall out of the content, so modules don't pick them:

| Content | Result | Example |
| --- | --- | --- |
| One row of glyph-only buttons | Controls inline beside the hero (Apple timer) | Timer |
| Progress and/or transport | Header, then full-width rows | Music |
| Text buttons | Header, then equal-width capsules | Agent permission |
| A session strip (`context`) | One quiet line above the header; the pager moves onto it | Agent moments |
| A moment's one text button, alone | A capsule beside the hero (`inline` tall, `inline-text-padding` each side) when the hero fits beside it, else full width under the text | A failed turn's Open terminal |
| An `Agenda` row (the calendar) | Under the header, one line per thing: the time in a column as wide as the widest (`fact` type, secondary) so titles line up, the title (primary), the where at the line's end (tertiary, at most `agenda.detail-share` of the line); `agenda.line-gap` between lines. The card's one action (Join) is the header's trailing text button, beside the hero | Calendar |
| `Gauges` with an empty headline | The header is the eyebrow alone (no hero line, no gap under it); the gauges are the hero. A `Plain` glyph `Button` as the trailing element sits bare on the eyebrow's line, `icon.header` in secondary like the pager, with a `header-button.hit` target | System stats |

## Behaviour

| Event | Island does |
| --- | --- |
| First activity arrives | Idle → compact |
| Decision arrives (e.g. agent permission) | Expands itself, then folds back to compact (still pending, accent "Allow?") after `motion.delay.decision-hold` unless you engage |
| Alert arrives (done, failed, "your turn") | Comes to the front in compact; doesn't expand itself |
| Decision answered | Folds back to compact (unless hovered) |
| Pointer rests on the pill for `motion.delay.hover-intent` | Becomes clickable and peeks. Until then it's click-through, so quick moves and clicks reach what's underneath (browser tabs) |
| Click on compact | Expand |
| Click on the idle pill | Opens the row of places; a tab opens its place as a switcher tab does (the module's own open command), Esc (the keyboard hook passes every other key on), a press elsewhere or leaving folds it back. Never takes focus |
| Pointer leaves expanded | Collapse after `motion.delay.collapse` |
| Fullscreen app in front (game, video, browser video, presentation) | Hide entirely (`hide_in_fullscreen`, on by default) |
| Something dragged over it (a tab, a file, a selection) | Nothing: no peek, no clicks, until the button is up and the pointer rests. The one exception is the file shelf's drop target, below |
| A drag from another app comes within `island.shelf.reach` of the pill (files, an image, a link, text) | The island stops being click-through for that drag only, and while Windows says it's over the pill, the pill grows into the drop target: the shelf's mark and "Drop to hold" in `label.secondary`, `island.shelf.drop-grow` wider and as tall as a peek, on `motion.spring.expand` (critically damped). Letting go holds it; moving away folds it back. Never over an open card other than the shelf's, never paused, hidden or in fullscreen. Effects: Copy, else Link, never Move |
| Press a held file's card, or the shelf's pill, and move past the system's drag distance | Windows' own drag, with the shell's image of the files; the pill drags every file held. Copy or Link, never Move: the original stays where it is |
| Right-click the pill, or the tray icon | Short menu: Settings, Pause ReUI for 1 hour / Resume, Hide the island / Show the island, Clipboard history, Devices, System stats, Shelf (Open shelf, Clear shelf), Timer, Quit. A left-click on the tray icon opens Settings |
| Click the minimal bubble | Swaps that activity into the pill |
| Click a tab on the view switcher (or wheel over it; Ctrl+Tab while the card has the keyboard) | Sends that place's own open command (the tray's); its card replaces the open one as a summoned card. The switcher and the gap above it count as the card for hover; a press there isn't "outside" |
| A brief confirmation (copied) while something else is in the pill | Shows in the minimal bubble for its moment (copy icon), never takes the pill; alone, it takes the pill with a glimpse of what was copied |
| Mouse wheel over the island | Cycles through all activities; over an open card with a row of cards it scrolls the row instead (horizontal wheel too); over a text block it scrolls the block |
| Click on a card that asks something (choices, a field, or an agent moment's buttons) | The card takes the keyboard without taking focus: ↑/↓ move (the list scrolls to keep the cursor in view), 1–9 pick, Space picks the choice under the cursor, Enter picks it (one choice) or presses the main button (`Primary`, else the `Accent` one), Shift+Enter presses a moment's first non-`Plain` button for its second meaning (`ACTION_ALT`: Deny → deny and say why), Esc gives the keyboard back. A card that arrives by itself never does, so a stray Enter can't answer it; the next moment is a new activity and never inherits it |
| Alt+click a button on an agent moment | Its second meaning (`alt:` + its action); elsewhere a plain click |

**Choosing what's in front.** The hub orders activities by priority. The activity you bring forward (bubble or wheel) stays in front until something more urgent arrives (anything above both it and *ongoing*, such as a permission request or a done alert); when that's gone, your choice comes back. The minimal bubble shows a **different kind** of activity from the pill when there is one (Claude in the pill → music in the bubble, and the other way round), so several sessions of one kind never hide another kind; the rest are a wheel away. This lives in the island only (`crates/reui-island/src/order.rs`, unit-tested); the hub's order is unchanged.

**Visibility modes** (Settings → General; saved in `%APPDATA%\ReUI\settings.json`, which the island watches):

| Mode | Shows |
| --- | --- |
| Always | The resting pill is always there (the default when settings.json has no `visibility`; a brand-new user's first run writes "When something is happening") |
| When something is happening | Only while there's at least one activity |
| Only when it needs me | Only for decisions and alerts (permission requests, questions, done, failed) |

**Hidden** (the hide shortcut, Win+Alt+I by default, or the tray; settings.json `hidden`, `hide_shortcut`): over any mode, the island stays out of sight until asked again, like a pause for the island alone: modules and clipboard recording carry on, decisions still come through, and a card the user summons with its own shortcut shows. The top edge doesn't reveal it. The hub registers the shortcut on its shared hotkey window and sends the island `ToggleHidden`; the island keeps the state.

In the hidden modes, pushing the pointer against the top edge above the island reveals it; it hides again `motion.delay.reveal-hold` after the pointer leaves. Showing and hiding use the presence animation (`motion.show`): the island emerges from the top edge (slides down, scales up from `from-scale` with a slight overshoot) and retracts into it, quicker, on hide.

## Appearance

The user picks a look in Settings → Appearance (`settings.json`; PRD: `docs/PRD-appearance.md`). `reui_design::look` turns those settings into a **`Look`**: the theme's `Palette`, the radii, insets and gaps, the size and the gap from the top. The island's renderer and pill draw only from it (never from colour or inset constants), and every Settings page's island preview draws from the same `Look`. A new look rebuilds the pill, like a DPI change; nothing is checked per frame.

| Setting | What changes |
| --- | --- |
| **Style** | *Island*: floats `top_gap` below the edge, the second activity in a detached bubble. *Notch*: flush with the top edge, flat top, square top corners joined to the edge by concave **ears** (`notch.ear`; a square minus a quarter circle, overlapping the shape by one DIP so no seam shows), bottom corners from the roundness; the second activity sits in the notch's trailing end, `notch.join-gap` away, flat-topped. A resting notch is `notch.idle-width` × `notch.idle-height` |
| **Theme** | `theme.dark` (today's colours) or `theme.light`: a white island with a 1 px `hairline` (8 % black) just inside its edge, black ink, Apple's light labels, fills and system colours. *Auto* follows Windows' app mode, re-read only when Windows broadcasts a colour change. Content colours (artwork, logos, swatches, an activity's accent) are never inverted |
| **Accent** | The default accent (activities without their own): ReUI blue (`color.accent.default`) or the Windows accent |
| **Roundness** | 0..1: the card radius from `look.roundness.min-radius` (10) to 44 (the panel's to 34), the compact pill and bubble from 10 to a capsule |
| **Padding** | Compact / Regular / Roomy scale the expanded `inset`, column, section and row gaps by `look.padding.*` (whole DIPs); max heights grow with Roomy so the same content fits |
| **Size** | `look.size.min..max`: the pill is drawn at the display's scale times this, so every dimension and the type scale together |

**Concentric in every combination.** Anything one inset inside an edge takes `concentric(outer, inset) = outer − inset`, never under `radius.min` (4). Controls are capsules only while the concentric radius allows one (within `radius.capsule-slack`), otherwise they take it (square-ish looks get square-ish buttons). Round content in the compact pill takes `concentric(pill end, side-margin)`, a circle at full roundness. Rings stay rings. Tested for every roundness × padding in `look.rs`.

## Tokens

| Group | Contents |
| --- | --- |
| `font` | `display` (Segoe UI Variable Display, 20 px and up), `text` (Segoe UI Variable Text), `icons` (Segoe Fluent Icons), `mono` (Cascadia Mono) |
| `color.island` | Island background (pure black) |
| `color.label` | `primary` #EBEBF0 (hero), `secondary` #8E8D94 (detail), `tertiary` (captions), `on-accent` (text on white or accent fills) |
| `color.fill` | Apple's grey fills for secondary buttons and tiles; `track` for unfilled rings, bars and progress |
| `color.system` | Apple dark system palette: red, orange, yellow, green, mint, teal, cyan, blue, indigo, purple, pink, gray |
| `color.accent.default` | Accent when an activity doesn't send one |
| `theme` | `dark` and `light`: the same keys each (background, hairline, labels, fills, track, red / orange / green), compiled into a `Palette` per theme |
| `look` | The Appearance ranges: `roundness.min-radius`, `padding.compact` / `roomy`, `size.min` / `max`, `top-gap.max` |
| `notch` | `ear`, `idle-width`, `idle-height`, `join-gap` |
| `opacity` | `tint-fill` (accent-tinted fills), `glow` (0 = off), `spark-fill` (the accent under a sparkline, fading to nothing) |
| `typography` | `eyebrow` 13/16 semibold · `value` 34/38 semibold tabular · `headline` 20/24 semibold · `code` 15/20 Cascadia Mono · `detail` 15/20 · `compact` 15/20 semibold tabular · `button` 15/20 semibold · `caption` 12/16 tabular · `context` 13/16 medium · `choice` (= button) · `choice-detail` 13/16 · `block-code` 12/16 Cascadia Mono · `block-prose` 13/18 · `gauge-label` 13/16 medium · `gauge-unit` 20/24 semibold · `fact-label` (= caption) · `fact` 15/20 tabular |
| `icon` | Glyph sizes for compact, button, expanded, the header's end button |
| `space` | 2 · 4 · 8 · 12 · 16 · 20 · 24 |
| `island` | Top offset, height, idle width, compact (centre gap, round and text margins), minimal and expanded geometry (widths per size, max heights, inset, column, line, section, row and strip gaps), the view switcher (`gap` under the card, `inset`, `tab-gap`, `rise` as it appears) |
| `radius` | `concentric` (expanded radius − inset, at the default look), `chip`, `min` (concentric radii never go under it), `capsule-slack` |
| `component` | Ring, art, app icon, button (with the roomy card's alert-style inset and gap), progress, bars, dot, choice, block, gauge (gap, glyph, sparkline) and fact sizes, the header button's target |
| `motion.spring` | `expand`, `collapse`, `peek`, `settle`, `reduced` (damping ratio + period) |
| `motion.height` | Height springs are slightly calmer than width: `damping-offset`, `period-factor` |
| `motion.delay` | `collapse`, `hover-intent`, `decision-hold`, `reveal-hold`, `fade` |
| `motion.show` | Presence animation: `in`, `out`, `from-scale`, `rise` |
| `motion.logo` | Brand-logo working animation (ticks: `interval`, `turn`); built but unused, logos are static for now |
| `motion.crossfade` | When compact content fades out and expanded content fades and scales in, as fractions of the morph |
| `motion.bars` | `fps` the island moves live level bars at |
| `settings` | The Settings window: window and caption, nav, page, row wells (outer and inner radii, 2 gap), controls (capsules, toggle, key caps, focus ring), menu, scrollbar, its colours, motion and type |
| `prefs` | The Settings window in the macOS System Settings style: sidebar, page tiles and their colours, hero card and its stage (the wallpaper's dim, the room under the island), grouped cards and hairlines, macOS controls, type and motion |

Format notes: dimensions are DIPs and durations are ms (plain numbers, or `{ "value", "unit" }`); colours are `"#RRGGBB"` or `{ "hex", "alpha" }`; `{group.token}` aliases resolve at build time; `$type` is inherited from the group. `spring` is a ReUI extension type: `{ "dampingRatio", "period" }`.

## Components

Elements a module can put in a region (`reui_protocol::Element`). Sizes depend on the slot.

| Element | Compact / minimal | Expanded leading | Expanded trailing |
| --- | --- | --- | --- |
| `Text { tone }` | `compact` type | `detail` type | `detail` type |
| `Glyph { tone }` | `icon.compact` | `icon.tile` | `icon.trailing` |
| `Art::Gradient` / `Art::Image` | 24 circle | 56, `radius.concentric` | 32 circle |
| `Art::Glyph` | Accent-tinted circle, `icon.tile-compact` glyph | Accent-tinted 56 tile, `icon.tile` glyph | 32 circle |
| `Art::Logo` | A single-colour SVG mark drawn bare, no tile or background, at `opacity.logo-bare` of its spot (rounded to whole DIPs, placed on whole pixels): an agent's brand mark (`design/assets/agents/`: Claude, OpenAI for Codex, OpenCode), which keeps its own colours (Claude's orange) or takes the primary label colour, or a device drawing (see *Device art*) in the primary label colour | Same in the 56 spot | Same in the 32 spot |
| `Ring(progress)` | 24, stroke 3 | 56, stroke 6 | 56, stroke 6 |
| `Bars { live }` | Live visuals moving at `motion.bars.fps` while live and shown (in the pill, the open card and the minimal bubble, each in its own activity's accent); resting levels otherwise, flat when not live | Same | Same |
| `Dot` | 8, accent | same | same |
| `Level { value, dim }` | As the compact trailing side, the **whole pill is the meter** (`island.compact.meter-width`): the pill is the track, an accent fill at `opacity.meter-fill` grows from the left, inset `meter-inset` so its round ends stay concentric with the pill's; the leading glyph sits bare in the fill's left end and the percentage (tabular) at the right end. `dim` (muted) dims the fill and quiets the number. In the bubble: a ring | Accent bar on a track | Same |
| `Button` | Its glyph or text (not pressable: a click on the pill opens it) | — | At the header's end: a `Plain` glyph bare on the first line (see the layout templates), otherwise an inline circle or capsule |

Images are sent once per surface (`HubMsg::Image`) ahead of the first snapshot that uses them, decoded with WIC, downscaled to 256 px on the short side and drawn cover-fitted (centre crop) into the shape. Until they arrive, a `fill.tertiary` placeholder of the same shape shows.

**Device art.** ReUI's own line drawings of devices (`design/assets/devices/`, rules and how to add one in its `README.md`), one per kind: earbuds, headphones, speaker, gamepad, mouse, keyboard, pen, watch, phone, and the Bluetooth rune for an unknown device. Sent as `Art::Logo`, so the island inks them in the theme's primary label colour and the dark and light looks need no extra drawings. Each comes twice: `<id>.svg` on a 48 grid with a 3 stroke for the open card's 56 spot (a bare mark there is 48 DIP), and `<id>-small.svg` on a 21 grid with a 1.5 stroke and no inner detail for the compact 24 spot (21 DIP), so one unit is one pixel at 100 % and two at 200 %. Strokes only (`currentColor`), round caps and joins, Fluent's weight and corner rounding, straight strokes on the pixel grid. `cargo test -p reui-island device_art_sheet -- --ignored` draws all of them as the island does into `target/devices-art/` (pill and card, dark and light, 100 % and 200 %, and magnified beside the Fluent glyphs).

Tones: `Primary`, `Secondary` (label colours), `Accent` (the activity's accent), `Positive` / `Warning` / `Negative` (system green / orange / red).

Hero (`Headline`): `Value` (numbers: `value` type, tabular figures), `Text` (`headline` type) or `Code` (`code` type in a `fill.tertiary` chip, radius `radius.chip`). An empty `Text` is no hero line at all: a row is the hero (gauges), or there's no header (a panel).

Rows: `Progress { fraction, leading, trailing }` (accent bar on track, tabular captions), `Timeline` (the same, advanced by the island, click to seek when allowed), `Buttons`, `Items`, `Search { query, buttons }` (a capsule field; the island owns the text while typing and sends `search:<q>`), `Tabs` (capsules with a quieter tabular count, selected one filled), `Note` (one quiet line, e.g. "No matches"), `Choices` and `Block` (see Agent moments below), `Gauges` and `Facts` (see System stats below), `Mixer` (see Music rows).

`Items` is a row of up to four **rich preview cards** (`component.card`: 116 high, radius 14, 8 gap and padding). Card faces: an image cover-fitted; text and code wrapped (`card-text`, `card-code`) in whole lines, the last ellipsised; a colour as a full swatch with its hex value in black or white, whichever reads; a link as the page image across the top half (`card.media`) with its title under it, or globe + title when there's no image; files as icon + name. Inside the bottom of every card is a footer (`card.footer`): the source app's icon, the age, and at most one fact on the right ("640 × 400", "4 lines", "3 files", the link's site). Over images the footer sits on a scrim. The whole card is the click target. The card under the pointer shows **delete and pin** in its top-right corner (`card.action` circles over the island's background at `opacity.card-action`, `card.action-gap` apart and from the edges; a pinned star is filled in the accent): the commonest actions without right-click, and clicking them keeps the panel open.

More than four cards show as 4½, the half card saying there's more; the row scrolls with eased motion on a timer that stops when settled, and is cut at the content inset (never past the pill's curve). The selected card (keyboard) has an accent ring `card.ring-gap` outside it, following its radius.

A summoned card with a search field takes the **keyboard without taking focus** (a low-level hook while it's open), so the app you're in stays in front and receives the paste.

**Header alignment:** the leading art's top sits one inset from the top, the same as its left edge; text is centred on the art when it fits, otherwise starts at the inset. Image art gets a 1 px inner hairline (`opacity.art-stroke`) so dark covers keep their edge on the black island.

**Mixer** (the volume card's each-app levels, `component.mixer`, layout `rows::mixer_line` / `mixer_list`): under the volume slider, one 32-high line per app, 8 apart. The app's icon (24, corners a quarter of its size, the exe's own icon) sits centred in the spot the volume's mute button has above, and is that app's mute: muted, it's dimmed under the muted-speaker glyph and its bar's fill dims as a muted `Level` does. Its name (`caption`, secondary) sits over its bar, and the bar's ends are the volume bar's, so the levels read as one column under the output menu's empty spot. While a bar is dragged its percentage shows at the right end of the name's line and its thumb on the bar. More than `mixer.max-height` (128: three apps and the edge of a fourth, which keeps the card within `max-height`) scrolls inside the card; then the wheel scrolls the list rather than a level. The header's trailing glyph button (the equalizer, `Secondary`; `Accent` while on) turns it on and off. No row and no note when nothing plays.

**Calls** (privacy module, `docs/PRD-calls.md`): iOS's call green as the accent. Compact: the app's icon · the call's time (`Clock`, counting up); muted, the leading side is a bare mic-off glyph in `Negative`. Minimal: the phone glyph in the accent (mic-off, `Negative`, while muted). Open: eyebrow "Call" / "Video call", hero the time (`Headline::Clock`), detail the app and one fact ("Discord · Microphone muted", "Teams · Sharing screen"), then the mic glyph button (`Accent` while muted) and "Open <app>".

**Music rows:** `Slider` (volume: mute button, accent bar, wheel ±5 %, trailing menu for the output device),
 `Lyrics` (current line primary, next line secondary, two lines tall so the card doesn't jump), `Timeline` with drag-to-scrub (thumb only while hovering or dragging) and "LIVE" for streams. Cards stay black: no colour washes (a `tint` option exists in the protocol but nothing uses it; it looked muddy). A `pager` shows "‹ 1/2 ›" at the top right. The real visualizer drives the bars from the activity's `audio` process at the same 30 fps timer, and falls back to synthetic levels.

A view whose only content is cards asks for the **wide** island (`island.expanded.wide-width`) and has no header at all.

**Agent moments** (a permission, a question, a plan) ask for the **roomy** island (`roomy-width`, between a timer card and the clipboard panel), modelled on iOS system alerts:

- **Session strip** (`Context`): one quiet line above the header, the agent's mark (bare, no tile), the project in secondary and the facts (what you asked, time in turn) in tertiary, cut first. The pager sits on it: the moments waiting ("‹ 1/2 ›", oldest first), or a question's steps when nothing else waits (then a step is a fact in the detail, "Features · 2 of 3", and Back a button). Compact shows the kind in the accent ("Allow?", "Question", "Plan"), or how many wait.
- **Header**: the tool's glyph bare in the accent (no tile; a question has none), eyebrow ("Permission", "Question", "Plan"), hero, and a detail that wraps to a second line instead of losing its end. The hero never repeats the body: a command shown whole below is introduced by the agent's description (or its shell), the detail by the shell and line count.
- **Answer field**: a `Search` row with the answer glyph ("Type an answer", "Tell Claude what to do instead", "What should change?"); while it's open for a deny or a plan's feedback, the buttons are Cancel and the action it sends.
- **Buttons**: equal capsules `button.roomy-inset` (16) from the edges and the bottom, `roomy-gap` (12) apart. The main action is `Accent` (tinted, accent text, like iOS's preferred action), the rest `Secondary`; `Plain` for "Answer in terminal". A short request with no body and no Always uses two inline circles beside the hero instead (✕ `Destructive`, ✓ `Accent`).
- **`Choices`**: one well per option (`fill.quaternary`, `radius.concentric` or a capsule when shorter), label in `choice` type and its description in `choice-detail` (secondary). In a multi-select list a picked one is accent-tinted with a check at its right end (an empty circle otherwise); the keyboard cursor is an accent hairline on the well's edge, shown only while the card has the keyboard. A description too long for one line wraps to a second. The list stops at `choice.max-height` (184) and scrolls inside the card like a block (the wheel, or the cursor moving), so a question never pushes the card past `roomy-max-height`.
- **`Block`**: text under the header, at most `block.max-height` tall, scrolling with the wheel (`wheel-lines` a notch, no easing, so nothing runs afterwards). Code and diffs are cut monospace lines (`block-code`) in a quiet concentric well; diff lines starting with `+` / `-` take Positive / Negative. Prose (a plan) wraps bare in secondary (`block-prose`).

**System stats** asks for the roomy island too, and its hero is a row rather than the header (layout: `reui_design::layout::gauges`):

- **Header**: the eyebrow ("System", accent) alone, Task Manager's glyph bare at its end. No headline, so no hero line.
- **`Gauges`**: two columns `gauge.gap` apart. Each is a ring (`ring.expanded`, accent on the track, the share) with a quiet glyph inside (`gauge.glyph`, secondary: a chip, a memory module), then beside it the label (`gauge-label`, secondary), the numerals (`value`, primary) with a smaller unit on their baseline (`gauge-unit`, secondary) and a detail (`detail`, secondary). A detail on either keeps its line on both, so the columns read level. Under them, across the column, the last minute as a sparkline: the zero line in the track colour (`spark-track`) under the whole minute, the area fading from `opacity.spark-fill` at the line's peak to nothing at zero, the line (`spark-stroke`, accent) and the newest point as an accent dot (`spark-dot`). It holds only what was read since the card opened, newest at the right, so it fills in from the right; 0–100 % fills its height. It's drawn once per reading: nothing animates.
- **`Facts`**: one line under the gauges, each a label (`fact-label`, tertiary) over its value (`fact`, primary, tabular), spread from inset to inset. No icons, wells or dividers.
- **Colour means something**: one accent for every ring, line and area; a number that has stayed high (90 % for three readings) is `Warning`, its unit with it.

**No filler text.** A panel never explains itself ("Click an item to paste it", "N saved", hotkey hints): the cards say what they are and clicking is the only thing to do. Every word on screen has to be content or a fact about content.

Buttons, by role:

| Role | Text button | Glyph button |
| --- | --- | --- |
| `Primary` | White capsule, black text | White circle, black glyph |
| `Secondary` | Grey fill capsule | Grey circle |
| `Accent` | Accent-tinted capsule, accent text | Accent-tinted circle |
| `Destructive` | Red-tinted capsule, red text | Red-tinted circle |
| `Plain` | Bare text | Bare glyph (media previous / next) |

Text buttons are 44 high capsules; inline glyph buttons are 40 circles; transport glyph buttons are 44 circles, 28 apart.

## Settings window

`reui-settings` follows macOS System Settings (dark), with the island's type (tokens under `prefs`; most of the older `settings` group is no longer read). A solid sidebar (#252527) holds a search field, the ReUI header row (icon, name, version), then the pages, each with a coloured icon tile (General grey, Appearance mint, Now Playing pink, Clipboard blue, Volume orange, Devices green, Timers red, Downloads & screenshots cyan, Agents purple, Privacy indigo); the selected page is an accent-filled row. The content has back/forward and the page title in its own caption row, then a hero card: the page's island on a stage that is the top of the user's screen (see *Settings previews* below), under it the page icon, title, one paragraph and the module's master toggle. Groups are a small eyebrow over one rounded card whose rows are split by inset hairlines. A row is a title, one quiet line, and its controls on the right: macOS-style toggles, segmented controls, steppers/popups, key caps, capsule buttons (a destructive one asks a second time). Keyboard focus is an accent ring following the control's radius. No filler: every line is a fact about the setting.

### Settings previews

Each feature page's hero card opens with a **stage that is the top of the user's screen**: their own wallpaper, cropped exactly as Windows shows it at the top centre of the primary monitor (fit, fill, stretch, centre, tile, span, and the background colour around a fitted or centred image), shown 1 DIP to 1 DIP under a `prefs.opacity.stage-dim` black so Settings stays calm. The stage's top edge is the screen's edge. No glows, gradients or page tints of our own. Without an image (a solid colour, a slideshow or Spotlight file that's gone, a decode failure) it's the wallpaper's solid colour, else `prefs.color.stage`. The wallpaper is read once (and again when Windows says it changed) and never leaves the machine.

**One island** hangs from that edge where it really sits: centred, `top_gap` below the edge, or joined to it with ears as a notch, at its real size (the look's size; 1 DIP = 1 DIP), in the user's theme, roundness and padding — every page, not just Appearance. It shows the **one state the page is about**, drawn as the island draws it (same tokens, the same layout functions as the island's renderer, `reui_design::layout`, the activity's own accent or the look's): General the compact pill with a second activity beside it, Appearance and Timers a running timer's card (with the view switcher under it while that's on: a running timer is the Timers place), Now Playing the open card, Clipboard the panel (or, when the panel is wider than the stage at the look's size, the toast a copy shows), Volume & brightness the meter pill, Devices the connect card, System stats the open card (CPU and memory gauges with a quiet minute, the facts under them), Coding agents the permission card (session strip, Permission → command → what it's for, Deny / Always / Allow). Content is plain and plausible: no invented brands, no artwork blobs — where the island would show artwork it shows what it shows without one, the accent-tinted glyph tile. The stage is as tall as the island plus `prefs.header.stage-room` (at least `stage-min`). When a page opens the island drops in from the edge once; at rest nothing moves.

Rendering: D3D11 + a Direct2D device context on a flip-model swap chain paced by its frame-latency waitable object, so motion runs at display rate (≈200 fps on a 200 Hz panel, ~0.3 ms CPU per frame) and the window does nothing at rest. Text layouts and page bitmaps are cached; `--perf` prints frame timings, `perf/harness.ps1` drives it.

## Motion

- The pill's width and height are springs (`motion.spring.*`); everything else is derived from them inside DWM, so morphs cost our process nothing and stay smooth when it's busy.
- Opening overshoots a little (`expand`); closing settles (`collapse`); returning to the core is calm (`settle`).
- Compact content is gone in the first quarter of an expansion; expanded content fades and scales in from `expanded-scale-from` after `expanded-in-start`.
- When Windows' "Animation effects" setting is off, every morph uses `motion.spring.reduced` (critically damped, short).
- Allowed to update while visible: countdowns and progress (once a second at most). Not allowed: idle loops, shimmer, pulsing.

## Open items

- **Continuous corners:** the island and tiles use circular arcs. Apple uses continuous (squircle-like) curvature for the expanded island; worth trying once the shape is final.
- **Clipboard v2:** OCR search, user categories, drag-out, large preview (see `docs/PRD-clipboard.md`).
- **Lock-screen style banner** from the Figma kit (14 px margins, 84–160 high) for notifications.
- **Image decoding** happens on the UI thread (once per image); move it to the hub-link thread if large images cause a hitch.
- **Figma sync:** tokens are hand-copied. Next: a ReUI Figma file with these tokens as variables, pulled with the Figma MCP (`get_variable_defs`).
