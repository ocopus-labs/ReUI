//! The bridge's ancestors, nearest first (the agent, its shell, the terminal…), sent with
//! each event so the hub can bring the agent's terminal forward when you click "Open
//! terminal" (PRD agent moments §6). The bridge knows its own process; the hub knows which
//! of these has a window when you click. One process snapshot per event: a few
//! milliseconds, and nothing at all is read when it fails.

use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::Console::GetConsoleWindow;
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
use windows::Win32::UI::WindowsAndMessaging::IsWindowVisible;

/// Far more than any real chain (bridge → agent → shell → terminal → its host).
const MAX_DEPTH: usize = 12;

/// `start`'s ancestors in a table of (pid, parent pid), nearest first. Stops at a missing
/// parent, a loop (PIDs are reused, so a table can contain one) or [`MAX_DEPTH`].
pub fn chain(table: &[(u32, u32)], start: u32) -> Vec<u32> {
    let mut out = Vec::new();
    let mut at = start;
    while out.len() < MAX_DEPTH {
        let Some(&(_, parent)) = table.iter().find(|(pid, _)| *pid == at) else { break };
        if parent == 0 || parent == start || out.contains(&parent) {
            break;
        }
        out.push(parent);
        at = parent;
    }
    out
}

/// This process's ancestors, or nothing when the snapshot fails.
pub fn own() -> Vec<u32> {
    chain(&processes(), std::process::id())
}

/// A visible console window this bridge shares (a classic console host, where the window
/// may not belong to any ancestor); 0 when there is none.
pub fn console() -> isize {
    unsafe {
        let hwnd: HWND = GetConsoleWindow();
        if hwnd.is_invalid() || !IsWindowVisible(hwnd).as_bool() {
            return 0;
        }
        hwnd.0 as isize
    }
}

fn processes() -> Vec<(u32, u32)> {
    let mut table = Vec::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return table };
        let mut entry = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut more = Process32FirstW(snapshot, &mut entry).is_ok();
        while more {
            table.push((entry.th32ProcessID, entry.th32ParentProcessID));
            more = Process32NextW(snapshot, &mut entry).is_ok();
        }
        _ = CloseHandle(snapshot);
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walks_up_to_the_terminal() {
        // bridge 40 → claude 30 → pwsh 20 → WindowsTerminal 10 → explorer 5 (no parent left)
        let table = [(40, 30), (30, 20), (20, 10), (10, 5), (99, 1)];
        assert_eq!(chain(&table, 40), [30, 20, 10, 5]);
        assert_eq!(chain(&table, 7), Vec::<u32>::new(), "not in the table");
    }

    #[test]
    fn reused_pids_cant_loop_it() {
        assert_eq!(chain(&[(40, 30), (30, 20), (20, 30)], 40), [30, 20]);
        assert_eq!(chain(&[(40, 30), (30, 40)], 40), [30], "back to the start");
        assert_eq!(chain(&[(40, 0)], 40), Vec::<u32>::new(), "the idle process is no parent");
        let long: Vec<(u32, u32)> = (1..100).map(|p| (p, p + 1)).collect();
        assert_eq!(chain(&long, 1).len(), MAX_DEPTH);
    }
}
