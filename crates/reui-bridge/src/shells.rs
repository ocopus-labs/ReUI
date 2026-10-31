//! `reui-bridge install|uninstall powershell|bash`: the shell hook that tells ReUI when a
//! long command finishes (docs/PRD-terminal-commands.md).
//!
//! The hook is a block of the shell's own language at the end of its startup file, between
//! two marker lines. It times every command at the prompt and runs `reui-bridge finished`
//! only after one that took [`MIN_SECONDS`] or more, so a quick `ls` never starts a process.
//! Installing again replaces the block (so it follows the bridge's path); uninstalling
//! removes exactly the block and leaves the rest of the file as it was.
//!
//! - **PowerShell 7** (`pwsh`): `Documents\PowerShell\profile.ps1` (all hosts: Windows
//!   Terminal, VS Code). Created when missing: pwsh runs local scripts by default.
//! - **Windows PowerShell 5.1:** `Documents\WindowsPowerShell\profile.ps1`, only when that
//!   file already exists. Its default execution policy on Windows clients refuses
//!   profiles, so creating one would print an error in every new window.
//! - **Bash** (Git Bash, MSYS2): `~/.bashrc`.

use std::path::{Path, PathBuf};

/// Commands shorter than this never reach ReUI: you were still looking.
pub const MIN_SECONDS: u64 = 10;

const BEGIN: &str = "# >>> ReUI: long commands >>>";
const END: &str = "# <<< ReUI: long commands <<<";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shell {
    PowerShell,
    Bash,
}

impl Shell {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "powershell" | "pwsh" => Some(Shell::PowerShell),
            "bash" => Some(Shell::Bash),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Shell::PowerShell => "PowerShell",
            Shell::Bash => "Bash",
        }
    }
}

/// Installs or removes the hook; the message for Settings, or why it couldn't.
pub fn run(shell: Shell, install: bool) -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = exe.to_string_lossy().replace('\\', "/");
    let mut changed = Vec::new();
    for (path, create) in files(shell)? {
        let original = match std::fs::read_to_string(&path) {
            Ok(text) => Some(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(format!("can't read {}: {e}", path.display())),
        };
        let text = match (&original, install) {
            (None, true) if !create => continue,
            (None, false) => continue,
            (Some(text), true) => with_block(text, &block(shell, &exe)),
            (None, true) => with_block("", &block(shell, &exe)),
            (Some(text), false) => without_block(text),
        };
        if original.as_deref() == Some(text.as_str()) {
            continue;
        }
        if !install && text.trim().is_empty() {
            // Nothing but our block was in it (we likely made it): leave no empty file.
            std::fs::remove_file(&path).map_err(|e| format!("can't remove {}: {e}", path.display()))?;
        } else {
            write_atomic(&path, &text)?;
        }
        changed.push(path.display().to_string());
    }
    let verb = if install { "Installed" } else { "Removed" };
    Ok(match changed.is_empty() {
        true if install => format!("ReUI's {} hook was already in place", shell.name()),
        true => format!("No ReUI {} hook to remove", shell.name()),
        false => format!("{verb} ReUI's {} hook in {} (new windows pick it up)", shell.name(), changed.join(", ")),
    })
}

/// The startup files for `shell`, and whether each may be created.
fn files(shell: Shell) -> Result<Vec<(PathBuf, bool)>, String> {
    Ok(match shell {
        Shell::PowerShell => {
            let docs = documents()?;
            vec![(docs.join("PowerShell").join("profile.ps1"), true), (docs.join("WindowsPowerShell").join("profile.ps1"), false)]
        }
        Shell::Bash => vec![(home()?.join(".bashrc"), true)],
    })
}

fn home() -> Result<PathBuf, String> {
    std::env::var("USERPROFILE").map(PathBuf::from).map_err(|_| "USERPROFILE is not set".to_string())
}

