# PRD: Clipboard

Status: v1 built (M0–M4); hands-on checks pending (see section 10) · Owner: Rohit · Feature #4 in [PLAN.md](../PLAN.md) · Oct 27, 2026

Reference: `design/reference/inspiration/clipboard-supaste.png` (Supaste). It shows a search field and three icon buttons (pinned, grid, add) in the top row; category tabs with counts (History 24, Prompts, Colors, Assets, Inspirations, +); and a horizontal row of rich cards that runs off the edge. Each card has a full-bleed face (photo, address text, a site's og image, a colour swatch, an icon) and a footer with the app icon, "5 min ago" and the size ("3.5 MB").

Inputs: a code audit of the current implementation and a competitor and platform study (Supaste, Paste, Maccy, Raycast, Win+V, Ditto, EcoPaste, CopyQ; Microsoft docs). Both are summarised in the appendix.

---

## 1. Problem

Windows' own history (Win+V) holds 25 items, is cleared on reboot, has no search, and shows plain text rows. People copy links, screenshots, code, colours and files all day, and lose them. ReUI already sits at the top of the screen, so it can keep everything you copy and give it back in one keystroke, as rich cards.

## 2. Goals

1. **Never lose a copy.** Everything you copy (text, rich text, images, files, links) is saved faithfully and pastes back exactly as it was copied.
2. **Find it in under 2 seconds.** Open with a hotkey, type a few letters or press a tab, press Enter.
3. **Recognise it at a glance.** Rich cards show the thing itself, not a description of it, plus the app it came from and one fact.
4. **Never leak or trigger anything.** Secrets are never stored. Copying a link never "visits" it in a way that consumes it. Nothing leaves the machine except opt-in link previews.
5. **Cost nothing at idle.** Capture is event-driven, heavy work runs off the hub thread, and the island draws only while visible.

## 3. Non-goals (v1)

- Cloud or cross-device sync.
- OCR search on images (planned for v2: Windows.Media.Ocr is local and free).
- Text expansion and snippets, AI rewrite, and "multi-clip" merging.
- Virtual files (Outlook attachments, zip contents: `FileGroupDescriptorW`).
- Editing an item's content.
- Drag-out from cards (v2).

## 4. Users and core scenarios

| # | Scenario | Today | v1 |
|---|---|---|---|
| S1 | Copy something, see that it was taken | "Copied" toast | Toast showing the thing itself (thumb or glyph) |
| S2 | Paste something copied a few minutes ago | Hotkey, then click one of 4 cards | Hotkey, ←/→ or 1–9, Enter (or click) |
| S3 | Find a link from yesterday | Not possible past 4 items | Hotkey, type "figma", Enter |
| S4 | Paste a screenshot into Slack | Works, but transparency is lost | Pixel-exact PNG with alpha |
| S5 | Paste formatted text from Word into email | Formatting lost | Rich paste by default, Shift+Enter for plain text |
| S6 | Keep a few things around forever (an address, an IBAN, a prompt) | Not possible | Pin |
| S7 | Copy a password from KeePass or Bitwarden | Skipped (flags honoured) | Skipped, with a quiet count |
| S8 | Copy a password-reset link | **Fetched immediately (consumes the token)** | Never fetched |
| S9 | Paste into an elevated window (admin terminal) | Silently does nothing | Item copied, quiet "Press Ctrl+V" hint |

## 5. Experience

### 5.1 Surfaces

1. **Toast (compact pill), on every new copy.** Leading: the image thumbnail, the app icon, or the type glyph. Trailing: nothing, or one quiet fact. The pill growing and showing the thing *is* the confirmation, so there is no word "Copied" in it (*decision D1*). The toast stays for 2.5 s. It never interrupts a higher-priority activity (agent permission, alert): the copy is still saved, and the toast is just skipped.
2. **Panel (expanded, wide), from the hotkey or a click on the toast.** It is the only full surface. Layout follows Supaste and the [no filler text](../DESIGN.md) rule:

```
╭──────────────────────────────────────────────────────────────╮
│  ⌕ search                                          ★   ⏸     │  row 1: search field, Pinned, Pause
│  All  Links  Images  Code  Colors  Files                     │  row 2: type tabs (only those with items)
│ ╭──────╮ ╭──────╮ ╭──────╮ ╭──────╮ ╭───                     │  row 3: cards, scroll sideways
│ │      │ │      │ │      │ │      │ │                        │
│ │ face │ │ face │ │ face │ │ face │ │                        │
│ │◧ 2m  4 lines  ...                                          │  footer: app icon, age, one fact
│ ╰──────╯ ╰──────╯ ╰──────╯ ╰──────╯ ╰───                     │
╰──────────────────────────────────────────────────────────────╯
```

   - **Search** is a real text field (the panel takes keyboard focus; see 5.3). It filters as you type across text, link title, site and URL, file names, and source app name. The empty state is the placeholder only.
   - **Tabs** are type filters computed from the data. A tab shows only when it has at least one item, and its count is the number of matches. *Decision D2:* user-made categories ("Prompts", "Inspirations" in the reference) are v2; v1 has Pinned as the only user collection, behind ★.
   - **Cards:** 4.5 cards visible (the half card says "there's more"), and the row scrolls with the wheel, trackpad, or ←/→. It loads lazily in pages of 20, and the whole history can be reached.
   - The **selected card** gets an accent ring that follows the card radius (concentric). On open, the selection is the newest item.
   - Card faces are as built (image, text, code, colour, link with og image, files). The footer is app icon · age · one fact. A pinned card shows a small ★ in the footer.
   - **Empty history:** a single quiet line, "Nothing copied yet", is the one allowed exception to the filler-text rule, because otherwise the panel would be blank. *Decision D3:* or don't open at all and just nudge the pill.
3. **Minimal (bubble):** the clipboard never takes the bubble. It isn't an ongoing activity.

### 5.2 Card content rules

| Type | Detected by | Face | Fact (footer right) | Paste-back formats |
|---|---|---|---|---|
| Text | `CF_UNICODETEXT` | Wrapped text, whole lines, ellipsis | "N lines" when more than 1, else nothing | Unicode text, plus HTML/RTF if captured |
| Rich text | `HTML Format` / `Rich Text Format` alongside text | Same as text (plain rendering) | "Formatted" | HTML + RTF + text |
| Code | Heuristic (≥2 lines, half or more ending in `; { } )` or indented) | Mono, tabs = 4 spaces | "N lines" | Text |
| Link | Single token starting `http(s)://` | og image top half + title; or globe + title/host | Site name or host | Text (plus `UniformResourceLocatorW`) |
| Colour | `#RGB #RRGGBB #RRGGBBAA rgb() rgba() hsl()`; the whole string must be the colour | Swatch + value | Format ("HEX", "RGB") | The original text |
| Image | `PNG` > `CF_DIBV5` > `CF_DIB` | Cover-fit thumbnail, alpha on a checker pattern | "W × H" | PNG + DIBV5 + DIB |
| Files | `CF_HDROP` | Icon of the first file + name | "N files" / "Missing" | HDROP (always as copy, see E-F2) |

### 5.3 Input model

The panel opened by the hotkey **takes keyboard focus**. Before taking it, ReUI records the foreground window (the target). Every exit path restores focus to the target first. This is the Win+V model, and it is the only way to type in search. The toast and hover-opened cards keep the current `WS_EX_NOACTIVATE` behaviour and never take focus.

| Key | Action |
|---|---|
| Win+Alt+V (configurable) | Open panel; if open, close it |
| Type | Search |
| ← / → (↑/↓ too) | Move selection; the row scrolls to keep it visible |
| Home / End | First / last |
| Tab / Shift+Tab | Next / previous type tab |
| Enter / click | Paste into target |
| Shift+Enter / Shift+click | Paste as plain text |
| Ctrl+Enter | Copy only (don't paste) |
| Alt+1…9 | Paste the Nth visible card |
| Ctrl+P | Pin / unpin |
| Delete | Remove item (a 3 s "Undo" appears in the pill) |
| Space | Large preview (v1.1) |
| Esc | Clear search; if empty, close |
| Focus lost / click outside | Close, without pasting |

The mouse matches this: the wheel scrolls the row, a click pastes, a right-click gives Pin / Copy only / Paste as plain text / Delete as a small menu (*decision D4*: menu now, or later?).

### 5.4 Paste-back

1. Write the item's formats to the clipboard, decoding and encoding *before* `OpenClipboard`. Include `CanIncludeInClipboardHistory = 0` and `ExcludeClipboardContentFromMonitorProcessing`, so neither we nor Win+V record our own write.
2. Close the panel and restore focus to the target. Wait until the target is actually foreground and Shift/Ctrl/Alt/Win are all released (poll `GetAsyncKeyState`, max 500 ms).
3. Pick the chord per target: Shift+Insert for conhost, mintty and PuTTY; Ctrl+V otherwise. The table can be extended in settings. Send it with scan codes.
4. If the target is elevated and we aren't (integrity-level check), skip the injection. The pill shows "Press Ctrl+V".
5. The pasted item moves to the top of history.
6. The previous clipboard is not restored (same as Win+V).

### 5.5 Privacy

- **Never stored:** content with `ExcludeClipboardContentFromMonitorProcessing`, `Clipboard Viewer Ignore`, or `CanIncludeInClipboardHistory` = 0 (or present but unreadable). These are checked *before* reading any content format.
- **Ignored apps:** a settings list (exe names), pre-filled with known password managers (KeePass, KeePassXC, Bitwarden, 1Password, Dashlane, Keeper).
- **Secret detection** (skip, don't store): private key blocks, `AKIA…` AWS keys, JWTs (`eyJ…` with 3 parts), GitHub/Slack/Stripe token prefixes, strings that are 32+ chars of high-entropy base64/hex with no spaces. *Decision D5:* skip entirely, or store but blur until hovered?
- **Pause** (⏸ in the panel and the tray): stops capture until resumed or for 1 h. The pill shows a quiet paused state.
- **Retention:** 30 days by default (setting: 1 day / 7 days / 30 days / forever). Pinned items never expire. **Clear history** in the tray and settings keeps pins. The DB uses `secure_delete = on`.
- **Link previews**, the only network use:
  - **When:** only when the card is actually drawn in the panel, never on copy.
  - **What:** URLs without credentials and without secret-looking query parameters (`token code sig signature key secret password reset magic invite otp auth state session`, `X-Amz-*`, Azure `sv`+`sig`) get the page fetched. Others get **origin only** (`https://host/`: the site name; no title or image, since those describe a different page) or nothing (*decision D6*). Requests bypass system proxies, because a proxy would resolve names itself and defeat the public-address check.
  - **Network rules:** GET only, no cookies, fixed UA, 4 s timeout, 512 KB HTML, 4 MB image, https/http only. DNS is resolved by us, and *every* address must be public: no loopback, RFC1918, link-local (incl. 169.254.169.254), CGNAT 100.64/10, ULA, fe80::/10, or IPv4-mapped. The connection goes to the checked address, and every redirect hop is re-checked.
  - Failures are retried at most once a day. A setting turns previews off.
- **Local only.** The DB is never synced or uploaded. *Decision D7:* encrypt the DB at rest with DPAPI (cost: key handling, slower search)?

## 6. Edge cases and required behaviour

**Capture**

| ID | Case | Required behaviour | Today |
|---|---|---|---|
| E-C1 | Clipboard locked by another app | Retry with backoff ~100 ms, then once more after 250 ms (timer, not sleep); keyed on sequence number | 10 × 15 ms blocking sleep, then lost |
| E-C2 | Burst of updates (apps writing in passes) | Coalesce within 75 ms; read the final state once | Every update read; re-toasts |
| E-C3 | Delayed rendering (Excel ranges, big images) | Read only the formats we keep, with a per-format byte cap; if the owner won't render, skip | Forces all renders |
| E-C4 | Huge image (8K screenshot) | Copy raw bytes, **close the clipboard**, encode on a worker; cap 50 MP | Encodes with the clipboard open, on the message thread |
| E-C5 | Image with transparency | Keep alpha (PNG / DIBV5 alpha) | Flattened to 24 bpp |
| E-C6 | Blank text + picture (Office shapes) | Fall through to the next format | Whole copy dropped |
| E-C7 | Very large text (> 1 M chars) | Store up to 16 MB; beyond that, skip with a toast fact "Too large"; never paste back truncated | Silently truncated |
| E-C8 | Whitespace-only text | Ignore | Ignored |
| E-C9 | Cut files in Explorer | Store paths; paste back as **copy**; card fact "Cut" | Treated as copy (OK) without the fact |
| E-C10 | Files later moved or deleted | Card fact "Missing", dimmed; pasting gives "File no longer exists" | Pastes stale paths |
| E-C11 | No clipboard owner / UWP host exe | Fall back to the foreground app at copy time; generic icon | No source |
| E-C12 | Our own paste-back write | Not re-captured; item moves to top | Not re-captured; doesn't move |
| E-C13 | Listener or hotkey registration fails | The module reports a status; the tray shows "Clipboard unavailable" / "Hotkey in use, change in settings" | Logged only |
| E-C14 | Same content copied again | Single entry moves to the top; the toast still shows (the user did copy) | OK |
| E-C15 | Same text, different rich formats | Same entry; keep the richest formats | n/a |

**Storage**

| ID | Case | Required behaviour | Today |
|---|---|---|---|
| E-S1 | Dedupe across app versions | Stable content hash (BLAKE3) | `DefaultHasher` (unstable) |
| E-S2 | DB locked/busy | `busy_timeout` 2 s; errors logged, module keeps running | Module stops on first error |
| E-S3 | DB corrupt | Rename aside to `clipboard.db.corrupt`, start fresh, toast fact once | Module stops |
| E-S4 | Caps | 1 000 items, 500 MB total (all blobs + text), oldest unpinned first; a single item > cap isn't stored | 200 items; image bytes only; reports the wrong id |
| E-S5 | Schema changes | `user_version` migrations | Ad hoc |
| E-S6 | Blocking I/O | All SQLite on a dedicated DB thread | On the hub's async thread |

**Paste-back**

| ID | Case | Required behaviour | Today |
|---|---|---|---|
| E-P1 | Target is elevated | Copy only; hint "Press Ctrl+V" | Silent no-op |
| E-P2 | Modifiers held (hotkey, Shift-click) | Wait for release; Shift-click means plain text, not Ctrl+Shift+V | Sends extra modifiers |
| E-P3 | Terminals | Shift+Insert for conhost, mintty, PuTTY; Ctrl+V for Windows Terminal | Ctrl+V everywhere |
| E-P4 | Target closed while the panel is open | Copy only | Pastes into whatever is foreground |
| E-P5 | Big image paste | Prepare data before opening the clipboard | 132 MB decode inside OpenClipboard |
| E-P6 | RDP / VM windows | Scan-code input | VK only |

**Links**

| ID | Case | Required behaviour | Today |
|---|---|---|---|
| E-L1 | Password reset / magic link / signed URL | Never fetched (see 5.5) | **Fetched on copy** |
| E-L2 | `user:pass@host` | Never fetched; credentials hidden on the card | Shown and sent |
| E-L3 | Name resolving to a private IP / DNS rebinding | Blocked by resolve-and-pin | Fetched |
| E-L4 | Redirect to a private host | Blocked per hop | Fetched, checked only at the end |
| E-L5 | Non-UTF-8 page, numeric entities | Charset from header/meta; full entity decode | Garbled |
| E-L6 | SVG / huge / truncated og:image | Content-type check; SVG via D2D; no partial decode | Partial decode |
| E-L7 | Offline when copied | Retry when shown later | Never retried |

**Panel and island**

| ID | Case | Required behaviour | Today |
|---|---|---|---|
| E-U1 | Hotkey while an agent alert/decision is showing | Panel opens over it; the alert returns after | Opens the agent card |
| E-U2 | Panel open, user not interacting | Stays until Esc, focus loss, or 60 s with no input | Folds after 8 s, or 450 ms after the pointer leaves |
| E-U3 | Copy while another card is open | Saved silently; no toast; the open card stays | Toast hijacks the card |
| E-U4 | Copy while the panel is open | New card slides in at the start; selection is kept | Republishes |
| E-U5 | Fullscreen app (game, video) | Toast suppressed; the hotkey still opens the panel on that monitor | Hotkey does nothing visible |
| E-U6 | Multiple monitors | Panel opens on the monitor with the foreground window | Island's own monitor |
| E-U7 | Empty history | See D3 | No feedback |
| E-U8 | Search with no results | Row shows one quiet "No matches" line | n/a |

**Rendering**

| ID | Case | Required behaviour | Today |
|---|---|---|---|
| E-R1 | Text like `#ab€c` (non-ASCII after `#`) | Plain text | **Panics, the module dies** |
| E-R2 | Hashtag words (`#bad`, `#add`) | Colour only when the whole string is a colour *and* it was copied alone; acceptable as-is | Swatch |
| E-R3 | Emoji / CJK / RTL | Colour emoji; reading direction from content; no split surrogate pairs | Mono emoji, LTR, may split |
| E-R4 | Long site name vs age in footer | Age is fixed; fact ellipsises | Can overlap |
| E-R5 | Image bytes not yet arrived | Type glyph placeholder | Empty well |

**Performance**

| ID | Case | Required behaviour | Today |
|---|---|---|---|
| E-X1 | Big text over the pipe | Send at most 2 KB of preview text | Up to 1 M chars per snapshot |
| E-X2 | Image cache growth | Hub and island evict images no longer referenced (LRU, 64 MB cap) | Grow forever |
| E-X3 | Search over 1 000 items | < 16 ms per keystroke (SQLite FTS5 + in-memory recent page) | n/a |

## 7. Performance budget

- Idle: 0% CPU, no timers. Capture of a normal copy takes < 5 ms on the clipboard thread (clipboard held < 2 ms), with encoding off-thread.
- Panel opens in under 120 ms from the hotkey to the first frame, and the first 20 cards are drawn from cached thumbnails.
- The memory ceiling for the module is 80 MB including caches.

## 8. Settings (v1)

Hotkey · Retention · Ignored apps · Link previews (off / sites only / full, default *full with secret filter*) · Pause · Clear history. They live in `%APPDATA%\ReUI\settings.json` and appear in the tray until a settings window exists.

## 9. Milestones

| M | Scope | Exit criteria |
|---|---|---|
| **M0 — Hardening** (bugs in today's code) — *done Oct 27, 2026; click-outside dismiss, hotkey over a live agent alert and paste-back after the worker rewrite still need a hands-on check* | E-R1 panic, E-S2 error handling, E-L1–L4 link safety (fetch on show, secret filter, per-hop + DNS checks), E-U1/U2 panel focus/timeouts, E-C4/E-P5 clipboard held open, E-X1 preview size | No module death on any input; no fetch of a secret URL; panel stays until dismissed |
| **M1 — Faithful capture** — *built Oct 28, 2026* | PNG/DIBV5 alpha, HTML/RTF, cut flag, missing files, debounce, retry, BLAKE3, DB thread, caps, `user_version` | Round-trip test: copy → paste back is byte-identical per format |
| **M2 — Panel** — *built Oct 28, 2026* | Focus model, search field, type tabs, scrolling row, keyboard map, selection ring, delete+undo, pins | All of S2, S3 and S6 with the keyboard only |
| **M3 — Paste-back hardening** — *built Oct 28, 2026* | Modifier wait, per-app chord, elevated detection, scan codes, plain-text paste | Works into Notepad, Word, Chrome, Slack, Windows Terminal, an admin terminal (hint), VS Code |
| **M4 — Privacy controls** — *built Oct 28, 2026* | Ignored apps, secret detection, pause, retention, clear | KeePass/Bitwarden/1Password copies never appear in the DB |
| v2 | OCR search, user categories, drag-out, large preview, sync | — |

## 10. Acceptance test plan

Each milestone has a script in `tools/` that saves and restores the user's clipboard and deletes its own test rows (as `tools/claude-states.ps1` does for agents). Unit tests cover classify/colour/URL safety/secret detection/footer facts. The manual matrix covers the paste targets in M3 and copies from Chrome, Edge, Firefox, Word, Excel, Figma, Snipping Tool, Explorer (copy and cut), VS Code, Windows Terminal, KeePassXC and Bitwarden.

## 11. Open decisions

| ID | Question | Proposal |
|---|---|---|
| D1 | Word "Copied" in the toast? | Drop the word, but show a glimpse of what was copied (text start, image size, file name, site). *Revised Oct 28: a glyph alone didn't read as a confirmation.* When something else is in the pill (music, an agent), the copy shows as a copy icon in the bubble for its 2.5 s instead (priority `Brief`). |
| D2 | User categories in v1? | No; Pinned only. Type tabs are automatic |
| D3 | Hotkey with empty history | Panel with one line, "Nothing copied yet" |
| D4 | Right-click menu on cards | Yes, in M2 (4 items) |
| D5 | Detected secrets | Skip entirely |
| D6 | Link previews for secret-looking URLs | Origin only (site name + favicon) |
| D7 | Encrypt DB at rest (DPAPI) | Not in v1; revisit with sync |
| D8 | Hotkey default | Keep Win+Alt+V (free on Win 11), configurable |

## Appendix: sources

- Code audit of `clipboard.rs`, `clipboard_os.rs`, `link.rs`, `render.rs`, `pill.rs`, `order.rs` (Oct 27, 2026); its findings are in the "Today" column above.
- Microsoft: [Clipboard formats](https://learn.microsoft.com/en-us/windows/win32/dataxchg/clipboard-formats), [Clipboard operations](https://learn.microsoft.com/en-us/windows/win32/dataxchg/clipboard-operations), [WM_RENDERFORMAT](https://learn.microsoft.com/en-us/windows/win32/dataxchg/wm-renderformat), [SendInput](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput), [SetForegroundWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setforegroundwindow), [Shell clipboard formats](https://learn.microsoft.com/en-us/windows/win32/shell/clipboard), [Windows Terminal actions](https://learn.microsoft.com/en-us/windows/terminal/customize-settings/actions), [PowerToys Advanced Paste](https://learn.microsoft.com/en-us/windows/powertoys/advanced-paste).
- Password managers: [KeePass 2.44](https://keepass.info/news/n200120_2.44.html), [CopyQ security](https://copyq.readthedocs.io/en/latest/security.html), [CrossPaste #5014](https://github.com/CrossPaste/crosspaste-desktop/issues/5014) (Bitwarden sets only `CanIncludeInClipboardHistory=0`).
- Competitors: [Supaste](https://supaste.com), [Paste](https://pasteapp.io/help/exclude-apps), [Maccy](https://github.com/p0deje/Maccy), [Raycast](https://manual.raycast.com/clipboard-history), [Ditto](https://github.com/sabrogden/Ditto/wiki), [EcoPaste](https://github.com/EcoPasteHub/EcoPaste), [CopyQ](https://hluk.github.io/CopyQ/).
- Link safety: [OWASP SSRF prevention](https://cheatsheetseries.owasp.org/cheatsheets/Server_Side_Request_Forgery_Prevention_Cheat_Sheet.html); magic links consumed by prefetchers ([Supabase](https://github.com/orgs/supabase/discussions/41618), [Better Auth](https://github.com/better-auth/better-auth/discussions/6985)).
