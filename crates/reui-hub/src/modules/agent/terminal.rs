//! "Open terminal" (a failed turn, a question only the terminal can answer): brings the
//! window the agent runs in forward (PRD agent moments §6, V5).
//!
//! The bridge sends its parent chain with every event (`reui_parents`, nearest first: the
//! agent, its shell, Windows Terminal or VS Code…), and a visible console window it shares
//! (`reui_console`, a classic console host). On a click the first ancestor that owns a
//! visible top-level window is the one: Windows Terminal's or VS Code's main window (VS
//! Code's terminal runs under a windowless pty host, whose parent has the window), or the
//! console, whose window Windows reports as its first client's. The Shell (Explorer) is
//! never it: a shell launched from the desktop has Explorer as its parent, and its windows
//! are the taskbar and the desktop. This reaches the window, not the tab.
//!
//! The island calls `AllowSetForegroundWindow` with the click it got, so the hub may then
//! set the foreground.

use serde_json::Value;

/// Processes whose windows are never a session's terminal.
const NOT_TERMINALS: [&str; 2] = ["explorer.exe", "dllhost.exe"];

/// Where a session's terminal can be found.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Origin {
    /// The bridge's ancestors, nearest first.
    pub parents: Vec<u32>,
    /// A visible console window the bridge shared (0: none).
    pub console: isize,
}

impl Origin {
    /// From a hook event, when the bridge sent its chain.
    pub(super) fn of(event: &Value) -> Option<Self> {
        let parents: Vec<u32> = event.get("reui_parents")?.as_array()?.iter().filter_map(Value::as_u64).filter_map(|p| u32::try_from(p).ok()).collect();
        let console = event.get("reui_console").and_then(Value::as_i64).unwrap_or(0) as isize;
        (!parents.is_empty() || console != 0).then_some(Self { parents, console })
    }
}

/// The window to bring forward: the first of `parents` with a window in `windows` ((hwnd,
/// pid, exe), front first), skipping the Shell; else the console window when it's still
/// there. Pure.
pub(super) fn pick(origin: &Origin, windows: &[(isize, u32, Option<String>)]) -> Option<isize> {
    let terminal = |(_, _, exe): &&(isize, u32, Option<String>)| exe.as_deref().is_none_or(|e| !NOT_TERMINALS.contains(&e));
    origin
        .parents
        .iter()
        .find_map(|pid| windows.iter().filter(terminal).find(|(_, p, _)| p == pid).map(|(hwnd, ..)| *hwnd))
        .or_else(|| windows.iter().any(|(hwnd, ..)| *hwnd == origin.console && origin.console != 0).then_some(origin.console))
}

/// Brings the session's terminal forward. Blocking (window enumeration); best effort.
pub(super) fn open(origin: &Origin) {
    use windows::Win32::Foundation::HWND;

    use crate::modules::media::player;

    let windows: Vec<(isize, u32, Option<String>)> = player::app_windows().into_iter().map(|(hwnd, pid)| (hwnd.0 as isize, pid, player::exe_name(pid))).collect();
    match pick(origin, &windows) {
        Some(hwnd) => player::focus(HWND(hwnd as *mut _)),
        None => println!("agent: no terminal window for this session"),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn w(hwnd: isize, pid: u32, exe: &str) -> (isize, u32, Option<String>) {
        (hwnd, pid, Some(exe.into()))
    }

    #[test]
    fn the_nearest_ancestor_with_a_window_is_the_terminal() {
        // Windows Terminal: bridge → claude (node) → pwsh → WindowsTerminal.
        let wt = Origin { parents: vec![30, 20, 10, 5], console: 0 };
        let windows = [w(1, 5, "explorer.exe"), w(2, 10, "windowsterminal.exe"), w(3, 77, "chrome.exe")];
        assert_eq!(pick(&wt, &windows), Some(2));
        // VS Code: … → pwsh → Code (pty host, no window) → Code (main window).
        let code = Origin { parents: vec![30, 21, 12, 11], console: 0 };
        assert_eq!(pick(&code, &[w(4, 11, "code.exe"), w(5, 12, "code.exe")]), Some(5), "the nearest wins");
        assert_eq!(pick(&code, &[w(4, 11, "code.exe")]), Some(4));
    }

    #[test]
    fn the_shell_is_never_it_and_a_console_is_the_fallback() {
        let from_desktop = Origin { parents: vec![30, 20, 5], console: 0 };
        assert_eq!(pick(&from_desktop, &[w(1, 5, "explorer.exe")]), None);
        // A classic console: its window, while it's still open.
        let conhost = Origin { parents: vec![30, 5], console: 9 };
        assert_eq!(pick(&conhost, &[w(1, 5, "explorer.exe"), w(9, 444, "conhost.exe")]), Some(9));
        assert_eq!(pick(&conhost, &[w(1, 5, "explorer.exe")]), None, "closed since");
        // An exe we couldn't read isn't ruled out.
        assert_eq!(pick(&from_desktop, &[(6, 20, None)]), Some(6));
    }

    #[test]
    fn the_origin_comes_with_the_event() {
        let event = json!({ "reui_parents": [30, 20, 10], "reui_console": 0 });
        assert_eq!(Origin::of(&event), Some(Origin { parents: vec![30, 20, 10], console: 0 }));
        assert_eq!(Origin::of(&json!({ "hook_event_name": "Stop" })), None, "an older bridge");
        assert_eq!(Origin::of(&json!({ "reui_parents": [] })), None);
    }
}
