# PRD: Clipboard panel v3 (the new design)

Status: **built (M1–M3), Nov 1, 2026.** Decisions are in §7.

Reference: [`docs/images/design/clipboard-v3.png`](images/design/clipboard-v3.png) (Rohit's design). As built (M2, drawn by the island offscreen): [`clipboard-v3-built.png`](images/design/clipboard-v3-built.png).

The clipboard stays in the notch (see [PRD-clipboard-window](PRD-clipboard-window.md) for why the separate window went). This PRD changes **how the notch panel looks and is arranged**. It does not change capture, storage, paste-back or privacy ([PRD-clipboard](PRD-clipboard.md)).

## 1. What changes, at a glance

| | Today | v3 |
|---|---|---|
| Header | Full-width search, then a row of tabs (History 24, Pinned 3, collections) | **One toolbar:** a group of type icons, a search pill, a count, Clear, settings |
| Type filters | None in the notch (the window had them) | Icon chips: All · Text · Links · Images · Files · Colors |
| Cards | About 116 × 116, 4.5 across, the footer shows app icon · age · fact | **Landscape, about 1.45 : 1, three across and the edge of a fourth.** Content on top; at the bottom a quiet tag line, then a name and the age |
| Card surface | Flat fill | Flat fill with a **fine grain** |
| Panel width | 560 DIP | 800 DIP (*D4*) |

```
 ╭─(ear)──────────────────────────────────────────────────────────────(ear)─╮
 │ (◷ T 🔗 ▣ ⧉ 🎨)  ( ⌕ Search history…                  )   (4) (🗑 Clear) ⚙ │  toolbar
 │ ╭──────────────╮ ╭──────────────╮ ╭──────────────╮ ╭───────                │
 │ │   ▤▤▤ files  │ │ 🔗 Wikipedia·│ │   ┌──────┐   │ │ A sea of               │
 │ │   fanned     │ │ Sea of clouds│ │   │ img  │   │ │ cloud fills            │  cards: 3 + an edge,
 │ │ 3 files      │ │ en.wiki…     │ │ image        │ │                        │  the row scrolls
 │ │ Itinerary.pdf  just now │ links · en.wikipedia.org 6s │ Summit.png 9s │ text   │
 │ ╰──────────────╯ ╰──────────────╯ ╰──────────────╯ ╰───────                │
 ╰─────────────────────────────────────────────────────────────────────────╯
```

## 2. The toolbar

Left to right, one row, vertically centred, the height of the search field:

1. **Type group:** one capsule holding six icon chips:
   - All (the history clock)
   - Text (T): text and code
   - Links
   - Images
   - Files
   - Colors (palette)

   The chosen chip is a filled light circle with a dark glyph; the others are quiet glyphs. A click filters the cards, and Tab / Shift+Tab move between chips (as tabs do today). Chips show even when they have no clips, so the group never changes shape (*D5*).
2. **Search pill:** the rest of the row. Magnifier, then the placeholder "Search history…". It's the field that already exists: it owns the keyboard once the panel has it, and searches text, titles, sites, URLs, file names and app names.
3. **Count:** a circle with the number of clips that match (*D2*).
4. **Clear:** a capsule (trash glyph + "Clear"). Removes every clip that isn't pinned or in a collection. The pill then offers **Undo** for 5 s, as Delete does. No confirmation dialog.
5. **Settings:** a bare gear glyph. It opens Settings → Clipboard.

**Pinned and collections** have no place in the design. *D3*: the All chip opens a small menu (History, Pinned, then each collection) when there is anything to choose. The chip's glyph becomes that collection's (pin, or the collection's first letter), so you can see you're not looking at everything.

**Pause** stays in the tray and the card's menu. While paused, the count circle turns into a **Resume** capsule (today's rule: the panel only mentions pausing while paused).

## 3. Cards

Size: about 225 × 156 DIP in an 800 panel. That is three across and the edge of a fourth, which says the row scrolls, with a 12 DIP gap and a radius concentric with the panel's. The body is on top. The **footer** is two lines, pinned to the bottom:

- **Tag** (small, semibold, quiet): what it is or how many ("3 files", "text", "image"). The image tag uses the clipboard accent, as in the design. Others are quiet (*D6*).
- **Name** (body, primary, one line, ellipsis) on the left and **age** (small, tertiary) on the right.

| Kind | Body | Tag | Name |
|---|---|---|---|
| Text | Up to 4 lines of the text, body size, primary | `text` | none (the body is the name); the age sits on the tag line |
| Code | Same, monospaced | `code` | none, as text |
| Link | `🔗 Site · Kind` (quiet, semibold), the **title** (semibold), the URL (tertiary, one line) | `links · host` (tag bold, host quiet) | none; the age on the tag line |
| Image | The image **contained** (aspect kept, not cropped), centred, its own small radius | `image` (accent) | file name when known (copied from a file or a page's `<img src>`), else `W × H` |
| Files | Up to three of the files' **thumbnails, fanned** (shell thumbnails: a PDF's first page, an image, else the type's icon; the document glyph until they arrive) | `N files` (or `file`) | the first file's name |
| Color | The swatch fills the card, as today | `color` | the value (`#5E5CE6`) |

What goes: the app icon in the footer (the design has none; the app stays searchable and appears in the card's menu as "From Edge"), the hover pin/delete circles stay as they are, the `★` for pinned stays at the tag line's end.

**Grain:** a 64 × 64 noise tile drawn over the fill of a card holding media (an image, files) as a tiled bitmap brush at 5 % opacity (`card.grain`), at the screen's pixels, the same tile every time. The text and link cards are flat, as in the design. It costs no idle work: it's part of the card's drawing, not an animation.

**Colours, from the reference:** cards, the field, the count and Clear all sit on the quietest well (`fill.quaternary`, about RGB 20 on black, as sampled from the picture); the chosen chip is a primary fill. **Ages** say "just now", "4m ago", "2h ago": minutes at the finest, since the panel isn't redrawn every second (the design's "6s ago" would go stale while it's open). **Size:** 800 DIP wide (`expanded.gallery-width`), cards 156 tall, three across and the edge of a fourth. Settings' Clipboard page shows the panel when its stage is wide enough, else the copy toast (the island is never shrunk to fit).

**Checking it:** `cargo test -p reui-island clipboard_panel_sheet -- --ignored` draws the panel offscreen with the island's own renderer (`target/clipboard-panel/`), so it can be compared with the reference without opening a window.

**Selected card** (keyboard): the hairline ring as today, following the new radius.

## 4. Where it applies

- Both looks (Pill and Notch). The design shows the Notch look with its ears; the toolbar and cards are the same in the Pill look.
- Light and dark themes: chips, pill and cards take the theme's fills; nothing in the design is dark-only except the black material.
- At every scale (100–200 %), and on whichever display the island is on.

## 5. Protocol and modules (meaning, not appearance)

The module says what's there; the island decides how it looks.

- `Tab` gains `glyph: Option<char>`: a tab that is an icon (with its label kept for screen readers and the tooltip).
- `Row::Search` gains `filters: Vec<Tab>` (drawn as the icon group before the field) and `trailing: Vec<Button>` keeps today's buttons.
- `ButtonContent::Count(u32)`: a number in a circle (*D2*).
- `Button.role` `Secondary` with `ButtonContent::Text` + glyph → `ButtonContent::Labelled(char, String)` for "🗑 Clear".
- `Item` gains `tag: Option<String>`, `name: Option<String>` and `tag_tone: Tone` (secondary / accent). `meta` stays: the fact after the tag on a one-line footer ("links · en.wikipedia.org"), and the shelf's cards still use it alone.
- `Preview::Files` gains `thumbs: Vec<ImageId>` (up to 3); `Preview::Link` gains `kind: Option<String>` ("Article", from `og:type`).
- Hub: `clipboard/panel.rs` builds the new toolbar and items; a new `clipboard/thumbs.rs` makes file thumbnails on a worker thread (`IShellItemImageFactory`, COM, never on the hub thread) and caches them by path and modified time.
- Island: `render/rows/search.rs` draws the toolbar; `render/rows/cards.rs` is split into `cards.rs` (row, scrolling, hit-testing) and `card_faces.rs` (one face per kind) to stay under ~500 lines; `render/grain.rs` makes the tile.
- `reui-design`: `layout/cards.rs` takes `visible`/`visible-more` 3/3.3 and lays out a card's face (`Face`: body, tag, name, an image contained); `CardSize::Gallery` / `Card::Gallery`; new tokens `card.grain`, `card.tag-gap`, `card.thumb-*`, `search.filter-*`, `search.toolbar-width`, `expanded.gallery-width` and the `card-tag`, `card-name`, `card-age`, `filter-letter` type styles.

## 6. Milestones

| | What | Done when |
|---|---|---|
| **M1** | Toolbar: type chips, search pill with placeholder, count, Clear + Undo, settings gear, Pinned/collections per *D3* | The panel filters by type from the chips and the keyboard; Clear and Undo work; tests for `to_message` and the layout |
| **M2** | Cards: new size and width, the tag/name/age footer, link, image (contained), text, code and color faces, grain | A screenshot of the real island next to the design reads the same at 100 % and 150 %, dark and light |
| **M3** | Files: fanned thumbnails (worker thread, cache); image names | Copying 3 files in Explorer shows their thumbnails in under 300 ms; idle CPU stays 0 |

**M3 as built:** `clipboard/thumbs.rs` asks the shell (`IShellItemImageFactory`, on the blocking pool) for the first three files of the first 12 cards that hold files, at 192 px, and keeps them by path while the hub runs (256 at most); the panel shows what's ready and is sent again as the rest arrive. The island fans them: the first upright in the middle, the second tilted out left behind it, the third right (`cards::Face::fan`, tokens `card.fan-*`). A copied image's name comes from the `<img src>` in the HTML a browser puts beside it (`os/formats.rs::image_name`: the last path part, `%`-decoded; inline `data:` images and names without an extension say nothing), kept in the clip's text column, so it shows on the card and is searchable; images without one are named by their size.

Each milestone is its own PR. The README picture, `docs/guide.md` and `ARCHITECTURE.md` change with M2.

## 7. Decisions

- **D1 Emoji button: left out.** Windows has Win+. for emoji; the toolbar has no emoji button.
- **D2 The count: kept.** Rohit drew it, and it answers "did my search find anything" at a glance. It's the one count the panel shows, an exception to the no-counts rule.
- **D3 Pinned and collections: a menu on the All chip** (§2), not a seventh chip or a second row.
- **D4 Panel width: 800 DIP.** The design's proportions measured against its toolbar (32 DIP tall): about 790. 800 fits three cards and the edge of a fourth at the design's size, and still fits a 1280 display at 100 %.
- **D5 Empty chips: always show all six,** so the group never changes shape (rather than hiding empty ones as tabs do today).
- **D6 Tag colour: as drawn.** The accent for images only; the other tags quiet. Accent on every tag would be louder.

## 8. Not in v3

An emoji picker, card editing, drag to reorder, a list layout, multi-select, and any change to capture, storage or paste formats.
