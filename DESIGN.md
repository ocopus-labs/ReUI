# ReUI Design System

The look and behaviour of every ReUI surface. The island is the first surface; the dock, taskbar and desktop of the future shell will use the same system.

**Where things live**

| What | Where |
| --- | --- |
| Tokens (source of truth) | `design/tokens.json`, W3C Design Tokens format |
| Tokens in code | `reui-design` crate, generated at build time: `tokens::color::label::PRIMARY` etc. |
| Components and layout | `crates/reui-island/src/render/` (one file per presentation and row) |
| Presentations and motion | `crates/reui-island/src/pill/` |
| Semantic view model (what modules may say) | `crates/reui-protocol` |
| Logo | `design/logo/reui.ico` (+ a PNG per size), drawn by `tools/logo.ps1` on each size's own pixel grid: a light screen with the black island at its top and its bubble in `color.system.blue`. Every program links it in (`reui_design::icon::embed()` in its `build.rs`); the tray, the Settings window and Apps & features load it; Settings' sidebar draws the same mark |
| References | `design/reference/` (Figma kit export, moodboard in `inspiration/`) |
| Figma | [Dynamic Island – Live Activities (Community)](https://www.figma.com/design/Oavznl8mcr41wrSfXdpCNs/Dynamic-Island---Live-Activities--Community-) by Jasper Cheyney; island geometry comes from here |
| Figma (system alerts) | [iOS 17 Dynamic Island Components (Community)](https://www.figma.com/design/Q5jcdhxYldk71bDnC5VzIE/iOS17-Dynamic-Island-Components--Community-); its alerts with buttons shape the agent cards (see `docs/PRD-agent-moments.md` §5.8) |

**The rule that keeps redesigns cheap:** modules describe *meaning* (priority, regions, element kinds, tones, button roles, an accent colour). Only the island decides *appearance*, and only from tokens. A redesign changes `tokens.json` and `render.rs`, never a module or the protocol. No hard-coded sizes, colours or fonts outside the token file.

## Principles

1. **Black and quiet.** The island is a pure black shape that blends into the top of the screen. Colour comes from one accent per activity, used sparingly: the eyebrow, a ring, the bars, one tinted button.
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
| **Idle** | No activities | Black capsule, `island.idle.width` × `island.height` |
| **Compact** | One activity in front | `leading` · `center-gap` · `trailing`. Both sides are the **same width** (the wider side, margin included). Round content (art, ring) is inset `side-margin` so it sits concentric in the capsule's end; text and bare glyphs keep `text-margin` clear of the curve. Text is capped at `max-side`. Apple reserves 126 in the middle for the camera; Windows has no cutout, so the gap is only 48. **A text label** (copied, Caps Lock on, a device, "Allow?") instead sits `joined-gap` after its mark and the pill is only as wide as the two; a level fills the whole pill (see `Level` below) |
| **Peek** | Pointer over compact | Compact grown by `island.compact.peek-grow` |
| **Minimal** | A second activity exists | Detached circle `island.minimal.size`, `gap` to the right of the pill, showing the second activity's `minimal` element |
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

## Behaviour

| Event | Island does |
| --- | --- |
| First activity arrives | Idle → compact |
| Decision arrives (e.g. agent permission) | Expands itself, then folds back to compact (still pending, accent "Allow?") after `motion.delay.decision-hold` unless you engage |
| Alert arrives (done, failed, "your turn") | Comes to the front in compact; doesn't expand itself |
| Decision answered | Folds back to compact (unless hovered) |
| Pointer rests on the pill for `motion.delay.hover-intent` | Becomes clickable and peeks. Until then it's click-through, so quick moves and clicks reach what's underneath (browser tabs) |
| Click on compact | Expand |
| Pointer leaves expanded | Collapse after `motion.delay.collapse` |
| Fullscreen app in front (game, video, browser video, presentation) | Hide entirely (`hide_in_fullscreen`, on by default) |
| Something dragged over it (a tab, a file, a selection) | Nothing: no peek, no clicks, until the button is up and the pointer rests |
| Right-click the pill, or the tray icon | Short menu: Settings, Pause ReUI for 1 hour / Resume, Clipboard history, Devices, Quit. A left-click on the tray icon opens Settings |
| Click the minimal bubble | Swaps that activity into the pill |
| A brief confirmation (copied) while something else is in the pill | Shows in the minimal bubble for its moment (copy icon), never takes the pill; alone, it takes the pill with a glimpse of what was copied |
| Mouse wheel over the island | Cycles through all activities; over an open card with a row of cards it scrolls the row instead (horizontal wheel too); over a text block it scrolls the block |
| Click on a card that asks something (choices or a field) | The card takes the keyboard without taking focus: ↑/↓ move, 1–9 pick, Space picks the choice under the cursor, Enter picks it (one choice) or presses the main button (several), Esc gives the keyboard back. A card that arrives by itself never does, so a stray Enter can't answer it |

**Choosing what's in front.** The hub orders activities by priority. The activity you bring forward (bubble or wheel) stays in front until something more urgent arrives (anything above both it and *ongoing*, such as a permission request or a done alert); when that's gone, your choice comes back. The minimal bubble shows a **different kind** of activity from the pill when there is one (Claude in the pill → music in the bubble, and the other way round), so several sessions of one kind never hide another kind; the rest are a wheel away. This lives in the island only (`crates/reui-island/src/order.rs`, unit-tested); the hub's order is unchanged.

**Visibility modes** (Settings → General; saved in `%APPDATA%\ReUI\settings.json`, which the island watches):

| Mode | Shows |
| --- | --- |
| Always | The resting pill is always there (the default when settings.json has no `visibility`; a brand-new user's first run writes "When something is happening") |
| When something is happening | Only while there's at least one activity |
| Only when it needs me | Only for decisions and alerts (permission requests, questions, done, failed) |

In the hidden modes, pushing the pointer against the top edge above the island reveals it; it hides again `motion.delay.reveal-hold` after the pointer leaves. Showing and hiding use the presence animation (`motion.show`): the island emerges from the top edge (slides down, scales up from `from-scale` with a slight overshoot) and retracts into it, quicker, on hide.

## Tokens

| Group | Contents |
| --- | --- |
| `font` | `display` (Segoe UI Variable Display, 20 px and up), `text` (Segoe UI Variable Text), `icons` (Segoe Fluent Icons), `mono` (Cascadia Mono) |
| `color.island` | Island background (pure black) |
| `color.label` | `primary` #EBEBF0 (hero), `secondary` #8E8D94 (detail), `tertiary` (captions), `on-accent` (text on white or accent fills) |
| `color.fill` | Apple's grey fills for secondary buttons and tiles; `track` for unfilled rings, bars and progress |
| `color.system` | Apple dark system palette: red, orange, yellow, green, mint, teal, cyan, blue, indigo, purple, pink, gray |
| `color.accent.default` | Accent when an activity doesn't send one |
| `opacity` | `tint-fill` (accent-tinted fills), `glow` (0 = off) |
| `typography` | `eyebrow` 13/16 semibold · `value` 34/38 semibold tabular · `headline` 20/24 semibold · `code` 15/20 Cascadia Mono · `detail` 15/20 · `compact` 15/20 semibold tabular · `button` 15/20 semibold · `caption` 12/16 tabular · `context` 13/16 medium · `choice` (= button) · `choice-detail` 13/16 · `block-code` 12/16 Cascadia Mono · `block-prose` 13/18 |
| `icon` | Glyph sizes for compact, button, expanded |
| `space` | 2 · 4 · 8 · 12 · 16 · 20 · 24 |
| `island` | Top offset, height, idle width, compact (centre gap, round and text margins), minimal and expanded geometry (widths per size, max heights, inset, column, line, section, row and strip gaps) |
| `radius` | `concentric` (expanded radius − inset), `chip` |
| `component` | Ring, art, app icon, button (with the roomy card's alert-style inset and gap), progress, bars, dot, choice and block sizes |
| `motion.spring` | `expand`, `collapse`, `peek`, `settle`, `reduced` (damping ratio + period) |
| `motion.height` | Height springs are slightly calmer than width: `damping-offset`, `period-factor` |
| `motion.delay` | `collapse`, `hover-intent`, `decision-hold`, `reveal-hold`, `fade` |
| `motion.show` | Presence animation: `in`, `out`, `from-scale`, `rise` |
| `motion.logo` | Brand-logo working animation (ticks: `interval`, `turn`); built but unused, logos are static for now |
| `motion.crossfade` | When compact content fades out and expanded content fades and scales in, as fractions of the morph |
| `motion.bars` | `fps` the island moves live level bars at |
| `settings` | The Settings window: window and caption, nav, page, row wells (outer and inner radii, 2 gap), controls (capsules, toggle, key caps, focus ring), menu, scrollbar, its colours, motion and type |
| `prefs` | The Settings window in the macOS System Settings style: sidebar, page tiles and their colours, hero card and preview wash, grouped cards and hairlines, macOS controls, type and motion |

Format notes: dimensions are DIPs and durations are ms (plain numbers, or `{ "value", "unit" }`); colours are `"#RRGGBB"` or `{ "hex", "alpha" }`; `{group.token}` aliases resolve at build time; `$type` is inherited from the group. `spring` is a ReUI extension type: `{ "dampingRatio", "period" }`.

## Components

Elements a module can put in a region (`reui_protocol::Element`). Sizes depend on the slot.

| Element | Compact / minimal | Expanded leading | Expanded trailing |
| --- | --- | --- | --- |
| `Text { tone }` | `compact` type | `detail` type | `detail` type |
| `Glyph { tone }` | `icon.compact` | `icon.tile` | `icon.trailing` |
| `Art::Gradient` / `Art::Image` | 24 circle | 56, `radius.concentric` | 32 circle |
| `Art::Glyph` | Accent-tinted circle, `icon.tile-compact` glyph | Accent-tinted 56 tile, `icon.tile` glyph | 32 circle |
| `Art::Logo` | Brand SVG (`design/assets/`) on an accent-tinted circle, at `opacity.logo` of the tile; keeps its own colours, or takes the accent if it has none | Same on the 56 tile | Same on the 32 circle |
| `Ring(progress)` | 24, stroke 3 | 56, stroke 6 | 56, stroke 6 |
| `Bars { live }` | Live visuals moving at `motion.bars.fps` while live and shown (in the pill, the open card and the minimal bubble, each in its own activity's accent); resting levels otherwise, flat when not live | Same | Same |
| `Dot` | 8, accent | same | same |
| `Level { value, dim }` | As the compact trailing side, the **whole pill is the meter** (`island.compact.meter-width`): the pill is the track, an accent fill at `opacity.meter-fill` grows from the left, inset `meter-inset` so its round ends stay concentric with the pill's; the leading glyph sits bare in the fill's left end and the percentage (tabular) at the right end. `dim` (muted) dims the fill and quiets the number. In the bubble: a ring | Accent bar on a track | Same |

Images are sent once per surface (`HubMsg::Image`) ahead of the first snapshot that uses them, decoded with WIC, downscaled to 256 px on the short side and drawn cover-fitted (centre crop) into the shape. Until they arrive, a `fill.tertiary` placeholder of the same shape shows.

Tones: `Primary`, `Secondary` (label colours), `Accent` (the activity's accent), `Positive` / `Warning` / `Negative` (system green / orange / red).

Hero (`Headline`): `Value` (numbers: `value` type, tabular figures), `Text` (`headline` type) or `Code` (`code` type in a `fill.tertiary` chip, radius `radius.chip`).

Rows: `Progress { fraction, leading, trailing }` (accent bar on track, tabular captions), `Timeline` (the same, advanced by the island, click to seek when allowed), `Buttons`, `Items`, `Search { query, buttons }` (a capsule field; the island owns the text while typing and sends `search:<q>`), `Tabs` (capsules with a quieter tabular count, selected one filled), `Note` (one quiet line, e.g. "No matches"), `Choices` and `Block` (see Agent moments below).

`Items` is a row of up to four **rich preview cards** (`component.card`: 116 high, radius 14, 8 gap and padding). Card faces: an image cover-fitted; text and code wrapped (`card-text`, `card-code`) in whole lines, the last ellipsised; a colour as a full swatch with its hex value in black or white, whichever reads; a link as the page image across the top half (`card.media`) with its title under it, or globe + title when there's no image; files as icon + name. Inside the bottom of every card is a footer (`card.footer`): the source app's icon, the age, and at most one fact on the right ("640 × 400", "4 lines", "3 files", the link's site). Over images the footer sits on a scrim. The whole card is the click target.

More than four cards show as 4½, the half card saying there's more; the row scrolls with eased motion on a timer that stops when settled, and is cut at the content inset (never past the pill's curve). The selected card (keyboard) has an accent ring `card.ring-gap` outside it, following its radius.

A summoned card with a search field takes the **keyboard without taking focus** (a low-level hook while it's open), so the app you're in stays in front and receives the paste.

**Header alignment:** the leading art's top sits one inset from the top, the same as its left edge; text is centred on the art when it fits, otherwise starts at the inset. Image art gets a 1 px inner hairline (`opacity.art-stroke`) so dark covers keep their edge on the black island.

**Music rows:** `Slider` (volume: mute button, accent bar, wheel ±5 %, trailing menu for the output device), `Lyrics` (current line primary, next line secondary, two lines tall so the card doesn't jump), `Timeline` with drag-to-scrub (thumb only while hovering or dragging) and "LIVE" for streams. Cards stay black: no colour washes (a `tint` option exists in the protocol but nothing uses it; it looked muddy). A `pager` shows "‹ 1/2 ›" at the top right. The real visualizer drives the bars from the activity's `audio` process at the same 30 fps timer, and falls back to synthetic levels.

A view whose only content is cards asks for the **wide** island (`island.expanded.wide-width`) and has no header at all.

**Agent moments** (a permission, a question, a plan) ask for the **roomy** island (`roomy-width`, between a timer card and the clipboard panel), modelled on iOS system alerts:

- **Session strip** (`Context`): one quiet line above the header, the agent's mark as a compact circle, the project in secondary and the facts (what you asked, time in turn) in tertiary, cut first. The pager ("‹ 1/3 ›", a question's steps) sits on it.
- **Header**: the tool's glyph bare in the accent (no tile; a question has none), eyebrow ("Permission", "Question", "Plan"), hero, and a detail that wraps to a second line instead of losing its end.
- **Buttons**: equal capsules `button.roomy-inset` (16) from the edges and the bottom, `roomy-gap` (12) apart. The main action is `Accent` (tinted, accent text, like iOS's preferred action), the rest `Secondary`; `Plain` for "Answer in terminal". A short request with no body and no Always uses two inline circles beside the hero instead (✕ `Destructive`, ✓ `Accent`).
- **`Choices`**: one well per option (`fill.quaternary`, `radius.concentric` or a capsule when shorter), label in `choice` type and its description in `choice-detail` (secondary). In a multi-select list a picked one is accent-tinted with a check at its right end (an empty circle otherwise); the keyboard cursor is an accent hairline on the well's edge, shown only while the card has the keyboard.
- **`Block`**: text under the header, at most `block.max-height` tall, scrolling with the wheel (`wheel-lines` a notch, no easing, so nothing runs afterwards). Code and diffs are cut monospace lines (`block-code`) in a quiet concentric well; diff lines starting with `+` / `-` take Positive / Negative. Prose (a plan) wraps bare in secondary (`block-prose`).

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

`reui-settings` follows macOS System Settings (dark), with the island's type (tokens under `prefs`; most of the older `settings` group is no longer read). A solid sidebar (#252527) holds a search field, the ReUI header row (icon, name, version), then the pages, each with a coloured icon tile (General grey, Now Playing pink, Clipboard blue, Volume orange, Devices green, Timers red, Agents purple, Privacy indigo); the selected page is an accent-filled row. The content has back/forward and the page title in its own caption row, then a hero card: a live island preview drawn from real components on a soft wash of the page colour, under it the page icon, title, one paragraph and the module's master toggle. Groups are a small eyebrow over one rounded card whose rows are split by inset hairlines. A row is a title, one quiet line, and its controls on the right: macOS-style toggles, segmented controls, steppers/popups, key caps, capsule buttons (a destructive one asks a second time). Keyboard focus is an accent ring following the control's radius. No filler: every line is a fact about the setting.

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
