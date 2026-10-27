# PRD: Clipboard window (clipboard v2)

Status: **superseded Oct 28, 2026.** The separate window was built and then rejected: the clipboard stays in the notch. Its feature list (collections, search with OCR, paste-as, recent paste, paste stack) lives on in the notch panel; the window code was deleted.

## Why v2

v1 put clipboard history inside the island. Rohit's verdict: "not up to the expectation". Supaste and Paste treat the clipboard as an app of its own: a dedicated window you summon from wherever you're typing, with rich cards, organisation and many ways to paste.

The island is for glanceable status. Browsing and organising need room. So v2 adds a **Clipboard window** and narrows the island to quick feedback.

**Kept from v1:**
- capture (formats, privacy markers, secrets, ignored apps)
- storage (SQLite, caps, retention)
- link previews
- paste-back (formats, elevation check, terminal chord)
- the toast and bubble confirmation

**Replaced:** the island panel (search, tabs and card row inside the pill).

**References:**
- **Supaste** ([supaste.com](https://supaste.com), [updates](https://supaste.com/updates)):
  - quick-paste window on ⌃⌘V;
  - Library window with List/Card/Board views;
  - notch shelf;
  - custom collections with counts;
  - ⌃⌘0–9 recent paste, sequential paste, multi-clip cards;
  - copy-as transforms, OCR, text expansion, reminders.
- **Paste** ([help](https://pasteapp.io/help/paste-on-mac)): bottom shelf, pinboards, multi-select paste, paste stack, Quick Look, editing.
- **Raycast:** list + preview, a ⌘K actions panel, OCR, paste-as formats.
- **Ditto:** for how Windows does caret placement, focus and paste-back.

## Goals

1. **Summon where you type.** One hotkey opens the window next to the text cursor, already searching.
2. **Find anything in two seconds.** Instant search across text, links, file names, app names and **text inside screenshots**; type and app filters; collections.
3. **Paste exactly what you mean:** as is, plain text, transformed (UPPER, lower, Title, trimmed, JSON or Markdown cleanup), several at once, or one after another.
4. **Organise:** collections you name (Prompts, Colors, Assets, Snippets…), pins, drag to file.
5. **Premium feel** at Supaste's level, while the island stays native and costs nothing at idle.

## Non-goals (v2)

- Sync across devices and sharing.
- AI rewriting inside the window. A "send to agent" hand-off to Claude/Codex is v3.
- Video clips.
- Editing images.

## 1. Surfaces

| Surface | What | Opens |
|---|---|---|
| **Island** (kept) | Copy confirmation: a glimpse in the pill when alone, the copy icon in the bubble otherwise; "Press Ctrl+V" hint | Automatically |
| **Quick window** (new) | The everyday surface: search, filters, collections, card strip or grid, preview | Hotkey; clicking the clipboard bubble/pill; tray |
| **Library** (new, M3) | The same window, larger and resizable: List / Cards / Board views, collection management, settings | From the quick window (⤢) or the tray |
| **Recent-paste keys** (new) | Ctrl+Alt+1…9 pastes the Nth most recent clip without opening anything | Hotkey |

## 2. The quick window

```
 ┌──────────────────────────────────────────────────────────────────┐
 │  ⌕  Search                                  Type ▾   App ▾   ⤢  │  search owns the keyboard
 │  History 128   Pinned 6   Prompts 12   Colors 9   Assets 4   +   │  collections with counts
 ├──────────────────────────────────────────────────────────────────┤
 │ ┌───────┐ ┌───────┐ ┌───────┐ ┌───────┐ ┌───────┐ ┌───────┐      │
 │ │ text  │ │ image │ │ link  │ │ ■ #0A │ │ code  │ │ files │  ⟶  │  cards: strip (default) or grid
 │ │       │ │       │ │ og    │ │ 84FF  │ │       │ │       │      │
 │ │1 ◧ 2m │ │2 ◧ 5m │ │3 ◧ 1h │ │4 ◧ 1h │ │5 ◧ 3h │ │6 ◧ 1d │      │  quick key · app icon · age · one fact
 │ └───────┘ └───────┘ └───────┘ └───────┘ └───────┘ └───────┘      │
 ├──────────────────────────────────────────────────────────────────┤
 │  Preview of the selected clip: full text / large image / page    │  grows on Space
 └──────────────────────────────────────────────────────────────────┘
```

- **Placement (*D1*):** next to the text caret, found when the hotkey fires. If no caret is found, at the mouse. Flipped above the caret near the bottom edge, kept inside the monitor's work area, at that monitor's DPI. Caret lookup order: UI Automation `TextPattern2` caret range → MSAA `OBJID_CARET` → `GetGUIThreadInfo().rcCaret` → mouse.
- **Size:** about 760 × 430 at 100 %. It remembers its size, and the resizable Library mode is larger.
- **Look:**
  - The island's black material and radius family: concentric corners, no blur, quiet text.
  - Cards about 140 × 140 with one corner radius.
  - Hover lifts the card a little lighter. The selection is a hairline in the accent plus a lighter fill, not a thick ring.
  - The strip fades out at the right edge instead of clipping.
  - A soft open/close motion (scale 0.96 → 1, 160 ms).
- **Cards** (as v1, larger and richer):
  - A face that is the content itself: image, swatch, link with page image and title, file icon and name, code in mono with light syntax colour.
  - Text cards get hierarchy: the first line in semibold, the rest quieter, plus a small type badge.
  - Footer: quick key (1–9), source app icon, age, and one fact (size, lines, site, "Cut", "Missing").
  - **Multi-clip** cards: several clips merged into one card.
- **Preview** (a strip under the cards, Space toggles a large view): full text with scrolling, the image at full size, a link's page image and URL, the file list, a colour in HEX/RGB/HSL.
- **Empty states:** "Nothing copied yet" / "No matches", one quiet line each.

## 3. Keyboard and mouse

| Key | Action |
|---|---|
| Hotkey (default Win+Alt+V, *D3*) | Open (captures the target field first); again: close |
| Type | Search (IME works; the window has real focus) |
| ←/→ (strip), arrows (grid) | Select |
| Enter / double-click | Paste into the target |
| Shift+Enter | Paste as plain text |
| Ctrl+Enter | Copy only |
| Ctrl+1…9 | Paste the Nth visible card |
| Space | Large preview |
| Ctrl+K | Actions: paste as UPPER / lower / Title / trimmed / single line / JSON pretty / Markdown, edit, rename, add to collection, pin, copy text from image (OCR), delete |
| Shift+←/→, Ctrl+click | Multi-select; Enter pastes them joined (newline) |
| Ctrl+Shift+V (in window) | Start a **paste stack**: the selection pastes one item per Ctrl+V in the target, in order |
| Ctrl+Tab / Ctrl+Shift+Tab | Next / previous collection |
| Ctrl+F | Focus search; Tab moves between search and cards |
| Ctrl+P / Del / Ctrl+Z | Pin / delete / undo |
| Esc | Clear search; again: close and return focus |
| Drag a card | Out to any app (text, image or file drop); onto a collection tab to file it |
| Right-click | Same menu as Ctrl+K |

**Outside the window:** Ctrl+Alt+1…9 pastes the Nth most recent clip directly.

## 4. Paste-back (focus model, *D2*)

1. **On the hotkey,** remember the target: the foreground window, the focused child (`GetGUIThreadInfo.hwndFocus`) and the caret rectangle. Then take focus; the hotkey counts as the last input, so this is allowed.
2. **On paste:**
   - write the clipboard (v1 formats, with our own writes marked);
   - hide the window;
   - restore the target with Ditto's sequence (`AttachThreadInput`, `SetForegroundWindow`, `SetFocus` on the focused child);
   - wait until it's in front;
   - release modifiers;
   - send the chord (v1: Shift+Insert for terminals).
3. **Blocked paste:** if the target is elevated or gone, leave the item copied and show the island hint "Press Ctrl+V".
4. **Clipboard restore:** optionally put back the user's previous clipboard about 300 ms later (setting, off by default).

## 5. Collections and search

- **Collections:**
  - **History** (everything, newest first) and **Pinned** are built in.
  - You can add your own (+), rename, reorder, colour them and delete them (the items stay in History).
  - An item can sit in several collections.
  - Counts show on the tabs.
  - Collections are kept forever (retention never removes collected items).
- **Filters:** a type chip (Text, Links, Images, Code, Colors, Files) and an app chip, combinable with search.
- **Search** covers text, link titles, sites and URLs, file names, app names, and OCR text from images.
  - OCR uses `Windows.Media.Ocr`: local, with no network.
  - It runs in the background when an image is captured, at low priority.
  - It's cached in the database.
- **Ranking:** substring and word-start matches first, then newest. Results come within 16 ms for 1 000 items (SQLite FTS5).

## 6. Rendering technology (*D4*)

The island stays native (Composition + Direct2D, 0 % idle). For the window there are two options:

| | Native D2D (like the island) | **WebView2 on demand (recommended)** |
|---|---|---|
| UI quality and speed to build | Every control is hand-built: text input with IME, scrolling, grid, drag and drop, menus, accessibility | HTML/CSS: rich layout, IME, drag and drop, accessibility and animation for free; fast design iteration |
| Cost | Tiny | ~80–120 MB while open; nothing when closed (created on open, kept warm for 60 s, then destroyed) |
| Open time | < 50 ms | ~150 ms warm, ~400 ms cold (prewarmed after the first hotkey of a session) |
| Fit with PLAN.md | — | PLAN already names "on-demand WebView2 for heavy panels like clipboard history" as the safety valve |

**Recommended:** WebView2 for the window, in the island process, created on open and destroyed when idle. The hub stays the source of truth; the window is another surface over the same IPC, with clipboard data as JSON. No remote content is ever loaded; a local bundle is shipped in the exe.

## 7. Edge cases (new in v2)

| ID | Case | Required behaviour |
|---|---|---|
| W-1 | No caret found (desktop, games, canvas apps) | Open at the mouse; paste still targets the remembered window |
| W-2 | Target closed while the window is open | Paste becomes copy only, with the island hint |
| W-3 | Hotkey pressed in a fullscreen game | Window opens on that monitor; never steals exclusive fullscreen (open as a normal window only if the game isn't exclusive fullscreen; otherwise do nothing) |
| W-4 | Two monitors, different DPI | Window on the caret's monitor at its DPI; move on `WM_DPICHANGED` |
| W-5 | Focus restore refused | Retry with `AttachThreadInput`; else copy only plus the hint |
| W-6 | Paste stack interrupted by a new copy | Stack ends; the island says so (bubble icon) |
| W-7 | Drag out to an app that only takes files | Offer the text as a temporary .txt file and an image as .png |
| W-8 | OCR on a huge screenshot | Downscale to OCR limits; time-box 2 s; skip on battery saver |
| W-9 | WebView2 runtime missing | Fall back to the v1 island panel and say how to install the runtime (Windows 11 has it built in) |
| W-10 | The window is open when the user copies | New card slides in; selection kept |

## 8. Performance budget

- **Closed:** 0 % CPU, no WebView, no timers. OCR runs only right after capture, at idle priority.
- **Open:**
  - first frame within 150 ms warm and 400 ms cold;
  - typing to results within 50 ms;
  - scrolling at the display's refresh rate;
  - memory within 150 MB while open, released within 60 s of closing.

## 9. Milestones

| M | Scope | Exit |
|---|---|---|
| **W0: Shell** (*built*) | Window host (WebView2 on demand), placement at caret or mouse, focus capture and restore, paste-back, Esc/close, island bubble/pill opens it | Opens next to the caret in Notepad, VS Code, Chrome and Word; Enter pastes there |
| **W1: Cards and search** (*built*) | Strip/grid of rich cards, preview, search, type/app filters, keyboard map basics, quick keys 1–9, the v1 island panel removed | Matches the layout; S2/S3 from v1 by keyboard only |
| **W2: Organise** (*built*) | Collections (create/rename/colour/reorder, drag to file), pins, delete/undo, Ctrl+K actions with transforms | Can build "Prompts" and paste from it |
| **W3: Power paste** (*built*) | Multi-select paste, paste stack, Ctrl+Alt+1–9, drag out | All four work into Word, Slack and VS Code |
| **W4: OCR + Library** (*OCR built; Library mode not yet*) | Local OCR search; Library mode (List/Cards/Board, larger, resizable) | Find a screenshot by a word inside it |
| W5 (later) | Text expansion triggers (opt-in, *D6*), reminders, send to agent | — |

## 10. Open decisions

| ID | Question | Proposal |
|---|---|---|
| D1 | Where does the window open? | At the text caret (Win+V-like), else at the mouse. Alternatives: drop down from the island at the top centre (Supaste notch-like), or a full-width bottom shelf (Paste-like) |
| D2 | Take focus (search with IME, normal window) or keep the hook-based no-focus model? | Take focus and restore it on paste (Ditto's proven sequence) |
| D3 | Hotkey | Keep Win+Alt+V; also Ctrl+Alt+1–9 for recent paste |
| D4 | Window rendering | WebView2 on demand (the island stays native) |
| D5 | Default card layout | Horizontal strip (Supaste/Paste feel), grid toggle remembered |
| D6 | Text expansion (`;sig`) | Later (W5), opt-in: it means watching every keystroke |
| D7 | What happens to the v1 island panel? | Removed; clicking the clipboard pill/bubble opens the window |