/// The Documents folder PowerShell uses (often moved into OneDrive). A sandbox
/// (`REUI_CONFIG_DIR`) keeps it under its own home instead.
fn documents() -> Result<PathBuf, String> {
    if std::env::var_os("REUI_CONFIG_DIR").is_some() {
        return Ok(home()?.join("Documents"));
    }
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_Documents, SHGetKnownFolderPath, KF_FLAG_DEFAULT};
    unsafe {
        if let Ok(raw) = SHGetKnownFolderPath(&FOLDERID_Documents, KF_FLAG_DEFAULT, None) {
            let path = raw.to_string().ok().map(PathBuf::from);
            CoTaskMemFree(Some(raw.0 as *const _));
            if let Some(path) = path {
                return Ok(path);
            }
        }
    }
    Ok(home()?.join("Documents"))
}

/// Whether `text` holds ReUI's block (Settings and the uninstaller look for the same line).
#[cfg(test)]
fn has_block(text: &str) -> bool {
    text.contains(BEGIN)
}

/// `text` with ReUI's block at its end (an older one taken out first).
pub fn with_block(text: &str, block: &str) -> String {
    let mut out = without_block(text);
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(block);
    out
}

/// `text` without ReUI's block (and the blank line we put before it); anything else as it
/// was. A begin marker with no end leaves the file alone: it isn't ours to guess at.
pub fn without_block(text: &str) -> String {
    let (Some(start), Some(end)) = (text.find(BEGIN), text.find(END)) else { return text.to_string() };
    if end < start {
        return text.to_string();
    }
    let mut after = end + END.len();
    if text[after..].starts_with("\r\n") {
        after += 2;
    } else if text[after..].starts_with('\n') {
        after += 1;
    }
    let before = text[..start].strip_suffix("\n\n").map(|b| format!("{b}\n")).or_else(|| text[..start].strip_suffix("\r\n\r\n").map(|b| format!("{b}\r\n"))).unwrap_or_else(|| text[..start].to_string());
    format!("{before}{}", &text[after..])
}

/// The hook for `shell`, running the bridge at `exe` (forward slashes).
pub fn block(shell: Shell, exe: &str) -> String {
    let body = match shell {
        Shell::PowerShell => POWERSHELL.replace("__REUI_BRIDGE__", &exe.replace('\'', "''")),
        Shell::Bash => BASH.replace("__REUI_BRIDGE__", &exe.replace('\'', r"'\''")),
    };
    format!("{BEGIN}\n{}\n{END}\n", body.replace("__REUI_MIN__", &MIN_SECONDS.to_string()).trim_end())
}

/// Wraps the prompt that's there when the profile runs (put it after a prompt theme's own
/// line to keep the theme). `$?` and `$LASTEXITCODE` are read first and put back after, so
/// the prompt and the next command see them as they were.
const POWERSHELL: &str = r#"# Tells ReUI when a command that took __REUI_MIN__ s or more finishes. Remove with Settings → Coding agents.
if (-not $global:__reui_prompt) {
    $global:__reui_prompt = $function:prompt
    function global:prompt {
        $ok = $?; $code = $global:LASTEXITCODE
        $last = Get-History -Count 1
        if ($last -and $last.Id -ne $global:__reui_seen) {
            $global:__reui_seen = $last.Id
            $seconds = [int]($last.EndExecutionTime - $last.StartExecutionTime).TotalSeconds
            if ($seconds -ge __REUI_MIN__) {
                $exit = if ($ok) { 0 } elseif ($code) { $code } else { 1 }
                $env:REUI_COMMAND = $last.CommandLine
                try { & '__REUI_BRIDGE__' finished --exit $exit --seconds $seconds --shell powershell 2>$null | Out-Null } catch {}
                Remove-Item Env:REUI_COMMAND -ErrorAction SilentlyContinue
            }
        }
        $global:LASTEXITCODE = $code
        & $global:__reui_prompt
    }
}"#;

