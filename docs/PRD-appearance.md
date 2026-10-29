# PRD: Appearance (styles, themes, shape)

Status: done (M0–M2) · Owner: Rohit · Oct 29, 2026

**Ask (Rohit):** themes, light and dark, liquid glass, adjustable roundness, margins and padding, and different styles, including a Mac-like one stuck to the top of the screen. All at ReUI's performance budget. Liquid glass was dropped the same day (see §2).

**Inputs:**
- The island is native: Rust, Windows.UI.Composition through a `DesktopWindowTarget`, Direct2D/DirectWrite drawing (see `ARCHITECTURE.md`). It isn't Tauri, so no Tauri plugin applies. Everything here uses DWM and Composition, with no new crates.
- The look already lives in one place, as DESIGN.md requires: `design/tokens.json` compiled into constants, and the island's `render/` and `pill/`. Modules never change for any of this.
- Today: the shape is a `CompositionRoundedRectangleGeometry` clip with springs on its size; the window is `WS_EX_LAYERED | WS_EX_NOREDIRECTIONBITMAP | WS_EX_TRANSPARENT`; colours are compile-time constants (`color::label::PRIMARY`, about 72 uses in the island).
- **Mac notch apps** (NotchNook, Alcove, boring.notch) draw the island as part of the hardware notch: flush with the top edge, flat top, small concave "ears" where it meets the screen edge, rounded bottom corners. Everything grows down and sideways out of it.

## 1. Goals

