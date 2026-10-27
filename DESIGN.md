# ReUI Design System

The look and behaviour of every ReUI surface. The island is the first surface; the dock, taskbar and desktop of the future shell will use the same system.

**Where things live**

| What | Where |
| --- | --- |
| Tokens (source of truth) | `design/tokens.json`, W3C Design Tokens format |
| Tokens in code | `reui-design` crate, generated at build time: `tokens::color::label::PRIMARY` etc. |
| Components and layout | `crates/reui-island/src/render.rs` |
| Presentations and motion | `crates/reui-island/src/pill.rs` |
| Semantic view model (what modules may say) | `crates/reui-protocol` |
| References | `design/reference/` (Figma kit export, moodboard in `inspiration/`) |
| Figma | [Dynamic Island – Live Activities (Community)](https://www.figma.com/design/Oavznl8mcr41wrSfXdpCNs/Dynamic-Island---Live-Activities--Community-) by Jasper Cheyney; island geometry comes from here |

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
| **Compact** | One activity in front | `leading` · `center-gap` · `trailing`. Both sides are the **same width** (the wider side, margin included). Round content (art, ring) is inset `side-margin` so it sits concentric in the capsule's end; text and bare glyphs keep `text-margin` clear of the curve. Text is capped at `max-side`. Apple reserves 126 in the middle for the camera; Windows has no cutout, so the gap is only 48 |
| **Peek** | Pointer over compact | Compact grown by `island.compact.peek-grow` |
| **Minimal** | A second activity exists | Detached circle `island.minimal.size`, `gap` to the right of the pill, showing the second activity's `minimal` element |
| **Expanded** | Click, or a decision or alert arrives | Width `island.expanded.width`. Header: `leading` · eyebrow / hero / detail · `trailing` (or inline controls). Then full-width rows. Height fits content between `min-height` 84 and `max-height` 200 |

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
| Fullscreen app in front | Hide entirely |
| Right-click the pill, or the tray icon | Menu: visibility mode, Quit |
| Click the minimal bubble | Swaps that activity into the pill |
| Mouse wheel over the island | Cycles through all activities (also in the open card) |

**Choosing what's in front.** The hub orders activities by priority. The activity you bring forward (bubble or wheel) stays in front until something more urgent arrives (anything above both it and *ongoing*, such as a permission request or a done alert); when that's gone, your choice comes back. The minimal bubble shows a **different kind** of activity from the pill when there is one (Claude in the pill → music in the bubble, and the other way round), so several sessions of one kind never hide another kind; the rest are a wheel away. This lives in the island only (`crates/reui-island/src/order.rs`, unit-tested); the hub's order is unchanged.

**Visibility modes** (menu; saved in `%APPDATA%\ReUI\settings.json`):

| Mode | Shows |
| --- | --- |
| Always (default) | The resting pill is always there |
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
| `typography` | `eyebrow` 13/16 semibold · `value` 34/38 semibold tabular · `headline` 20/24 semibold · `code` 15/20 Cascadia Mono · `detail` 15/20 · `compact` 15/20 semibold tabular · `button` 15/20 semibold · `caption` 12/16 tabular |
| `icon` | Glyph sizes for compact, button, expanded |
| `space` | 2 · 4 · 8 · 12 · 16 · 20 · 24 |
| `island` | Top offset, height, idle width, compact (centre gap, round and text margins), minimal and expanded geometry (inset, column, line, section and row gaps) |
| `radius` | `concentric` (expanded radius − inset), `chip` |
| `component` | Ring, art, app icon, button, progress, bars and dot sizes |
| `motion.spring` | `expand`, `collapse`, `peek`, `settle`, `reduced` (damping ratio + period) |
| `motion.height` | Height springs are slightly calmer than width: `damping-offset`, `period-factor` |
| `motion.delay` | `collapse`, `hover-intent`, `decision-hold`, `reveal-hold`, `fade` |
| `motion.show` | Presence animation: `in`, `out`, `from-scale`, `rise` |
| `motion.logo` | Brand-logo working animation (ticks: `interval`, `turn`); built but unused, logos are static for now |
| `motion.crossfade` | When compact content fades out and expanded content fades and scales in, as fractions of the morph |
| `motion.bars` | `fps` the island moves live level bars at |

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

Images are sent once per surface (`HubMsg::Image`) ahead of the first snapshot that uses them, decoded with WIC, downscaled to 256 px on the short side and drawn cover-fitted (centre crop) into the shape. Until they arrive, a `fill.tertiary` placeholder of the same shape shows.

Tones: `Primary`, `Secondary` (label colours), `Accent` (the activity's accent), `Positive` / `Warning` / `Negative` (system green / orange / red).

Hero (`Headline`): `Value` (numbers: `value` type, tabular figures), `Text` (`headline` type) or `Code` (`code` type in a `fill.tertiary` chip, radius `radius.chip`).

Rows: `Progress { fraction, leading, trailing }` (accent bar on track, tabular captions) and `Buttons`.

Buttons, by role:

| Role | Text button | Glyph button |
| --- | --- | --- |
| `Primary` | White capsule, black text | White circle, black glyph |
| `Secondary` | Grey fill capsule | Grey circle |
| `Accent` | Accent-tinted capsule, accent text | Accent-tinted circle |
| `Destructive` | Red-tinted capsule, red text | Red-tinted circle |
| `Plain` | Bare text | Bare glyph (media previous / next) |

Text buttons are 44 high capsules; inline glyph buttons are 40 circles; transport glyph buttons are 44 circles, 28 apart.

## Motion

- The pill's width and height are springs (`motion.spring.*`); everything else is derived from them inside DWM, so morphs cost our process nothing and stay smooth when it's busy.
- Opening overshoots a little (`expand`); closing settles (`collapse`); returning to the core is calm (`settle`).
- Compact content is gone in the first quarter of an expansion; expanded content fades and scales in from `expanded-scale-from` after `expanded-in-start`.
- When Windows' "Animation effects" setting is off, every morph uses `motion.spring.reduced` (critically damped, short).
- Allowed to update while visible: countdowns and progress (once a second at most). Not allowed: idle loops, shimmer, pulsing.

## Open items

- **Continuous corners:** the island and tiles use circular arcs. Apple uses continuous (squircle-like) curvature for the expanded island; worth trying once the shape is final.
- **Clipboard panel:** a larger expanded variant with tabs, search and horizontal preview cards (see `inspiration/clipboard-supaste.png`); needs a "panel" presentation and scrolling.
- **Lock-screen style banner** from the Figma kit (14 px margins, 84–160 high) for notifications.
- **Image decoding** happens on the UI thread (once per image); move it to the hub-link thread if large images cause a hitch.
- **Figma sync:** tokens are hand-copied. Next: a ReUI Figma file with these tokens as variables, pulled with the Figma MCP (`get_variable_defs`).
