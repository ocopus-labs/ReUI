# PRD: File shelf (hold things on the island, drag them out anywhere)

Status: M0–M1 built · Owner: Rohit · "Downloads and file shelf" in [PLAN.md](../PLAN.md) · Oct 30, 2026

**References:**
- **NotchNook "Tray"** (lo.cafe): drag a file toward the notch and it waits there across apps, desktops and restarts; drag it out into any app, or AirDrop it. Reviews: one file out at a time, and dragging out *moved* the original ([Macworld](https://www.macworld.com/article/2406934/notfhnook-macbook-dynamic-island-widgets-files-tray.html), [MacStories](https://www.macstories.net/reviews/notchnook-and-mediamate-two-apps-to-add-a-dynamic-island-to-the-mac/)).
- **boring.notch "Shelf"**: files, URLs, images and text dropped onto the notch, kept by security-scoped bookmarks (references, not copies) across launches; the notch opens when a drag from outside enters its region ([DeepWiki: Shelf System](https://deepwiki.com/TheBoredTeam/boring.notch/3.6-shelf-system)). Asked for next: paste into the shelf ([#1716](https://github.com/TheBoredTeam/boring.notch/issues/1716)).
- **Windhawk "Dynamic Island for Windows"** (devcode90) **File Tray**: "drag files onto the island to park them, click a row to open it. Files are only referenced, never copied or moved" ([1.3.0](https://github.com/ramensoftware/windhawk-mods/pull/5653)); persistence came in 1.4.0. Its review found the drag *out* allowed `DROPEFFECT_MOVE`, so dropping on a folder on the same drive moved the user's original, and that its store stayed on disk after removal ([#6312](https://github.com/ramensoftware/windhawk-mods/pull/6312), [#6446](https://github.com/ramensoftware/windhawk-mods/pull/6446)). Lesson: never offer Move, either way.
- **Windows OLE drag and drop**: `DoDragDrop` finds "the window under the current cursor location" and calls that window's registered `IDropTarget` ([DoDragDrop](https://learn.microsoft.com/en-us/windows/win32/api/ole2/nf-ole2-dodragdrop), [RegisterDragDrop](https://learn.microsoft.com/en-us/windows/win32/api/ole2/nf-ole2-registerdragdrop)): a `WS_EX_TRANSPARENT` layered window is skipped by that hit test, exactly as clicks skip it. Elevation: Explorer (medium integrity) can't drop into an elevated window and an elevated app can't drop into ReUI; both fail silently, and `ChangeWindowMessageFilterEx` doesn't fix OLE drops ([UIPI and drag and drop](https://learn.microsoft.com/pl-pl/archive/blogs/patricka/q-why-doesnt-drag-and-drop-work-when-my-application-is-running-elevated-a-mandatory-integrity-control-and-uipi), [photocraft #8559](https://github.com/storytold/photocraft/pull/8559)).

**Inputs (what ReUI already has):**
- The clipboard's card row (`Row::Items`) and its faces (`Preview::Files`, `Image`, `Link`, `Text`), with a card menu, keyboard and a hover; the island draws them for any module.
- The view switcher's places (`HubMsg::Destinations`) and the idle pill's row of places: one table in `modules/destinations.rs`.
- The island is `WS_EX_LAYERED | WS_EX_NOREDIRECTIONBITMAP | WS_EX_TRANSPARENT`, and drops `WS_EX_TRANSPARENT` only while the pointer is on its shapes (`set_clickthrough`). A low-level mouse hook sees every move, and `drag_guard.rs` already knows when a button that went down elsewhere is held (a drag passing over).

## 1. Problem

Moving a file from one place to another on Windows means two windows side by side, or a trip through a temporary folder. Attaching three files from three folders to an email means three trips through the file picker. Images and text in a browser have no place to wait at all, short of saving them somewhere and remembering where.

## 2. Goals

1. **Park it in one gesture.** Drag anything (files, folders, an image, a link, selected text) to the top of the screen; the island grows into a target; let go.
2. **Take it anywhere.** Drag it back out of the island into any app that takes drops: a mail, a chat, an upload page, an Explorer folder. The original stays where it was.
3. **Nothing is lost or moved.** Files are referenced, never copied or moved. Only things that have no file (a browser image, dropped text) get one, in ReUI's own folder, with a size cap.
4. **Still ReUI.** Click-through everywhere except the island and, while a drag passes near, the target. Never steals focus. Nothing runs at rest: no polling, no watching folders.

## 3. Non-goals (v1)

- Finished-download alerts (PLAN.md's other half: a folder watcher; its own milestone, M3).
- AirDrop-like sending, cloud sync, sharing, any network.
- Pasting into the shelf (M2), selecting several cards to drag together (M2; the pill drags everything, §5.4).
- Moving files with the shelf. Copy and link only, both ways (Windhawk's lesson).

## 4. Scenarios

| # | Scenario | v1 |
|---|---|---|
| S1 | Park, then attach | Drag `Report.pdf` from Explorer toward the island: near it, the island grows into the drop target ("Drop to hold"). Let go: the pill shows the shelf's mark and `1`. Later, in the mail, drag the pill (or its card) onto the message: attached. `Report.pdf` never left its folder |
| S2 | Collect from several folders | Three drops from three folders: three cards, the pill says `3`. Drag the **pill** into the upload dialog: all three files at once. Or open the shelf and drag one card |
| S3 | A browser image | Drag an image from a page to the island. It has no file, so ReUI saves it (`%LOCALAPPDATA%\ReUI\shelf\`) and holds that; the card shows the picture. Drag it into a chat: it's a PNG file |
| S4 | A link, some text | Drag a link from the address bar or a page: a link card. Select a paragraph and drag it: a text card. Dragged out, they're text (and a URL) for whatever takes them |
| S5 | Quick actions | Open the shelf (its tab in the view switcher or the idle pill's row, or click the pill). Click a card: it opens. Right-click: Open, Show in folder, Copy path, Remove |
| S6 | Clear all | The card's clear button, or Tray → Shelf → Clear shelf |
| S7 | A held file was moved or deleted | Its card says **Missing**; open and drag are off for it; Remove still works |
| S8 | An elevated app | Dropping from an app run as administrator does nothing (Windows forbids it); the guide says so (§7) |

## 5. Experience

### 5.1 Taking a drop

The island must accept a drag only over itself, and stay click-through everywhere else, during the drag too. Windows finds a drop target by hit-testing the window under the pointer, and a `WS_EX_TRANSPARENT` window isn't hit, so:

1. **A drag passes near.** The mouse hook sees a button held that went down elsewhere (`DragGuard`) and the pointer within `island.shelf.reach` (24 DIPs) of the island's shape. Only then does the island drop `WS_EX_TRANSPARENT`, so the next move of the drag finds it. Away from that zone it is click-through again at once, so a drop on a page right under the island's empty area still lands there.
2. **Windows asks (`DragEnter`).** ReUI looks at the formats on offer (never the content yet): files (`CF_HDROP`), virtual files (`FileGroupDescriptorW` + `FileContents`, a browser's image), a URL (`UniformResourceLocatorW`), a PNG, text. Nothing it can hold, or a drag ReUI itself started: no effect, the cursor says no, the island doesn't change.
3. **The target.** Something it can hold: the pill grows into the drop target, the shelf's mark and **Drop to hold** in `label.secondary`, `island.shelf.drop-grow` (24) wider than the compact pill and as tall as a peek, on `motion.spring.expand` (critically damped: it arrives and stops). With Windows' animation effects off, the reduced spring. The shell's drag image keeps following the pointer over it (`IDropTargetHelper`).
4. **The effect.** Copy where the source allows it, else Link. **Never Move**: a source that sees Move may delete its original after the drop.
5. **Let go (`Drop`).** The island reads the data then (one pass, on its own thread's time, a few milliseconds for files) and sends it to the hub; the target folds into the shelf's pill showing the new count. Leaving the zone instead (`DragLeave`) folds it back to what was there.

Over an open card that isn't the shelf (a decision expanded by itself), no target appears: a drag never folds a moment. Over the open shelf card, a drop just adds a card. In a hidden visibility mode the zone is where the island would be, and a drop target reveals it. Paused, hidden with its shortcut, or a fullscreen app in front: no target.

### 5.2 What's held

| Dropped | Held as | Card face |
|---|---|---|
| Files and folders (`CF_HDROP`) | Their paths: **references**. One drop is one card | `Preview::Files` (first name, count); meta "N files" or "Folder" |
| Files under `%TEMP%` (an archive's preview, a browser's temporary copy) | Copied into ReUI's folder first: they're about to vanish | as above |
| A virtual file (a browser's image, a mail attachment) | Saved to ReUI's folder under its own name | an image: `Preview::Image` (a thumbnail) and "W × H"; else `Preview::Files` |
| A PNG with no file | Saved as `Image.png` | `Preview::Image` |
| A URL | The URL | `Preview::Link`; meta the host |
| Text | The text (in the store, not a file) | `Preview::Text` / `Code` (the clipboard's `classify`) |

- **ReUI's folder** is `%LOCALAPPDATA%\ReUI\shelf\` (the store `shelf.json` and the saved files in `files\`). A saved file is deleted when its card is removed. The saved files never exceed `saved_cap_mb` (default 512): over it, the oldest saved cards go first, references are never touched (they cost nothing).
- **One virtual item is at most 12 MB in v1** (one pipe frame is 16 MB). A bigger one isn't held; M2 streams it to the folder from the island instead.

### 5.3 How long things stay

The user decides, in Settings → Shelf → **Keep**: *Until removed* (default), *Until ReUI restarts*, *A day*, *A week*. Expiry is checked when the hub starts and when the shelf changes or opens, never on a timer. "Until removed" survives restarts and reboots (the store is a small JSON file).

### 5.4 Where it shows

- **The pill, while something is held.** A Passive activity: the shelf's mark and the count (`1`, `3`) beside it, as a label sits beside its mark. With something more important in front, the shelf rides in the minimal bubble. Nothing held: no activity at all.
- **A place.** The shelf joins the view switcher and the idle pill's row of places (glyph Package `E7B8`, after Clipboard), opened by its `open` command like the others. Opened empty it reads "Nothing held".
- **The card.** Wide, eyebrow "Shelf", a clear button at its end (Delete `E74D`, plain), then the row of cards, newest first; the clipboard's faces, menu and keyboard (←/→, Enter opens, Delete removes).
- **Drag out.** Press a card and move past the system's drag distance: Windows' drag starts with the shell's own image of the files. Dragging the **compact pill** drags every held file at once (S2). Effects offered: Copy and Link, never Move. A card stays held after it's dragged out.
- **Tray.** Tray → Shelf → Open shelf, Clear shelf.

### 5.5 Actions

| Action | What it does |
|---|---|
| Click / Enter | Open: files with their default app (`ShellExecuteW` "open"), a link in the browser, text is copied |
| Open | Same as the click |
| Show in folder | Explorer with the files selected (`SHOpenFolderAndSelectItems`) |
| Copy path | The paths, one per line (a link: its URL; text: the text) to the clipboard |
| Remove (Delete) | Removes the card; a file ReUI saved goes with it. Originals never |
| Clear | Every card, the same way |

## 6. Protocol (v22)

Semantic only:
- `Destination::drops: bool`: this place takes things dropped on the surface. The island offers a drop target only while some destination says so, and sends the drop to that module.
- `ClientMsg::Drop { module, items: Vec<Dropped> }` with `Dropped::{ Files(Vec<String>), Text(String), Link(String), Data { name, bytes } }`: what was let go on the island, as the surface read it. The hub routes it as `ModuleInput::Drop`.
- `Item::drag: Option<Drag>` with `Drag::{ Files(Vec<String>), Text(String), Link(String) }`: what a card carries when dragged out of the surface. A compact pill drags the files of its card's row.

## 7. Edge cases

| Case | Behaviour |
|---|---|
| A held file is moved, renamed or deleted | "Missing" (checked when the card is drawn, one `GetFileAttributesW` per path); not dragged, not opened; Remove works |
| Network paths (`\\server\share`) | Held like any path; existence is checked only when the card is drawn, so an offline share reads Missing until it's back, and a slow one never blocks the hub (the check runs on a blocking thread) |
| Huge files | References cost nothing whatever the size. Virtual data over 12 MB isn't held (§5.2) |
| Elevated source or target | Windows refuses drops across integrity levels (UIPI) and says nothing. The guide explains it; nothing in ReUI can fix it safely |
| A drag that starts on the island and comes back | Ignored (ReUI's own drag is never dropped on itself) |
| Dragging text in a browser past the island | A text selection drag is a real OLE drag: the target offers to hold it, as it should. A window being moved or a text selection being extended holds the mouse capture: the island becomes hit-testable but gets nothing |
| Settings off (`enabled: false`) | No module, no destination with `drops`, so no target and no tab |
| Explorer restarts mid-drag | The drag ends; nothing held changes |
| ReUI's folder is full (`saved_cap_mb`) | Oldest saved cards go first |

## 8. Privacy

Nothing leaves the PC. Paths, links and text live in `%LOCALAPPDATA%\ReUI\shelf\shelf.json`, saved images and virtual files beside it; Clear (or Settings → Shelf → Clear) deletes them, and the uninstaller removes the folder with the rest of `%LOCALAPPDATA%\ReUI`. Link cards don't fetch previews (unlike the clipboard's).

## 9. Performance budget

- At rest: nothing. No timers, no polling, no folder watchers. The mouse hook already runs; it adds one rectangle test per move while a button held elsewhere is down.
- A drag passing: one window-style change on entering and leaving the zone; the target is the pill's own springs.
- The hub writes `shelf.json` on change only.

## 10. Settings

`%APPDATA%\ReUI\shelf.json`: `enabled` (true), `keep` ("removed" | "restart" | "day" | "week", default "removed"), `saved_cap_mb` (512). Settings → Shelf: the switch, Keep, and Clear the shelf.

## 11. Milestones

| # | What | Done when | Status |
|---|---|---|---|
| M0 | Protocol (v22); hub module: store, what a drop becomes, expiry, cap (pure, tested); the card; actions; destination; settings file | Tests | Done |
| M1 | Island drop target (zone, `IDropTarget`, the target face), drag out of cards and of the pill (`CF_HDROP` via the shell's data object; text and links), Settings → Shelf, tray entry | OLE plumbing tested by calling the drop target with built data objects and reading back built drag data; a sandbox island screenshotted with a seeded shelf; the real drags by hand | Done (§12) |
| M2 | Paste into the shelf; select several cards; stream big virtual files; text out as a `.txt` virtual file (`CFSTR_FILEDESCRIPTOR`/`FILECONTENTS`) for Explorer | | Planned |
| M3 | Finished downloads (folder watcher on Downloads, an Alert with Open / Show / Hold) | | Planned |

## 12. Testing

- Unit: the store (add, remove, expiry, cap, transient paths), what a drop becomes, the card, the drop zone and effect rules, the drag-data object (`CF_HDROP` read back with `DragQueryFileW`, text and URL formats), the drop target called directly with a data object built in the test (`DragEnter` → effect, `Drop` → the items sent).
- By hand (needs real mouse input, which tests never inject): S1–S4 from Explorer, Edge/Chrome and Firefox; drag out into Explorer (copies, never moves), Outlook/Gmail, a chat.
