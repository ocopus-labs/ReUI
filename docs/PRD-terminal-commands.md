# PRD: Long terminal commands

Status: built · Owner: Rohit · From [research-features.md](research-features.md) §5d #2 · Nov 1, 2026

**References:**
- **The ask** (r/SideProject, Oct 31, 2026): "you start a build or test run, switch away, and don't know it's done or that it failed."
- **Prior art:**
  - fish and zsh's `notify-done` / `undistract-me` (Linux) pop a desktop notification when a command that ran longer than a threshold finishes in a terminal that isn't focused.
  - iTerm2's "Alert on next mark", and Warp's notifications for long commands, are terminal-specific.
  - Windows Terminal flashes its taskbar button on a BEL (`\a`) when unfocused, and only if the command rings it.
- **Shell hooks:**
  - **PowerShell:** the `prompt` function runs before every prompt. `Get-History -Count 1` has the last command's text, `StartExecutionTime` and `EndExecutionTime`. `$?` (success) and `$LASTEXITCODE` (a native program's exit code) are as the command left them when `prompt` starts.
  - **Bash:** no preexec hook. A `DEBUG` trap runs before each simple command, and `PROMPT_COMMAND` runs before each prompt; `$?` at the start of `PROMPT_COMMAND` is the command's status.
- **ReUI today:**
  - `reui-bridge activity` lets a script report itself, but someone has to write the script.
  - The bridge sends its parent process chain, and the agent module's `terminal.rs` turns it into the window to bring forward ("Open terminal").
  - Settings → Coding agents connects agents by running `reui-bridge install`.

## 1. Problem

A build, a test run, an `npm install`: you start it, switch to the browser or the editor, and either check back too early, or forget and find out much later that it finished (or failed) minutes ago. Nothing tells you unless the command was written to.

## 2. Goals

1. **Automatic, once connected.** Every command that ran **10 s or more** reports when it finishes. Nothing to add to each command.
2. **Only when you're elsewhere.** No card for a terminal that's in front: you watched it finish.
3. **One glance:** done or failed, the command, its exit code, how long it took, and **Open terminal** to go back.
4. **Free while you type.** A quick command doesn't start a process; the hook does a subtraction and a comparison.
5. **Opt-in and reversible:** Settings → Coding agents → Terminals → Connect / Disconnect. The installer writes a marked block at the end of the shell's startup file and takes exactly that block out again. Uninstalling ReUI removes it too.
6. **Fails quiet:** with ReUI not running, or not installed any more, the prompt behaves as before. Nothing is printed and the exit code isn't touched.

## 3. Non-goals (v1)

- zsh, fish, Nushell, cmd.exe. cmd.exe has no prompt hook.
- WSL shells. Their bash can run Windows programs, but the parent chain (needed for "in front" and Open terminal) stops at the WSL boundary.
- A configurable threshold. It's 10 s, written into the hook; anyone can edit the number in their own profile.
- The tab inside Windows Terminal. Open terminal reaches the window (agent moments §6).
- The command's output.

## 4. Scenarios

| # | Scenario | Result |
|---|---|---|
| S1 | `cargo build --release` in Windows Terminal (pwsh), you switch to the browser; it fails after 2 min | the pill: ✕ Failed. Open: eyebrow "PowerShell", hero `cargo build --release`, "Exit 101 · 2m 14s", **Open terminal** |
| S2 | `npm test` in Git Bash takes 40 s; you wait in that window | nothing |
| S3 | `npm install` in VS Code's terminal, you're in the editor (same window) | nothing: VS Code is the window in front |
| S4 | A command of 3 s | nothing, and no process started |
| S5 | Two terminals, a command finishes in each | two cards, one per terminal |
| S6 | Same terminal, another long command finishes | its card replaces the first |
| S7 | Open terminal | the terminal's window comes forward; the card goes |
| S8 | Away for an hour | a finished card goes after 5 min, a failed one after 30 |
| S9 | ReUI isn't running | the prompt is unchanged; nothing printed |
| S10 | Oh My Posh or Starship | ReUI wraps the prompt that exists when its block runs: their line must come before ReUI's block (the guide says so) |
| S11 | Disconnect | the block is removed, and the file is otherwise byte for byte as it was; an otherwise empty profile ReUI created is deleted |

## 5. Rules

### 5.1 The hooks