1. **Choose a style:** the floating island (today), or a notch attached to the top edge like a Mac.
2. **Choose a theme:** Auto (follows Windows' app mode), Dark, Light.
3. **Shape it:** roundness, the gap from the top, padding, and overall size.
4. **Costs what it costs today.** 0 % CPU at rest in every combination; no new animation loops.
5. **Still ReUI.** Every combination keeps the design rules: concentric corners (inner radius = outer − inset, recomputed from the user's roundness and padding), eyebrow → hero → detail, one accent per activity.

## 2. Non-goals (v1)

- **Glass / Liquid Glass: dropped, Oct 29, 2026.** No material setting, no host backdrop. (It would have been `CreateHostBackdropBrush` + saturation + tint + a rim; Composition has no refraction, and the spike was never run.)
- Per-module colours or layouts chosen by the user; custom fonts; a theme editor or theme files.
- Animated or video backgrounds; wallpaper-matched colours beyond Windows' own accent.

## 3. Settings (new page: Appearance)

One page with a live preview at the top (Settings' island previews, drawn with the same look):

| Setting | Values | Default |
|---|---|---|
| Style | **Island** (floating) · **Notch** (attached to the top) | Island |
| Theme | Auto · Dark · Light | Dark |
| Roundness | Slider: square-ish (radius 10) … capsule (full) | Capsule, as today |
| Gap from the top | Slider 0–24 (Island only; Notch is always 0) | 11 |
| Padding | Compact · Regular · Roomy (scales `inset`, gaps) | Regular |
| Size | Slider 90–125 % (scales every island dimension and the type) | 100 % |
| Accent | Windows accent · ReUI blue | ReUI blue |

Stored in `%APPDATA%\ReUI\settings.json` (the island already watches it): `style`, `theme`, `roundness` (0..1), `top_gap`, `padding`, `scale`, `accent`. Unknown values fall back to defaults; nothing is written that the island can't read.

## 4. Experience

### 4.1 Styles

- **Island (floating):** today's capsule, `top_gap` below the screen edge, the minimal bubble detached beside it.
- **Notch (Mac-like):** the shape's top edge is flush with the screen's top (no gap) and its top corners are square, joined to the screen edge by two concave **ears** (radius = `ear` token, scaled), so it reads as part of the screen's frame. Bottom corners take the roundness setting. Compact grows sideways, expanded drops down from it, with the same springs.
  - **The second activity** sits inside the notch's trailing end, separated by a hairline gap, instead of a detached bubble (a detached circle under a notch looks broken).
  - Implementation: the existing rounded rectangle extends `radius` above the screen top (so its top corners are off-screen and the visible top is flat), plus two small ear visuals (a square with a quarter-circle cut out) that follow the shape's width spring. This keeps size springs and hit testing as they are.
  - Hidden modes slide the notch up into the edge.
- **Idle look** (both styles): the resting pill as today. In Notch style the idle notch is narrower (`notch.idle-width`) and only as tall as the menu-bar zone.

### 4.2 Themes

- **Dark:** today, pure black.
- **Light:** a white island (`#FFFFFF` with a 1 px hairline at 8 % black so it holds on white windows), primary ink black, secondary and tertiary as Apple's light labels, fills and tracks inverted. Accent tints keep their alpha.
- **Auto:** follows `AppsUseLightTheme` (`HKCU\...\Themes\Personalize`), re-read on `WM_SETTINGCHANGE` "ImmersiveColorSet". No polling.
- Artwork, logos and the meter keep their own colours in both themes; content colours are never inverted.

### 4.3 Shape: roundness, gap, padding, size

- **Roundness** sets the expanded radius from 10 up to "capsule" (today's 44), and compact/minimal radius from 10 up to half the height. Every inner radius is recomputed: `concentric = radius − inset`, clamped at 4. Buttons stay capsules only where they're at the inset and the concentric radius allows; otherwise they take the concentric radius.
- **Gap from the top** moves the island down (0–24).
- **Padding** scales `inset`, `section-gap`, `row-gap`, `column-gap` by 0.8 / 1 / 1.2.
- **Size** scales every island dimension and the type; the window and surfaces are sized from the scaled maximums.

## 5. Engineering

- **`reui-design`:** tokens gain a `theme.light` colour set alongside today's (dark) colours, plus `look.*` (ranges) and `notch.*` tokens. The build script emits a `Palette` struct for each theme in addition to today's constants.
- **`look.rs`** (pure, tested; in `reui-design` so the island and Settings' previews share it): `Appearance` (the settings keys, parsed leniently) and `Look { palette, style, roundness, top_gap, padding, size, accent }` with the concentric maths.
- **`reui-island`:**
  - The renderer and pill read colours and geometry from the `Look` instead of constants (mechanical refactor; dark/Island/100 % must render pixel-identical to today).
  - `pill/setup.rs`: shape for both styles (ears), rebuilt when the look changes (rare; never per frame).
  - Theme events: `WM_SETTINGCHANGE` → Auto re-reads the app mode and the accent.
- **`reui-settings`:** the Appearance page; its island preview uses the same `Look` (light, notch, shape).
- **Protocol:** no change. Modules never know the look.

## 6. Milestones

| # | What | Done when | Status |
|---|---|---|---|
| M0 | Runtime `Look` and palette refactor; nothing visible changes | Pixel-identical review sheets; tests | Done: compact, timer, permission and music cards identical before/after (only the live level bars differ) |
| M1 | Light / Auto theme; Appearance page with Theme, Accent | Light island readable over light and dark windows; Auto follows Windows live | Done: light over white and dark backdrops; Auto flips live in a sandbox (an `apps-theme` stand-in + "ImmersiveColorSet"), unit-tested; the page also carries the shape rows |
| M2 | Roundness, gap, padding, size; Notch style with ears and the in-notch second activity | Every combination keeps concentric corners; notch flush at 100/125/150 % DPI | Done: concentric maths tested for every roundness × padding; screenshots at roundness 0 / 0.5 / 1, padding and size extremes; notch checked at effective scales 1.0 / 1.1 / 1.25 (Size on a 100 % display; 150 % not available here) |

## 7. To verify

| ID | Question | How |
|---|---|---|
| V3 | Notch ears at fractional DPI (125 %, 150 %): no seam between the ears and the shape | Screenshots at each scale. Answer: no seam at 110 % and 125 % (drawn by Size; the ears overlap the shape by one DIP); 150 % unverified on this machine |

## 8. Open questions

- Should Notch default to Dark regardless of the theme (a notch is hardware black on a Mac)? This draft: Notch follows the theme, but Light + Notch shows a hint that Dark reads as hardware.
- A third style later: a **bar** along the whole top edge (status-bar-like) for the future shell.
