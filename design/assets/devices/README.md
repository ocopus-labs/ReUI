# Device drawings

ReUI's own line drawings of devices, shown in the island when a device connects, runs low, and so on. They're original works drawn for ReUI (no vendor photos, renders or third-party icon sets were copied or traced), under the repository's licence, MIT OR Apache-2.0. The Bluetooth rune in `bluetooth.svg` is the Bluetooth SIG's symbol for the technology, drawn here as a plain outline the way Windows' own icon font shows it; it names the connection, not a product.

## The set the island uses

One drawing per **kind** of device, not per brand or model: it says what the device is, and the name on the card says whose it is.

| Id | Drawing |
| --- | --- |
| `earbuds` | A pair of in-ear buds, one in front of the other, each a round body with its ear tip (no stems, no brand silhouette) |
| `headphones` | Over-ear headphones (also headsets) |
| `speaker` | A speaker: tweeter and woofer |
| `gamepad` | A controller: D-pad and two face buttons |
| `mouse` | A mouse from above |
| `keyboard` | Two rows of keys and a space bar |
| `pen` | A stylus |
| `watch` | A round watch on its strap, with a crown |
| `phone` | A phone |
| `bluetooth` | The fallback for a device of unknown kind |

`crates/reui-hub/src/modules/devices/art.rs` looks them up (`art::svg(id, small)`). It also accepts the model-family ids the device catalog uses (`airpods`, `airpods-pro`, `airpods-max`, `beats-earbuds`, `beats-headphones`, `galaxy-buds`, `headset`, `xbox-controller`, `dualsense`, `dualshock`, `switch-pro`) and gives each its kind's drawing. Brand silhouettes aren't drawn: at 24 DIP they'd blur into each other, and the kind is what the moment needs.

## Two detail levels

| File | Where | Grid | Stroke |
| --- | --- | --- | --- |
| `<id>.svg` | The open card's leading 56 DIP spot | `viewBox="0 0 48 48"` | 3 |
| `<id>-small.svg` | The compact pill's 24 DIP circle (and other small spots) | `viewBox="0 0 21 21"` | 1.5 |

The island draws these through `Art::Logo`: bare, no tile, filling `opacity.logo-bare` (0.86) of the spot, rounded to whole DIPs. That's 48 DIP in the 56 spot and 21 in the 24 spot, so each drawing's grid is the size it's shown at: one unit is one pixel at 100 % and two at 200 %.

Rules, so the set reads as one family with the Segoe Fluent glyphs the island uses elsewhere:

- **Stroke only**, `stroke="currentColor"`, round caps and joins; small solid dots (`fill="currentColor"`) only for buttons and keys. No colours, gradients or opacity of their own: the island sets the ink.
- **Weight** follows Fluent's (about 1/16 of the glyph): 3 on the 48 grid, 1.5 on the 21 grid (a touch heavier, so the small drawings hold at 21 px).
- **Pixel grid:** straight strokes centred on half units on the 48 grid (`x="13.5"`) and on quarter units on the 21 grid, so their edges land on pixels.
- **Optical size:** tall things (phone, mouse, speaker) about 40 high, wide things (keyboard, gamepad) about 42 wide, everything centred on 24 (10.5 on the small grid); round shapes a little larger than square ones. Two strokes never come closer than one stroke width apart.
- **Corners:** rounded like Fluent's (a phone's radius is about a quarter of its width); no sharp corners except where the object has them (the pen's tip, the Bluetooth rune).
- **Small drawings drop inner detail** (the watch's hands, the pen's band, the second row of keys, the ear tips' seams) rather than shrink it.

## Adding a kind

1. Draw `<id>.svg` on the 48 grid and `<id>-small.svg` on the 21 grid by the rules above, in the same markup as the others (one `<g>` with the stroke attributes, then plain shapes).
2. Add it to `KINDS` in `art.rs` (and any family ids to `FAMILIES`); its tests check every file is present, on its grid, single-colour and small.
3. Add it to the contact sheet (`KINDS` in `crates/reui-island/src/render/sheet.rs`) and look at it beside the others: `cargo test -p reui-island device_art_sheet -- --ignored` writes `target/devices-art/`: each kind in a compact pill and an open card, dark and light, at 100 % and 200 %, and a magnified sheet of every drawing at its exact pixel size beside the Fluent glyph at the same size.

## Device colour

The drawings are outlines in the theme's label ink (near-white on the dark island, black on the light one), so a device's own colour never makes them vanish: black AirPods Max on the black island are still a white outline. The colour therefore doesn't need a second drawing. If it's ever shown, it should be expressed as **ink, not geometry**: the island would draw the same SVG in a colour the module names semantically (for example a tone meaning "the device's finish", resolved by the island from a small set of quiet finishes and never darker than `label.secondary` on dark or lighter than it on light), so contrast stays the island's job. Nothing builds this yet; the research (`docs/research-device-personalization.md` §5.3) leaves it for phase P2 with the Apple colour byte.