- **Marker lines:** `# >>> ReUI: long commands >>>` … `# <<< ReUI: long commands <<<`, at the end of the file. Installing again replaces the block, so it follows the bridge's path. Uninstalling removes it and the blank line before it. A begin marker without an end is left alone.
- **PowerShell 7:** `Documents\PowerShell\profile.ps1`, which covers all hosts (Windows Terminal, VS Code), created if missing. `Documents` is the known folder, often in OneDrive.
- **Windows PowerShell 5.1:** `Documents\WindowsPowerShell\profile.ps1`, only if it exists already. Windows clients default to the *Restricted* execution policy, so creating a profile would print an error in every new window.
- **Bash:** `~/.bashrc` (Git Bash, MSYS2).
- **What the hook does after each command:**
  1. If it ran ≥ 10 s, run `reui-bridge finished --exit N --seconds S --shell powershell|bash`, with the command line in `REUI_COMMAND` (Windows PowerShell mangles quotes in native arguments).
  2. Put `$?` and `$LASTEXITCODE` back as they were.
- **PowerShell:** the command is the history line. **Bash:** the first simple command of the line (`BASH_COMMAND`, so `make && make test` reads `make`).

### 5.2 The bridge

- `finished` prints nothing and always exits 0. It sends `finished:{command, exit, seconds, shell, reui_parents, reui_console}` to the hub's `commands` module.

### 5.3 The card

- **Shown** unless the terminal found from the parent chain is the foreground window.
- **Key** per terminal: the shell's process (the bridge's nearest ancestor). A newer report replaces the card.
- **Priority:** Alert for both outcomes. A Brief would be gone before you looked, and the point is to find it when you come back.
- **Pill:** ✓ Done (positive) or ✕ Failed (negative). **Card:** the terminal glyph, eyebrow the shell, hero the command's first line as code (secrets masked), detail "Done · 2m 14s" or "Exit 101 · 2m 14s", **Open terminal** when the chain was sent.
- **Goes:** on Open terminal, after 5 min (done) or 30 min (failed), or when a report from that terminal arrives while it's in front.

## 6. Design

- **Bridge:**
  - `shells.rs`: the blocks, install and uninstall, pure helpers tested.
  - `finished.rs`: the arguments read into the hub command, pure, tested.
  - `main.rs`: routing. `install|uninstall powershell|bash` goes to `shells` instead of the agents' installer.
- **Hub:**
  - `modules/commands/`: `finished.rs` (the report, its key, its card; pure, tested) and `mod.rs` (the loop: in front or not, one timer for the next card to go, Open terminal).
  - `agent/terminal.rs` is now shared (`pub(crate)`) and gains `in_front`.
  - `--no-commands`.
- **Settings:**
  - `model/shells.rs`: connected or not, by the same marker; the paths.
  - Coding agents → **Terminals**: PowerShell and Git Bash, each with Connect / Disconnect, run through the bridge as agents are.
- **Setup:** uninstall finds hooked shells (`hooked_shells`, with the Documents known folder) and runs `reui-bridge uninstall powershell|bash` while the bridge still exists.
- **Private** (docs/PRD-share-privacy.md): the module declares itself private, since a command line can hold a host or a token, so its cards stay out of screen shares.

## 7. Milestones

- **M1 (this change):** PowerShell (7 and 5.1) and Bash, the card, Settings, uninstall.
- **M2, if asked:** zsh (`preexec` / `precmd`), a threshold setting (Settings rewrites the block), the whole command line in Bash (from history), quieting from the card (`commands.json`).

## 8. Tests

- **Unit:**
  - The block goes at the end, installing twice keeps one, removing restores the file exactly (LF and CRLF), and half a block is left alone.
  - Paths are quoted for each shell.
  - `finished`'s arguments, with the command from the variable or after `--`, and missing parts mean nothing is sent.
  - The report parses, has its key and reads as Done or Failed.
  - Settings sees a connected shell by its marker.
  - Setup finds hooked shells.
- **Checked on this machine** (sandboxed hub, debug log):
  - Bash (Git Bash 5.2, interactive, `--rcfile`): `sleep 11 && false` was reported (exit 1, 11 s), the next command still saw `$?` = 1, and a 1 s command sent nothing.
  - pwsh 7 and Windows PowerShell 5.1: after `Start-Sleep 11; cmd /c exit 7` the wrapped `prompt` reported exit 7 and 11 s, and `$LASTEXITCODE` stayed 7.
  - From the terminal in front, reports are dropped ("its terminal is in front"). With Notepad in front, the pill showed ✕ Failed and the card read "PowerShell · `cargo build --release` · Exit 101 · 2m 14s · Open terminal".
  - Install then uninstall left `.bashrc` exactly as it was.
- **By hand, still to do:** Windows Terminal and VS Code with the real profile loaded, Open terminal from the card, a prompt theme (Oh My Posh) above the block.
