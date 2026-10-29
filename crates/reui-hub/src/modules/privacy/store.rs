//! The OS side: reads the consent store, finds the processes behind each use, and wakes on
//! a change, all on one thread of its own (`reui-privacy`).
//!
//! Event-driven, no polling. The thread sleeps in `WaitForMultipleObjects` on:
//! - a `RegNotifyChangeKeyValue` event on `ConsentStore` (whole subtree, names and values),
//!   one-shot so it's re-armed before each read; registered from this long-lived thread,
//!   which the notification is tied to;
//! - a handle to every process holding a use, so an app that exits or crashes without
//!   writing its stop is noticed the moment it goes;
//! - a stop event for shutdown.
//!
//! Each wake reads the five capability keys (a few dozen values, well under a millisecond)
//! and, only while something is in use, the process list.

use std::sync::mpsc as std_mpsc;
use std::thread::JoinHandle;

use tokio::sync::mpsc::UnboundedSender;
use windows::core::{w, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, FILETIME, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegGetValueW, RegNotifyChangeKeyValue, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, KEY_NOTIFY, KEY_READ, REG_NOTIFY_CHANGE_LAST_SET, REG_NOTIFY_CHANGE_NAME,
    RRF_RT_REG_QWORD,
};
use windows::Win32::System::Threading::{
    CreateEventW, GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, SetEvent, WaitForMultipleObjects, WaitForSingleObject, INFINITE, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
};

use super::alive::{alive, Live, Proc};
use super::consent::{self, App, Entry, Kind};

const ROOT: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore");
/// `WaitForMultipleObjects` takes 64; two are the stop and registry events.
const MAX_PROCESSES: usize = 62;

/// Keeps the watcher thread running; dropping it stops the thread (and blocks briefly
/// for it, so drop it off the hub thread).
pub struct Watcher {
    stop: isize,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        unsafe {
            _ = SetEvent(HANDLE(self.stop as _));
        }
        if let Some(t) = self.thread.take() {
            _ = t.join();
        }
        unsafe {
            _ = CloseHandle(HANDLE(self.stop as _));
        }
    }
}

/// Starts watching; the first snapshot is sent straight away, then one on every change
/// (only when what's live differs). `None` if the store can't be opened or watched.
pub fn watch(tx: UnboundedSender<Vec<Live>>) -> Option<Watcher> {
    let stop = unsafe { CreateEventW(None, true, false, None) }.ok()?;
    let stop_raw = stop.0 as isize;
    let (ready_tx, ready) = std_mpsc::channel();
    let thread = std::thread::Builder::new().name("reui-privacy".into()).spawn(move || run(stop_raw, tx, ready_tx)).ok();
    match (thread, ready.recv().unwrap_or(false)) {
        (Some(thread), true) => Some(Watcher { stop: stop_raw, thread: Some(thread) }),
        (thread, _) => {
            if let Some(t) = thread {
                _ = t.join();
            }
            unsafe { _ = CloseHandle(stop) };
            None
        }
    }
}

fn own_dir() -> String {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.display().to_string())).unwrap_or_default()
}

fn arm(key: HKEY, event: HANDLE) -> bool {
    unsafe { RegNotifyChangeKeyValue(key, true, REG_NOTIFY_CHANGE_NAME | REG_NOTIFY_CHANGE_LAST_SET, Some(event), true) == ERROR_SUCCESS }
}

fn run(stop: isize, tx: UnboundedSender<Vec<Live>>, ready: std_mpsc::Sender<bool>) {
    let stop = HANDLE(stop as _);
    let mut root = HKEY::default();
    if unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, ROOT, None, KEY_READ | KEY_NOTIFY, &mut root) } != ERROR_SUCCESS {
        eprintln!("privacy: can't open the consent store; no indicators");
        _ = ready.send(false);
        return;
    }
    let Ok(changed) = (unsafe { CreateEventW(None, false, false, None) }) else {
        unsafe { _ = RegCloseKey(root) };
        _ = ready.send(false);
        return;
    };
    if !arm(root, changed) {
        eprintln!("privacy: can't watch the consent store; no indicators");
        unsafe {
            _ = RegCloseKey(root);
            _ = CloseHandle(changed);
        }
        _ = ready.send(false);
        return;
    }
    _ = ready.send(true);
    let own = own_dir();
    let mut last: Option<Vec<Live>> = None;
    loop {
        let entries = read(root);
        let uses = consent::uses(&entries, |a| consent::is_own(a, &own));
        let procs = if uses.is_empty() { Vec::new() } else { processes(&uses) };
        let (live, waits) = hold(alive(&uses, &procs));
        if last.as_ref() != Some(&live) {
            if tx.send(live.clone()).is_err() {
                close_all(&waits);
                break;
            }
            last = Some(live);
        }
        let mut handles = vec![stop, changed];
        handles.extend(waits.iter().copied());
        let woke = unsafe { WaitForMultipleObjects(&handles, false, INFINITE) };
        close_all(&waits);
        let index = woke.0.wrapping_sub(WAIT_OBJECT_0.0) as usize;
        if index == 0 || index >= handles.len() {
            break;
        }
        // The registry's notification is one-shot: re-arm before reading so no change is
        // missed between the read and the next wait.
        if index == 1 && !arm(root, changed) {
            eprintln!("privacy: lost the consent store watch");
            break;
        }
    }
    unsafe {
        _ = RegCloseKey(root);
        _ = CloseHandle(changed);
    }
}

