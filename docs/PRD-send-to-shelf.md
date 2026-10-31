# PRD: Send to → ReUI shelf

Status: built · Owner: Rohit · From [research-features.md](research-features.md) §5e #2 · Extends [PRD-file-shelf.md](PRD-file-shelf.md) · Nov 1, 2026

**References:**
- **Explorer's Send to menu** is the folder `shell:sendto` (`FOLDERID_SendTo`, `%APPDATA%\Microsoft\Windows\SendTo`). Any shortcut placed there is an entry; choosing it drops the selected items on the shortcut, which runs its target with the shortcut's arguments followed by every selected path, in one process. No registration, no COM, no admin, and it works unpackaged. On Windows 11 it's in the classic menu (**Show more options**, or Shift+right-click).
- **A COM context-menu handler** (`IExplorerCommand`) would put an entry in Windows 11's first-level menu, but needs package identity (a sparse package) for that menu, an in-process DLL loaded into Explorer, and registration. Too much weight and risk for a convenience.
- **NotchNook, boring.notch:** files reach the shelf only by dragging. Windhawk's Dynamic Island likewise.
- **ReUI today:** the shelf takes drops on the island (`ModuleInput::Drop` → `intake::plan` → one card per drop). Module commands arrive over the pipe from Settings and `reui-bridge` (`ClientMsg::Command`). `reui-bridge` is a console program (agents run it), so a shortcut to it would flash a console window; `reui-hub.exe` has none in release builds and already handles a second launch (single instance → opens Settings).

## 1. Problem

Putting a file on the shelf means dragging it to the top of the screen. With Explorer maximised and several files selected that's a long drag. On a touchpad, or with the island hidden (*When needed*), it's awkward, and a keyboard user can't do it at all.

## 2. Goals

1. **From Explorer's menu.** Select files, Send to → **ReUI shelf**: held, as if dropped.
2. **The same card.** Several selected files are one card, as a drag of them would be. Files under temporary folders are copied first, as for a drop.
3. **You see it worked.** The shelf opens with the new card.
4. **Nothing flashes.** No console window, no Settings window.
5. **Works when ReUI isn't running.** The entry starts ReUI with the files held.
6. **Scripts too:** `reui-bridge shelf <files…>`.
7. **Installed and removed cleanly** by the per-user setup, like the Start menu entry.

## 3. Non-goals

- An entry in Windows 11's first-level context menu (needs package identity, §References).
- Sending text, links or images from other apps (they have no Send to).
- Choosing a card to add to. Each send is a new card.

## 4. Scenarios

| # | Scenario | Result |
|---|---|---|
| S1 | Three files selected in Explorer, Send to → ReUI shelf | one card with three files; the shelf opens on it; the pill counts one more |
| S2 | A folder | a card for the folder, as a dropped folder |
| S3 | A file inside a zip opened in Explorer (`%TEMP%`) | copied into ReUI's shelf folder first, then held |
| S4 | ReUI isn't running | ReUI starts, then holds the files; the card shows in the pill (the island wasn't there to open it) |
| S5 | A script: `reui-bridge shelf build\out.zip` | the full path held; exit 0 |
| S6 | `reui-bridge shelf missing.txt` | nothing held; "none of those files exist"; exit 1 |
| S7 | ReUI is running but its shelf is off (Settings → Shelf) | nothing happens |
| S8 | Uninstall | the Send to entry is gone with the Start menu entry |

## 5. Rules

### 5.1 The entry

- `%APPDATA%\Microsoft\Windows\SendTo\ReUI shelf.lnk` → the installed `reui-hub.exe`, arguments `--shelf`, ReUI's icon, description "Hold on ReUI's shelf".
- Written by the installer on every install and upgrade (so it follows the install folder), removed by uninstall's "remove shortcuts" step. Failing to write it doesn't fail the install.

### 5.2 `reui-hub --shelf <paths…>`

- Everything after `--shelf` is a path, whatever it looks like (a file may be called `--demo`); flags go before it.
- Paths are made absolute (Explorer's already are), quotes trimmed, missing ones dropped, duplicates (compared without case) dropped.
- **Another hub runs:** the paths go to it as the shelf command `add` (below), over the pipe as Settings commands do; this process exits. It keeps trying for 5 s (the hub may still be starting, at sign-in). Nothing left after filtering: it exits without opening Settings.
- **This is the first hub:** it starts as usual and its shelf holds the paths once it has loaded what it held before.

### 5.3 The shelf command

- `add`, then one full path per line (`reui_protocol::shelf_add` / `parse_shelf_add`). Windows file names can't contain a line break, so a line is always one path.
- The shelf holds them as one `Dropped::Files` drop (the same intake as a drag: references, temp copies, the cap), marks the card open and asks the island to bring it forward (`focus`).

### 5.4 `reui-bridge shelf <files…>`

- Same filtering; none left is an error (exit 1). ReUI not running: exit 1, "ReUI isn't running". For scripts and agents; Explorer uses the hub so no console window shows.

## 6. Design

- **Protocol:** `shelf_add`, `parse_shelf_add` (tested).
- **Hub:** `send_to.rs` (split the arguments, keep existing paths: tested; forward over the pipe), `main.rs` (`--shelf` before and after the single-instance check), `FileShelf { sent }`, the shelf's `add` command.
- **Bridge:** `shelf` subcommand (tested).
- **Setup:** `Layout::send_to` and `send_to_shortcut()`, `shortcut::create(…, args, …)` and `send_to_folder()`, install and uninstall; the flow test checks the entry's target and arguments and that uninstall removes it.

## 7. Milestones

- **M1 (this change):** all of the above.

## 8. Tests

- Unit: the command round-trips paths with spaces, `;` and `,`, ignores blank lines and CR; the flag split; existing-only and de-duplicated paths; the bridge's paths.
- Setup flow test: after install the Send to shortcut targets `reui-hub.exe` with `--shelf`; after uninstall it's gone.
- Checked on this machine with a sandboxed hub: `reui-hub --shelf one.txt` as the first instance held it after start; a second `reui-hub --shelf "two words.txt" missing.txt` exited 0 and the running hub held `two words.txt` only; `reui-bridge shelf` held a file (exit 0) and refused a missing one (exit 1).
- By hand, still to do: the real Send to menu on an installed build (one file, several, a folder, a file inside a zip) and with ReUI not running.
