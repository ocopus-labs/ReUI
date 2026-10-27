# Clipboard window: message contract

The clipboard window (see [PRD-clipboard-window.md](PRD-clipboard-window.md)) has three parts:
- **Hub** (`crates/reui-hub/src/modules/clipboard*.rs`): the source of truth for data and actions.
- **Host** (`crates/reui-island/src/clipwin.rs`): the WebView2 window inside the island process. It places the window, takes and restores focus, serves the bundle and images, and bridges messages.
- **Web UI** (`crates/reui-island/web/clipboard/`): HTML/CSS/JS, embedded in the island exe.

**Transport:** `HubMsg::Window { surface: "clipboard", message, images }` and `ClientMsg::Window { surface: "clipboard", message }`. `message` is one JSON object with a `"type"`. The host passes hub messages to the page (`window.chrome.webview.postMessage`) and page messages to the hub, except the `host.*` messages and the interceptions noted below.

**Images:** every image id a message refers to is listed in `images` and arrives first as `HubMsg::Image`. The page loads them as `https://reui.local/image/<id>`, served by the host from its image store. The host keeps the images of the latest `open`/`state`/`preview` message alive. In the JSON every image id is a **string** of the decimal `u64` (`"13021587761245113856"`): real ids are 64-bit, and JavaScript numbers would round them.

## Hub → window

```jsonc
// Show the window for this target. The host places it and takes focus, then hands `view` to the page.
{ "type": "open",
  "target": { "hwnd": 1234, "focus": 5678, "caret": [l, t, r, b] /* screen px, or null */ },
  "view": View }

// New data for the open window (after a query, an action, a new copy). Always a full view.
{ "type": "state", "view": View }

// Full content for the preview pane (answer to "preview").
{ "type": "preview", "id": 12, "text": "…up to 200 KB…" /* or null */, "image": 991 /* full-size image id, or null */ }

// Close (the hotkey pressed again, or a paste from outside finished the job).
{ "type": "close" }
```

```jsonc
View = {
  "query": "", "collection": "history",                      // collection id
  "filter": { "type": null /* "text"|"code"|"links"|"images"|"colors"|"files" */, "app": null /* exe */ },
  "layout": "strip",                                         // "strip" | "grid"
  "collections": [ { "id": "history", "name": "History", "count": 128, "color": null, "builtin": true },
                   { "id": "pinned",  "name": "Pinned",  "count": 6,   "color": null, "builtin": true },
                   { "id": "c7", "name": "Prompts", "count": 12, "color": "#0A84FF", "builtin": false } ],
  "types": [ { "id": "text", "label": "Text", "count": 40 } ],   // only types with matches
  "apps":  [ { "id": "chrome.exe", "name": "Chrome", "icon": 77, "count": 30 } ],
  "items": [ Item ],                                         // up to 60, newest first (pinned first in Pinned)
  "total": 128,                                              // matches before the cap
  "paused": false, "undo": false,
  "stack": { "active": false, "remaining": 0 },
  "empty": null                                              // "Nothing copied yet" | "No matches" | null
}

Item = {
  "id": 12, "kind": "text",                                  // text|code|link|image|color|files
  "text": "first 2000 chars", "rich": true,                  // rich: has HTML/RTF
  "title": null, "site": null, "url": null,                  // links
  "image": 991,                                              // thumbnail (images) or page image (links), or null
  "icon": 77, "app": "Chrome",                               // source app icon id / name, or null
  "age": "2m", "meta": "640 × 400",                          // the one footer fact, or null
  "color": null,                                             // "#0A84FF" for colours
  "files": null,                                             // ["a.txt", "b.png"] names
  "pinned": false, "collections": ["c7"],
  "cut": false, "missing": false, "lines": 3, "ocr": false   // ocr: image has recognised text
}
```

## Window → hub

| `type` | Fields | Effect |
|---|---|---|
| `query` | `query`, `collection`, `filter` | Re-query; answers with `state`. The page debounces about 60 ms |
| `layout` | `layout` | Remembered; answers with `state` |
| `paste` | `ids` (1+), `mode` | **Host intercepts:** hides the window and restores the target's focus, then forwards with `"focused": true/false`. The hub pastes (joined by newlines when there are several); if not focused, it copies and shows the island's "Press Ctrl+V" hint |
| `copy` | `ids`, `mode` | Copy only. The host hides the window (focus restored, no keystroke) |
| `stack` | `ids` | **Host intercepts** like paste. The hub puts the first item on the clipboard; each Ctrl+V in the target advances to the next |
| `pin` / `unpin` | `id` | |
| `delete` | `ids` | Undo is available for 5 s (`view.undo`) |
| `undo` | — | |
| `collect` / `uncollect` | `ids`, `collection` | Add to or remove from a collection |
| `collection.create` | `name`, `color?` | |
| `collection.rename` | `id`, `name` | |
| `collection.color` | `id`, `color` | |
| `collection.delete` | `id` | The items stay in History |
| `collection.reorder` | `ids` | |
| `edit` | `id`, `text` | Replaces a text item's content (stored as a new version; the old one is kept under undo) |
| `preview` | `id` | Answers with `preview` |
| `ocr` | `id` | Copies the image's recognised text |
| `pause` / `resume` | — | |
| `export` | `id` | **Host only (drag-out):** the hub answers `{ "type": "export", "id", "text"?, "png"? (image id), "files"? }` |

**`mode`:** `"normal" | "plain" | "upper" | "lower" | "title" | "trim" | "single-line" | "json" | "markdown"`. Everything except `normal` pastes as plain text with the transform applied.

## Page ↔ host only (never reach the hub)

| From page | Effect |
|---|---|
| `host.ready` | The page loaded; the host sends the pending `open.view` |
| `host.close` | Hide (Esc on an empty search, click on ✕). Focus returns to the target |
| `host.resize` with `w`, `h` | Content-driven size in DIPs, clamped to the work area |
| `host.drag` with `id` | Start a native drag-out (the host asks the hub for `export`, then runs DoDragDrop) |

| From host to page | |
|---|---|
| `{ "type": "host.shown" }` / `{ "type": "host.hidden" }` | Lifecycle (focus the search field on shown) |

The host also tells the hub `{ "type": "opened" }` / `{ "type": "closed" }`, so the hub knows the window is up and can answer the hotkey with `close`.
