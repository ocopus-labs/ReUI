# PRD: Downloads and screenshots

Status: M0–M1 built · Owner: Rohit · Oct 30, 2026

**Ask:** two moments from `docs/research-features.md` §4–5: **downloads in progress** (#3: "Is it done yet, and where did it go?") and the **screenshot moment** (#4: after Win+Shift+S the shot lives only on the clipboard or in a folder).

**Evidence:** downloads ship in Atoll (β), Notchify 5.8, NotchPixel and Halo DynamicWin; exuupery ships them too, and its #26 "doesn't detect Firefox" shows the browser patterns are where it goes wrong. Screenshots: Notchify (snip, recording), exuupery (screenshots, markup asks), devcode90 #145, Atoll's recording.

**Inputs:**
- Windows has no download API. Browsers write a partial file into the download folder and rename it when it's done. Chrome, Edge, Brave and Vivaldi write `<name>.crdownload` (first `Unconfirmed <n>.crdownload` while the name is settled), Opera `.opdownload`, Firefox `<name>.part` beside a 0-byte `<name>` placeholder, older tools `.partial`, qBittorrent `.!qB`, µTorrent `.!ut`.
- **None of them records the total size in the file system.** A partial file is only as long as what has arrived. So ReUI shows the size so far and the speed, and a percentage only when a total is known (no source gives one today; see §7).
- `ReadDirectoryChangesW` reports a growing file while the browser writes (measured: a change event per flushed write, every 250 ms in a test), its rename when it's done, and nothing at all in between. The size comes from the file itself (`GetFileInformationByHandle` through `std::fs::metadata`), which is exact even while the browser holds it open.
- Windows 11's Snipping Tool (Win+Shift+S, and PrtScn by default) puts the shot on the clipboard and, with "Automatically save screenshots" on (the default), also saves a PNG into `Pictures\Screenshots` (`FOLDERID_Screenshots`). Win+PrtScn saves there without the clipboard. The clipboard module already captures the image with its thumbnail and its source app (`SnippingTool.exe` or `ScreenClippingHost.exe`).

## 1. Goals

1. **Downloads:** while a browser downloads into a watched folder, an Ongoing activity says what and how far; when it's done, a Brief **Downloaded** card with **Open** and **Show in folder**.
2. **Screenshots:** a shot gets one Brief thumbnail card with **Open** and **Show in folder** when there's a file, and **Copied** when it's on the clipboard. Never two cards for one shot (the clipboard's own toast gives way).
3. **Idle costs nothing.** A blocked thread waiting on directory events; no polling, no timers while nothing downloads.
4. **Private.** Nothing leaves the PC. A download's contents are never read (only its name and size); a screenshot is read once to make its thumbnail.

## 2. Non-goals (v1)

- Downloads that don't use a partial file (curl, a copy into Downloads), sub-folders, download managers that write elsewhere.
- Pause, resume or cancel: the browser owns the download.
- Screen recordings (`Videos\Screen Recordings`), markup, a second capture tool.
- Drag-out of the finished file: it comes with the file shelf (research §5 #6).

## 3. Settings

| Setting | File, key | Default |
|---|---|---|
| Downloads on/off | `downloads.json` `enabled` (read at start) | On |
| More folders to watch | `downloads.json` `folders` (list of paths) | `[]` (Downloads only) |
| Screenshots on/off | `screenshots.json` `enabled` (read at start) | On |

Settings has one page, **Downloads & screenshots**: a group per feature, each with its switch (not one in the header card, which would read as both), Downloads with its folders (remove; add by path).

## 4. Experience

### 4.1 A download

| When | Island |
|---|---|
| A partial file appears and is still there 1 s later | Ongoing activity. **Compact:** the download glyph, then the size so far and the name ("48 MB · setup.exe"); a ring in place of the glyph when the total is known. **Open:** eyebrow "Downloading", the name as the hero, detail "48.2 MB · 3.1 MB/s" (or "48 of 120 MB · 3.1 MB/s"), a progress row only when the total is known, **Show in folder** |
| It grows | The size and speed update at most once a second, on the browser's own writes. The speed is an average over the last seconds; after 5 s without a write it's left out rather than shown falling |
| Chrome settles the name (`Unconfirmed 123.crdownload` → `setup.exe.crdownload`) | Same activity, new name |
| It's renamed to its final name | The Ongoing activity goes; a Brief **Downloaded** card comes: the file's icon, "Downloaded", the name, its size and folder, **Show in folder** and **Open** (accent). It stays 6 s, longer while open |
| The partial file is deleted and its final name doesn't appear within 1 s | Cancelled: the activity goes, nothing else |
| It hasn't grown for 30 s (paused in Chrome, or left behind by a crash) | It stops showing until it grows again. At start (or after Windows dropped changes) only partial files written in the last 10 s count, so an old abandoned `.crdownload` never shows |
| Two or more at once | One card with a pager ("‹ 1/2 ›"), newest first; compact shows the newest. Finished ones gather in one Downloaded card the same way |

### 4.2 A screenshot

| When | Island |
|---|---|
| Snipping Tool puts an image on the clipboard | A Brief card with the thumbnail: eyebrow "Screenshot", hero "Copied", detail its size ("1920 × 1080"). The clipboard history keeps it as usual; the clipboard's own copy toast doesn't show |
| A PNG or JPEG appears in `Pictures\Screenshots` within 3 s of that | The same card, now with **Show in folder** and **Open** |
| A file appears with no clipboard image (Win+PrtScn, a save from Snipping Tool's window) | The card with the file's thumbnail, hero "Saved", the buttons |
| The card's time is up (6 s, longer while open) | It goes |

Compact: the thumbnail as the round art, then "Copied" or "Screenshot". Bubble beside something else: the thumbnail.

## 5. Engineering

- **`modules/folders/`** (shared OS side, like `audio/`): `watch.rs`, one thread per watcher with an overlapped `ReadDirectoryChangesW` per folder and `WaitForMultipleObjects` (an event to stop); file name, size and last-write changes, renames as one event with both names, a buffer overflow as "rescan". `shell.rs`: known folders (`SHGetKnownFolderPath`), Open (`ShellExecuteW`), Show in folder (`SHOpenFolderAndSelectItems`), each user-asked shell call on a short-lived STA thread. Folders are re-opened when settings change. `notify.rs` (pure) turns a change buffer into changes.
- **`modules/downloads/`:** `partial.rs` (which names are partial, by browser, and their final names; pure), `tracker.rs` (downloads by path: appear, grow, rename, finish, cancel, the 1 s debounce, the speed, the once-a-second size reads; fake clock), `view.rs`, `settings.rs`, `mod.rs` (the loop; sizes and file icons are read off the hub thread).
- **`modules/screenshots/`:** `shots.rs` (a clipboard image and a file within 3 s are one shot; the same file twice is one; pure), `view.rs`, `mod.rs` (the folder watch, thumbnails off the hub thread, the clipboard's channel). The clipboard module asks `screenshots::claim` with the source exe before its toast: when the screenshot module runs and the exe is Snipping Tool's, it hands over the thumbnail and skips its toast.
- **Why two modules:** each has its own switch, file and Settings rows, and either runs without the other. They share the folder watch and the shell calls, not state.
- **Protocol:** no change. Existing rows and elements (`Art::Glyph`, `Art::Image`, `Ring`, `Row::Progress`, `Row::Buttons`, `Pager`) say all of it.
- **Testing:** `REUI_CONFIG_DIR` sandboxes may set `REUI_DOWNLOADS_DIR` and `REUI_SCREENSHOTS_DIR` to watch temp folders instead of the user's own (ignored outside a sandbox).

## 6. Milestones

| # | What | Done when | Status |
|---|---|---|---|
| M0 | Folder watch; partial-file rules; tracker and shot merging, tested | Unit tests; a scripted growing `.crdownload` and `.part` in a sandbox produce the right events | Done: a real-folder watch test; measured one change event per second or so while a file grows (Windows coalesces), each read exact |
| M1 | Both modules, views, settings, the Settings page, docs | A scripted download and a file in a temp Screenshots folder show their cards; Open and Show in folder work | Done in a sandbox: Chrome-style (unconfirmed → named → done), Firefox-style beside it (pager), a cancel, a stall, a saved shot. Open / Show in folder and the Snipping Tool clipboard hand-off are unit-tested up to the shell call, not clicked in a sandbox |
| M2 | Total and percentage from a browser's own record; drag-out to the shelf; recordings | — | Not started |

## 7. Open questions

- **A real total.** Chrome and Edge keep `total_bytes` in their `History` SQLite database, which the browser holds locked while it runs; Firefox in `places.sqlite`. Reading a copy would be a browser-specific, read-only spike. Until then the percentage never shows, which is honest.
- Should a finished download's card be an Alert rather than a Brief when the user has been away (screen locked)?