/// Opens a wait handle on each use's processes; a process that's already gone (signalled)
/// drops out, and a use with none left goes with it.
fn hold(live: Vec<Live>) -> (Vec<Live>, Vec<HANDLE>) {
    let mut waits = Vec::new();
    let mut out = Vec::new();
    for mut l in live {
        let mut kept = Vec::new();
        for pid in l.pids {
            if waits.len() >= MAX_PROCESSES {
                // Out of wait slots: still shown, its exit noticed on the next change.
                kept.push(pid);
                continue;
            }
            match unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) } {
                Ok(h) if unsafe { WaitForSingleObject(h, 0) } == WAIT_OBJECT_0 => unsafe { _ = CloseHandle(h) },
                Ok(h) => {
                    waits.push(h);
                    kept.push(pid);
                }
                // Can't wait on it (protected): trust the registry for this one.
                Err(_) => kept.push(pid),
            }
        }
        if !kept.is_empty() {
            l.pids = kept;
            out.push(l);
        }
    }
    (out, waits)
}

fn close_all(handles: &[HANDLE]) {
    for h in handles {
        unsafe { _ = CloseHandle(*h) };
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

/// A QWORD value of `sub` under `key`; 0 when missing.
fn qword(key: HKEY, sub: &[u16], name: PCWSTR) -> u64 {
    let mut value = 0u64;
    let mut size = size_of::<u64>() as u32;
    let ok = unsafe { RegGetValueW(key, PCWSTR(sub.as_ptr()), name, RRF_RT_REG_QWORD, None, Some(&mut value as *mut u64 as *mut _), Some(&mut size)) };
    if ok == ERROR_SUCCESS {
        value
    } else {
        0
    }
}

fn subkeys(key: HKEY) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = [0u16; 512];
    for index in 0.. {
        let mut len = buf.len() as u32;
        let r = unsafe { RegEnumKeyExW(key, index, Some(PWSTR(buf.as_mut_ptr())), &mut len, None, None, None, None) };
        if r != ERROR_SUCCESS {
            break;
        }
        out.push(String::from_utf16_lossy(&buf[..len as usize]));
    }
    out
}

fn open(parent: HKEY, sub: &str) -> Option<HKEY> {
    let mut key = HKEY::default();
    let name = wide(sub);
    (unsafe { RegOpenKeyExW(parent, PCWSTR(name.as_ptr()), None, KEY_READ, &mut key) } == ERROR_SUCCESS).then_some(key)
}

/// Every app's record under one capability key.
fn read_capability(cap: HKEY, kind: Kind, out: &mut Vec<Entry>) {
    let mut record = |key: HKEY, name: &str, non_packaged: bool| {
        let sub = wide(name);
        let (start, stop) = (qword(key, &sub, w!("LastUsedTimeStart")), qword(key, &sub, w!("LastUsedTimeStop")));
        if start != 0 {
            out.push(Entry { kind, app: App::from_key(name, non_packaged), start, stop });
        }
    };
    for name in subkeys(cap) {
        if name.eq_ignore_ascii_case("NonPackaged") {
            if let Some(np) = open(cap, &name) {
                for exe in subkeys(np) {
                    record(np, &exe, true);
                }
                unsafe { _ = RegCloseKey(np) };
            }
        } else {
            record(cap, &name, false);
        }
    }
}

/// The consent store's records for the kinds ReUI shows. A capability nobody has used yet
/// may have no key at all.
pub fn read(root: HKEY) -> Vec<Entry> {
    let mut out = Vec::new();
    for kind in Kind::ALL {
        for cap in kind.capabilities() {
            if let Some(key) = open(root, cap) {
                read_capability(key, kind, &mut out);
                unsafe { _ = RegCloseKey(key) };
            }
        }
    }
    out
}

/// Running processes that could hold one of `uses`: those with a matching file name, or
/// every process while a packaged app is in use (its exe isn't in the store).
fn processes(uses: &[consent::Use]) -> Vec<Proc> {
    let names: Vec<String> = uses.iter().filter_map(|u| u.app.file_name()).collect();
    let any_packaged = uses.iter().any(|u| matches!(u.app, App::Packaged(_)));
    let Ok(snap) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else { return Vec::new() };
    let mut out = Vec::new();
    let mut e = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
    let mut ok = unsafe { Process32FirstW(snap, &mut e) }.is_ok();
    while ok {
        let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
        let name = String::from_utf16_lossy(&e.szExeFile[..len]).to_lowercase();
        if any_packaged || names.contains(&name) {
            let (path, created) = describe(e.th32ProcessID);
            out.push(Proc { pid: e.th32ProcessID, name, path, created });
        }
        ok = unsafe { Process32NextW(snap, &mut e) }.is_ok();
    }
    unsafe { _ = CloseHandle(snap) };
    out
}

/// A process's full image path and creation time, where it lets us read them.
fn describe(pid: u32) -> (Option<String>, Option<u64>) {
    let Ok(h) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else { return (None, None) };
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    let path = unsafe { QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len) }.ok().map(|_| String::from_utf16_lossy(&buf[..len as usize]));
    let (mut created, mut exited, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
    let created = unsafe { GetProcessTimes(h, &mut created, &mut exited, &mut kernel, &mut user) }.ok().map(|_| (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime));
    unsafe { _ = CloseHandle(h) };
    (path, created)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads (never writes) this machine's consent store and prints what's live.
    #[test]
    #[ignore = "reads the machine's consent store"]
    fn privacy_reads_the_real_store() {
        let mut root = HKEY::default();
        assert_eq!(unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, ROOT, None, KEY_READ, &mut root) }, ERROR_SUCCESS);
        let entries = read(root);
        let uses = consent::uses(&entries, |a| consent::is_own(a, &own_dir()));
        println!("{} records; in use per the store: {uses:#?}", entries.len());
        println!("with a process behind them: {:#?}", hold(alive(&uses, &processes(&uses))).0);
        unsafe { _ = RegCloseKey(root) };
    }
}
