//! The OS side (docs/PRD-update-restart.md §5): reads what Windows Update says about
//! restarts and wakes when it writes, on a thread of its own (`reui-wu-watch`).
//!
//! Event-driven, no polling. The thread sleeps in `WaitForMultipleObjects` on a stop event
//! and one `RegNotifyChangeKeyValue` event per key (whole subtree, names and values,
//! one-shot, so re-armed before each read). Both keys are readable by any user. Each wake
//! reads a few values and sends a [`State`] only when it changed.

use std::sync::mpsc as std_mpsc;
use std::thread::JoinHandle;

use tokio::sync::mpsc::UnboundedSender;
use windows::core::{w, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumValueW, RegGetValueW, RegNotifyChangeKeyValue, RegOpenKeyExW, HKEY, HKEY_LOCAL_MACHINE, KEY_NOTIFY, KEY_READ, REG_NOTIFY_CHANGE_LAST_SET, REG_NOTIFY_CHANGE_NAME,
    RRF_RT_REG_DWORD,
};
use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForMultipleObjects, INFINITE};

use super::plan::State;

/// Holds `RebootRequired` (a subkey whose value names are the updates waiting) while a
/// restart is needed.
const AUTO_UPDATE: PCWSTR = w!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate\Auto Update");
/// `UX\Settings` holds active hours; `UX\StateVariables` what Windows plans (M3).
const UX: PCWSTR = w!(r"SOFTWARE\Microsoft\WindowsUpdate\UX");

/// Keeps the thread running; dropping it stops the thread (blocking briefly for it).
pub struct Watcher {
    stop: isize,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        unsafe { _ = SetEvent(HANDLE(self.stop as _)) };
        if let Some(t) = self.thread.take() {
            _ = t.join();
        }
        unsafe { _ = CloseHandle(HANDLE(self.stop as _)) };
    }
}

/// Starts watching; the first state is sent straight away. `None` if the keys can't be
/// opened or watched (then nothing shows, as if no restart were ever needed).
pub fn watch(tx: UnboundedSender<State>) -> Option<Watcher> {
    let stop = unsafe { CreateEventW(None, true, false, None) }.ok()?;
    let stop_raw = stop.0 as isize;
    let (ready_tx, ready) = std_mpsc::channel();
    let thread = std::thread::Builder::new().name("reui-wu-watch".into()).spawn(move || run(stop_raw, tx, ready_tx)).ok();
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

fn arm(key: HKEY, event: HANDLE) -> bool {
    unsafe { RegNotifyChangeKeyValue(key, true, REG_NOTIFY_CHANGE_NAME | REG_NOTIFY_CHANGE_LAST_SET, Some(event), true) == ERROR_SUCCESS }
}

fn open(path: PCWSTR) -> Option<HKEY> {
    let mut key = HKEY::default();
    (unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, path, None, KEY_READ | KEY_NOTIFY, &mut key) } == ERROR_SUCCESS).then_some(key)
}

fn run(stop: isize, tx: UnboundedSender<State>, ready: std_mpsc::Sender<bool>) {
    let stop = HANDLE(stop as _);
    let keys: Vec<HKEY> = [AUTO_UPDATE, UX].into_iter().filter_map(open).collect();
    let events: Vec<HANDLE> = keys.iter().filter_map(|_| unsafe { CreateEventW(None, false, false, None) }.ok()).collect();
    let close = |keys: &[HKEY], events: &[HANDLE]| unsafe {
        for k in keys {
            _ = RegCloseKey(*k);
        }
        for e in events {
            _ = CloseHandle(*e);
        }
    };
    if keys.len() != 2 || events.len() != 2 || !keys.iter().zip(&events).all(|(k, e)| arm(*k, *e)) {
        eprintln!("update-restart: can't watch Windows Update's keys; no restart notes");
        close(&keys, &events);
        _ = ready.send(false);
        return;
    }
    _ = ready.send(true);
    let mut last: Option<State> = None;
    loop {
        let state = read(keys[0], keys[1]);
        if last.as_ref() != Some(&state) {
            if tx.send(state.clone()).is_err() {
                break;
            }
            last = Some(state);
        }
        let handles = [stop, events[0], events[1]];
        let woke = unsafe { WaitForMultipleObjects(&handles, false, INFINITE) };
        let index = woke.0.wrapping_sub(WAIT_OBJECT_0.0) as usize;
        if index == 0 || index >= handles.len() {
            break;
        }
        // One-shot: re-arm before reading so no change is missed in between.
        if !arm(keys[index - 1], events[index - 1]) {
            eprintln!("update-restart: lost the watch on Windows Update's keys");
            break;
        }
    }
    close(&keys, &events);
}

fn read(auto_update: HKEY, ux: HKEY) -> State {
    State { waiting: value_names(auto_update, w!("RebootRequired")), active: active_hours(ux) }
}

/// The value names of `auto_update\sub` (none if it isn't there).
fn value_names(parent: HKEY, sub: PCWSTR) -> Vec<String> {
    let mut key = HKEY::default();
    if unsafe { RegOpenKeyExW(parent, sub, None, KEY_READ, &mut key) } != ERROR_SUCCESS {
        return Vec::new();
    }
    let mut names = Vec::new();
    let mut buf = [0u16; 256];
    for i in 0..256u32 {
        let mut len = buf.len() as u32;
        if unsafe { RegEnumValueW(key, i, Some(PWSTR(buf.as_mut_ptr())), &mut len, None, None, None, None) } != ERROR_SUCCESS {
            break;
        }
        names.push(String::from_utf16_lossy(&buf[..len as usize]));
    }
    unsafe { _ = RegCloseKey(key) };
    // A key with no values (it happens while Windows writes) still means a restart.
    if names.is_empty() { vec![String::new()] } else { names }
}

fn active_hours(ux: HKEY) -> Option<(u32, u32)> {
    let dword = |name: PCWSTR| -> Option<u32> {
        let mut value = 0u32;
        let mut size = 4u32;
        let ok = unsafe { RegGetValueW(ux, w!("Settings"), name, RRF_RT_REG_DWORD, None, Some((&mut value as *mut u32).cast()), Some(&mut size)) } == ERROR_SUCCESS;
        ok.then_some(value)
    };
    let (start, end) = (dword(w!("ActiveHoursStart"))?, dword(w!("ActiveHoursEnd"))?);
    (start < 24 && end < 24 && start != end).then_some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads this PC's real keys, and the watch starts and stops cleanly.
    #[test]
    #[ignore = "reads Windows Update's keys: cargo test -p reui-hub reads_windows_updates_keys -- --ignored --nocapture"]
    fn reads_windows_updates_keys() {
        let (auto_update, ux) = (open(AUTO_UPDATE).expect("Auto Update"), open(UX).expect("UX"));
        let state = read(auto_update, ux);
        println!("{state:?}");
        unsafe {
            _ = RegCloseKey(auto_update);
            _ = RegCloseKey(ux);
        }
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let watcher = watch(tx).expect("a watch");
        assert_eq!(rx.blocking_recv(), Some(state));
        drop(watcher);
    }
}