/// A DEBUG trap notes when the first command after a prompt starts; the last entry of
/// PROMPT_COMMAND reports it. `$?` of the command is taken by the first entry, before
/// anything else in PROMPT_COMMAND changes it.
const BASH: &str = r#"# Tells ReUI when a command that took __REUI_MIN__ s or more finishes. Remove with Settings → Coding agents.
if [ -z "$__reui_hooked" ]; then
    __reui_hooked=1
    __reui_ready=1
    __reui_before() {
        if [ -n "$__reui_ready" ] && [ -z "$COMP_LINE" ]; then
            __reui_ready=
            __reui_start=$SECONDS
            __reui_cmd=$BASH_COMMAND
        fi
    }
    __reui_after() {
        if [ -n "$__reui_start" ]; then
            local seconds=$((SECONDS - __reui_start))
            if [ "$seconds" -ge __REUI_MIN__ ]; then
                REUI_COMMAND="$__reui_cmd" '__REUI_BRIDGE__' finished --exit "$__reui_status" --seconds "$seconds" --shell bash >/dev/null 2>&1
            fi
        fi
        __reui_start=
        __reui_ready=1
    }
    trap '__reui_before' DEBUG
    PROMPT_COMMAND="__reui_status=\$?;${PROMPT_COMMAND:+$PROMPT_COMMAND;}__reui_after"
fi"#;

fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("can't create {}: {e}", dir.display()))?;
    }
    let temp = path.with_extension("reui-tmp");
    std::fs::write(&temp, text).map_err(|e| format!("can't write {}: {e}", temp.display()))?;
    std::fs::rename(&temp, path).map_err(|e| format!("can't replace {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_block_goes_at_the_end_and_comes_out_exactly() {
        let profile = "Import-Module posh-git\nSet-Alias g git\n";
        let block = block(Shell::PowerShell, "C:/Users/me/AppData/Local/ReUI/bin/reui-bridge.exe");
        let installed = with_block(profile, &block);
        assert!(installed.starts_with(profile) && installed.ends_with(&block) && has_block(&installed));
        assert_eq!(without_block(&installed), profile);
        // Installing again replaces it rather than adding a second.
        let again = with_block(&installed, &block);
        assert_eq!(again, installed);
        assert_eq!(again.matches(BEGIN).count(), 1);
    }

    #[test]
    fn a_new_file_holds_only_the_block_and_removing_it_leaves_it_empty() {
        let block = block(Shell::Bash, "C:/ReUI/reui-bridge.exe");
        let installed = with_block("", &block);
        assert_eq!(installed, block);
        assert_eq!(without_block(&installed), "");
    }

    #[test]
    fn whatever_follows_the_block_stays() {
        let text = format!("a\n\n{BEGIN}\nx\n{END}\nb\n");
        assert_eq!(without_block(&text), "a\nb\n");
        let crlf = format!("a\r\n\r\n{BEGIN}\r\nx\r\n{END}\r\nb\r\n");
        assert_eq!(without_block(&crlf), "a\r\nb\r\n");
        // Half a block isn't ours to remove.
        let broken = format!("a\n{BEGIN}\nx\n");
        assert_eq!(without_block(&broken), broken);
    }

    #[test]
    fn the_bridge_path_is_quoted_for_each_shell() {
        let ps = block(Shell::PowerShell, "C:/Users/O'Neil/reui-bridge.exe");
        assert!(ps.contains("& 'C:/Users/O''Neil/reui-bridge.exe' finished"));
        assert!(ps.contains("-ge 10") && !ps.contains("__REUI"));
        let sh = block(Shell::Bash, "C:/Users/O'Neil/reui-bridge.exe");
        assert!(sh.contains(r"'C:/Users/O'\''Neil/reui-bridge.exe' finished"));
        assert!(sh.contains("-ge 10") && !sh.contains("__REUI"));
    }

    #[test]
    fn shells_by_name() {
        assert_eq!(Shell::parse("powershell"), Some(Shell::PowerShell));
        assert_eq!(Shell::parse("pwsh"), Some(Shell::PowerShell));
        assert_eq!(Shell::parse("bash"), Some(Shell::Bash));
        assert_eq!(Shell::parse("fish"), None);
    }
}
