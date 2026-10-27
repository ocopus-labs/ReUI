//! What install and uninstall do to running programs. Behind a trait so the step ordering
//! is tested without stopping, starting or deleting anything real.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use windows::core::w;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, WAIT_OBJECT_0};
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, TerminateProcess, WaitForSingleObject, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, GetWindowThreadProcessId, PostMessageW, WM_CLOSE};

use crate::layout::is_under;
use crate::payload_gen::PROGRAMS;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// How long ReUI gets to quit on its own (it puts Windows' flyout back as it goes).
const QUIT_GRACE: Duration = Duration::from_secs(5);
const CHILD_TIMEOUT: Duration = Duration::from_secs(20);

pub trait System {
    /// Stops every ReUI program running from `install_dir` (and only those: a dev build
    /// elsewhere is left alone).
    fn stop_reui(&mut self, install_dir: &Path);
    /// Runs a program without a window and waits for it.
    fn run(&mut self, exe: &Path, args: &[&str]) -> Result<(), String>;
    /// Starts a program and doesn't wait.
    fn launch(&mut self, exe: &Path) -> Result<(), String>;
    /// Deletes `dir` after this process exits (it may hold the running setup exe).
    fn delete_after_exit(&mut self, dir: &Path);
}

pub struct Real;

impl System for Real {
    fn stop_reui(&mut self, install_dir: &Path) {
        // Closing the island makes the hub exit normally (restoring Windows' flyout).
        for hwnd in island_windows() {
            let mut pid = 0;
            unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
            if image_path(pid).is_some_and(|p| is_under(&p, install_dir)) {
                _ = unsafe { PostMessageW(Some(hwnd), WM_CLOSE, Default::default(), LPARAM(0)) };
            }
        }
        let deadline = Instant::now() + QUIT_GRACE;
        for (name, pid) in reui_processes(install_dir) {
            let Ok(process) = (unsafe { OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_TERMINATE, false, pid) }) else { continue };
            // The hub and island quit on their own; anything else (Settings, a bridge) just stops.
            let graceful = name == "reui-hub.exe" || name == "reui-island.exe";
            let left = if graceful { deadline.saturating_duration_since(Instant::now()).as_millis() as u32 } else { 0 };
            unsafe {
                if WaitForSingleObject(process, left) != WAIT_OBJECT_0 {
                    _ = TerminateProcess(process, 1);
                    WaitForSingleObject(process, 2000);
                }
                _ = CloseHandle(process);
            }
        }
    }

    fn run(&mut self, exe: &Path, args: &[&str]) -> Result<(), String> {
        let mut child = Command::new(exe).args(args).creation_flags(CREATE_NO_WINDOW).spawn().map_err(|e| format!("{}: {e}", exe.display()))?;
        let deadline = Instant::now() + CHILD_TIMEOUT;
        loop {
            match child.try_wait().map_err(|e| e.to_string())? {
                Some(status) if status.success() => return Ok(()),
                Some(status) => return Err(format!("{} {} exited with {status}", exe.display(), args.join(" "))),
                None if Instant::now() > deadline => {
                    _ = child.kill();
                    return Err(format!("{} {} didn't finish", exe.display(), args.join(" ")));
                }
                None => std::thread::sleep(Duration::from_millis(50)),
            }
        }
    }

    fn launch(&mut self, exe: &Path) -> Result<(), String> {
        let mut command = Command::new(exe);
        if let Some(dir) = exe.parent() {
            command.current_dir(dir);
        }
        command.spawn().map(drop).map_err(|e| format!("{}: {e}", exe.display()))
    }

    fn delete_after_exit(&mut self, dir: &Path) {
        // A running exe can't delete itself, and delete-on-reboot needs admin; a hidden cmd
        // waits for us to exit, then removes the folder.
        let dir = dir.display().to_string();
        let script = format!("ping -n 3 127.0.0.1 >nul & rd /s /q \"{dir}\"");
        _ = Command::new("cmd.exe").raw_arg("/d /c").raw_arg(format!("\"{script}\"")).creation_flags(CREATE_NO_WINDOW).current_dir(std::env::temp_dir()).spawn();
    }
}

fn island_windows() -> Vec<HWND> {
    let mut out = Vec::new();
    let mut after = None;
    while let Ok(hwnd) = unsafe { FindWindowExW(None, after, w!("ReUI.Island"), None) } {
        out.push(hwnd);
        after = Some(hwnd);
    }
    out
}

fn image_path(pid: u32) -> Option<PathBuf> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        _ = CloseHandle(process);
        ok.then(|| PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])))
    }
}

/// ReUI programs (name, pid) whose image is inside `install_dir`, the hub first.
pub fn reui_processes(install_dir: &Path) -> Vec<(&'static str, u32)> {
    let mut found: Vec<(usize, u32)> = Vec::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return Vec::new() };
        let mut entry = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snapshot, &mut entry).is_ok();
        while ok {
            let n = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..n]).to_lowercase();
            if let Some(rank) = PROGRAMS.iter().position(|p| *p == name) {
                if image_path(entry.th32ProcessID).is_some_and(|p| is_under(&p, install_dir)) {
                    found.push((rank, entry.th32ProcessID));
                }
            }
            ok = Process32NextW(snapshot, &mut entry).is_ok();
        }
        _ = CloseHandle(snapshot);
    }
    found.sort();
    found.into_iter().map(|(rank, pid)| (PROGRAMS[rank], pid)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in "reui-hub.exe" (a renamed copy of Windows' ping) running from a temp
    /// install dir is found and stopped; nothing outside that folder is touched.
    #[test]
    fn stops_only_programs_inside_the_install_dir() {
        let dir = std::env::temp_dir().join(format!("reui-setup-stop-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("reui-hub.exe");
        let system = PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join(r"System32\PING.EXE");
        std::fs::copy(system, &fake).unwrap();
        let mut child = Command::new(&fake).args(["-n", "60", "127.0.0.1"]).creation_flags(CREATE_NO_WINDOW).spawn().unwrap();

        assert_eq!(reui_processes(&dir), vec![("reui-hub.exe", child.id())]);
        assert!(reui_processes(&dir.join("elsewhere")).is_empty());
        let started = Instant::now();
        Real.stop_reui(&dir);
        assert!(child.try_wait().unwrap().is_some() || child.wait().is_ok());
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(reui_processes(&dir).is_empty());
        _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn deletes_a_folder_after_we_let_go() {
        let dir = std::env::temp_dir().join(format!("reui-setup-later dir-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub").join("x.exe"), b"x").unwrap();
        Real.delete_after_exit(&dir);
        let started = Instant::now();
        while dir.exists() && started.elapsed() < Duration::from_secs(15) {
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(!dir.exists());
    }

    #[test]
    fn run_reports_exit_codes() {
        let cmd = PathBuf::from(std::env::var_os("ComSpec").unwrap());
        Real.run(&cmd, &["/d", "/c", "exit 0"]).unwrap();
        assert!(Real.run(&cmd, &["/d", "/c", "exit 3"]).unwrap_err().contains("exit"));
    }
}
