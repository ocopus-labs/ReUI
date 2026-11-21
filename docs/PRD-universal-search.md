# PRD: Universal search

Status: draft · Owner: Rohit · From [research-features.md](research-features.md) §5f #1 · Nov 3, 2026

**References:**
- **The ask** (a reply to the 0.2.0 post): "implement this quick search tool like in this repo [Heimdall] … just copy the code, credit it and change up the appearance. Or use this one: [Takeoff], which I think is much more lightweight."
- **[Heimdall](https://github.com/Kryzern/Heimdall)** (MIT, Ethan Dsouza / Kryzern; Rust, ~9.5k lines, under 2 MB):
  - Alt+Space, a glass window, apps, files, Windows settings and Control Panel, a calculator, open windows, web searches, notes, usage history and pins.
  - **Apps** come from `shell:AppsFolder`, the Start menu's own list, so classic and Store apps arrive together (`apps.rs`, 154 lines).
  - **Names** are scored by prefix, word boundary and whole-name match after a fast substring prefilter (`heimdall-core/search.rs`).
  - **A small calculator** (`calc.rs`, 236 lines) and **"frecency" history** with pins (`history.rs`).
  - **Files:** it reads the NTFS master file table and tails the USN journal, from a Windows service running as SYSTEM, because a volume handle needs admin (`ntfs.rs`, `heimdall-indexer`).
- **[Takeoff](https://github.com/akiraeng/takeoff-launcher)** (MIT, Akira; C++, ~16k lines, under 500 KB, ~10 MB of memory):
  - Alt+Space; apps from the Start menu folders and the desktop, a few user folders walked for files.
  - **Acronym and prefix matching:** `vsc` → Visual Studio Code, `dev man` → Device Manager.
  - System tools, power commands, a calculator, web search keywords, Alt+1…8 quick launch, Ctrl+Enter as administrator, Ctrl+K actions.
  - Nothing written to disk.
- **ReUI today:**
  - The island already has a panel that takes the keyboard without taking focus (clipboard, Win+Alt+V): a search field, Tab between filters, Enter acts, Esc goes back to where you were typing.
  - Shortcuts are registered by the hub (`modules/hotkey.rs`); Settings records them.
  - The clipboard module reads app icons and file thumbnails on the blocking pool (`clipboard/os/image.rs`).
- **PowerToys Run** uses the Windows Search index for files: no admin, nothing to index itself.

## 1. Problem

Opening an app, a file or a setting means the Start menu (slow, web results mixed in) or a separate launcher, which is one more program to install and run. ReUI is already at the top of the screen, already has a keyboard-first panel, and is already running.

## 2. Goals

1. **Shortcut, a few letters, Enter.** The island opens on a search field and the first result is right most of the time: `vsc` → Visual Studio Code, `dev man` → Device Manager, `bluetooth` → Bluetooth settings.
2. **Apps, files, settings and quick answers in one list:** installed apps (desktop and Store), files and folders, Windows settings and Control Panel pages, a calculator, and power commands.
3. **Fast:** results as you type, under 50 ms for apps and settings; files may follow a moment later without the list jumping.
4. **Nothing at rest:** no indexer of our own and no polling. The app list is cached and refreshed when the Start menu folders change; files are asked of Windows' own index only while you type.
5. **No admin, ever.** Per-user, like the rest of ReUI.
6. **Credit where it's due:** Heimdall's and Takeoff's MIT notices, wherever their code or ideas are used.

## 3. Non-goals

- An index of the whole disk of our own (Heimdall's NTFS reader needs a SYSTEM service).
- Plugins, scripting, themes of its own.
- Web results by default. Maybe later as keywords (`g …`), off until turned on.
- Replacing the clipboard search, which stays in the clipboard panel.

## 4. Experience

### 4.1 Opening and closing

- **Alt+Space** opens the island on the search panel (*D1*), with the keyboard on it and focus left where it was, as the clipboard does. Pressing it again, or Esc on an empty field, closes it, and you're back where you were typing.
- It also opens from the row of places (a search glyph), and from the tray.
- Pressed with a card waiting (a permission request), search still opens; the request waits, as with the other shortcuts.

### 4.2 The panel

```
 ╭────────────────────────────────────────────────────────────────╮
 │  ⌕  vsc                                                          │  the field
 │  ▣  Visual Studio Code                                App      │  selected: a lighter row
 │  ▣  vscode-settings.json         Documents\dotfiles   File     │
 │  ⚙  Visual effects               Settings                      │
 ╰────────────────────────────────────────────────────────────────╯
```

- Up to 8 rows: the icon (the app's, the file type's, a settings glyph), the name with the typed letters in bold, and quietly where it is. Kind words like "App" or "File" only when two rows would otherwise look alike (*D3*).
- **Empty field:** your most used and pinned results (*D4*), not a blank panel.
- **Typed maths** (`125*8`, `sqrt(144)`): the answer as the first row; Enter copies it.
- **Power commands** (lock, sleep, restart, shut down, sign out) only when typed by name; restart and shut down ask once, as Restart now does.
- **Nothing found:** one quiet line, "No results", and a row to search the web only if web keywords are on.

### 4.3 Keys

| Key | Does |
|---|---|
| Type | Searches |
| ↑ / ↓ | Moves the selection |
| Enter | Opens it |
| Ctrl+Enter | Opens it as administrator (apps) |
| Ctrl+Shift+Enter | Shows a file in its folder |
| Alt+1 … Alt+8 | Opens the nth result |
| Ctrl+P | Pins or unpins it (it then shows on an empty field) |
| Ctrl+C | Copies its path (files), or the answer (maths) |
| Esc | Clears the field, then closes |

Right-click on a row: the same actions as a menu.

### 4.4 Ranking

1. **Pinned** results, then
2. **Name score**: whole name > prefix > word start > acronym (`vsc`) > anywhere inside, Heimdall's and Takeoff's rules combined, then
3. **Frecency**: how often and how recently you opened it, and which result you picked last time for this query (Heimdall's `history.rs`), kept in `%APPDATA%\ReUI\search-history.json`, never sent anywhere. Clearable in Settings.

Apps outrank files with the same score; files from Windows' index come after apps and settings, so a slow file answer never pushes the right app down.

### 4.5 Settings

Settings → **Search**:
- on or off;
- the shortcut;
- which kinds show (apps, files, settings, calculator, power);
- folders to leave out;
- clear history.

## 5. Design

- **Module** `modules/search/`, one job per file:
  - `apps.rs`: the installed apps, from `shell:AppsFolder` on the blocking pool (from Heimdall `apps.rs`), cached, and read again when the Start menu folders change (`ReadDirectoryChangesW` on the user's and all users' Start Menu, the same thread pattern as `modules/folders/`).
  - `settings_pages.rs`: Windows settings (`ms-settings:` URIs) and Control Panel applets, a fixed list with search words.
  - `files.rs`: Windows Search on the blocking pool (the `SystemIndex` catalogue through its OLE DB query helper), names matching the query, at most 20, cancelled when the query changes. Off, or no index: no file rows, nothing breaks.
  - `score.rs`: name scoring and the acronym rule (from Heimdall's scorer, Takeoff's acronym idea); pure, tested.
  - `calc.rs`: the calculator (from Heimdall), pure, tested.
  - `history.rs`: frecency and pins (from Heimdall), pure, tested.
  - `launch.rs`: opening (`ShellExecuteEx`; `runas` for administrator; `AppsFolder` IDs for Store apps), show in folder, power commands.
  - `view.rs` and `mod.rs`.
- **Protocol:** one new row, `Row::Results(Vec<Result>)`, where each `Result` has an icon (`ImageId`), a title, the matched ranges, a detail, an action, and its menu actions. The search field is today's `Row::Search`. The island draws the list in `render/rows/results.rs` (layout in `reui-design/layout/results.rs`). Meaning only, as everywhere: no sizes or colours in the protocol.
- **Icons:** apps' and files' icons through the existing icon helpers, on the blocking pool, cached by path.
- **Credit:**
  - `THIRD_PARTY_NOTICES.md` with Heimdall's and Takeoff's MIT notices.
  - A line at the top of each file built from theirs ("Adapted from Heimdall's `apps.rs`, MIT, © 2026 Ethan Dsouza").
  - The README's acknowledgements and the guide's Search section.
- **Flag:** `--no-search`. **Files on disk:** `search.json` (settings), `search-history.json`.

## 6. Milestones

| | What | Done when |
|---|---|---|
| **M1** | The panel, apps and settings pages, the scorer, Enter / Ctrl+Enter / Alt+1…8, Alt+Space; notices and credit | `vsc`, `dev man`, `bluetooth` find the right first row; apps open; Store apps too; 0 CPU at rest; scorer tests |
| **M2** | Calculator, power commands (asking once for restart and shut down), history and pins, right-click menu | Maths copies; the empty field shows your most used; history survives a restart |
| **M3** | Files through Windows Search, show in folder, folders left out; Settings → Search | Files from Documents and Downloads appear under 150 ms after typing stops; no index means no file rows and no errors |

Each milestone is its own PR; `docs/guide.md`, the README feature list, `ARCHITECTURE.md`, `docs/development.md` and `THIRD_PARTY_NOTICES.md` change with M1.

## 7. Decisions

- **D1: Alt+Space by default?** Both references and PowerToys Run use it. It replaces Windows' own window menu shortcut (that menu still opens with a right-click on the title bar), so it's changeable in Settings. The alternative is Win+Alt+Space, which collides with nothing but is harder to press. Proposal: Alt+Space.
- **D2: files through Windows Search, or our own folder walk (Takeoff)?** Windows' index is free and covers what the user indexes; a walk of Desktop, Documents and Downloads is predictable but costs a scan at start and a watcher. Proposal: Windows Search; revisit if it proves unreliable.
- **D3: kind words ("App", "File")?** Our no-filler rule says one fact per row, so the row shows where it is ("Documents\dotfiles"); a kind word only when the icon can't tell two rows apart.
- **D4: an empty field shows the most used?** Yes; it's what makes the panel useful before typing. The alternative is a blank field.
- **D5: copy or port?** Heimdall's Rust (apps, scorer, calculator, history) is adapted into ReUI's modules and conventions, not vendored as a crate: its UI, settings and indexer stay out. Takeoff contributes ideas (acronym rule, power-command aliases), not code, since it's C++.
