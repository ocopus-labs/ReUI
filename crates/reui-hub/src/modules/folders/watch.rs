//! Watching folders for changes: one thread per [`Watch`], an overlapped
//! `ReadDirectoryChangesW` per folder and one `WaitForMultipleObjects` over them, so a
//! quiet folder costs a blocked thread and nothing else (no polling, no timers). Only the
//! folder itself, not its sub-folders: browsers and Snipping Tool write at the top.
//!
//! Dropping the [`Watch`] stops the thread (its stop event), cancels the reads and closes
//! the folders; a module re-creates it when its settings change the folder list.

use std::path::PathBuf;
use std::thread::JoinHandle;

use tokio::sync::mpsc::UnboundedSender;
use windows::core::HSTRING;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::Threading::{CreateEventW, ResetEvent, SetEvent, WaitForMultipleObjects, INFINITE};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

use super::notify::{self, Change};

/// 64 KB: the most a read may ask for on a network share, and room for hundreds of
/// changes between two wakes.
const BUFFER_WORDS: usize = 16 * 1024;
/// `WaitForMultipleObjects` takes 64 handles, one of them the stop event.
const MAX_FOLDERS: usize = 63;

/// A running folder watch; changes go to the channel it was started with.
pub struct Watch {
    /// The stop event, as an integer so the watch can live in a module's (`Send`) future.
    stop: isize,
    thread: Option<JoinHandle<()>>,
}

impl Watch {
    /// Watches `folders` (those that exist; the rest are skipped) on a thread named `name`.
    /// `None` only when Windows won't give us an event.
    pub fn start(name: &str, folders: Vec<PathBuf>, tx: UnboundedSender<Change>) -> Option<Watch> {
        let stop = unsafe { CreateEventW(None, true, false, None) }.ok()?;
        let stop_raw = stop.0 as isize;
        let thread = std::thread::Builder::new().name(name.into()).spawn(move || run(HANDLE(stop_raw as _), folders, tx));
        match thread {
            Ok(thread) => Some(Watch { stop: stop_raw, thread: Some(thread) }),
            Err(_) => {
                _ = unsafe { CloseHandle(stop) };
                None
            }
        }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        let stop = HANDLE(self.stop as _);
        _ = unsafe { SetEvent(stop) };
        if let Some(thread) = self.thread.take() {
            _ = thread.join();
        }
        _ = unsafe { CloseHandle(stop) };
    }
}

/// One watched folder and its read in flight. The buffer and the `OVERLAPPED` are boxed:
/// Windows writes into them while the read is pending, wherever the `Dir` itself moves.
struct Dir {
    path: PathBuf,
    handle: HANDLE,
    event: HANDLE,
    overlapped: Box<OVERLAPPED>,
    buffer: Box<[u32]>,
    pending: bool,
}

impl Dir {
    fn open(path: PathBuf) -> Option<Dir> {
        unsafe {
            let handle = CreateFileW(
                &HSTRING::from(path.as_os_str()),
                FILE_LIST_DIRECTORY.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
                None,
            )
            .ok()?;
            let Ok(event) = CreateEventW(None, true, false, None) else {
                _ = CloseHandle(handle);
                return None;
            };
            let overlapped = Box::new(OVERLAPPED { hEvent: event, ..Default::default() });
            Some(Dir { path, handle, event, overlapped, buffer: vec![0u32; BUFFER_WORDS].into_boxed_slice(), pending: false })
        }
    }

    /// Starts the next read; `false` when the folder can't be read any more (deleted).
    fn arm(&mut self) -> bool {
        let filter = FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_SIZE | FILE_NOTIFY_CHANGE_LAST_WRITE;
        unsafe {
            _ = ResetEvent(self.event);
            *self.overlapped = OVERLAPPED { hEvent: self.event, ..Default::default() };
            let bytes = (self.buffer.len() * 4) as u32;
            self.pending = ReadDirectoryChangesW(self.handle, self.buffer.as_mut_ptr().cast(), bytes, false, filter, None, Some(&mut *self.overlapped), None).is_ok();
        }
        self.pending
    }

    /// What the finished read found; `None` when it failed (the folder went away).
    fn take(&mut self) -> Option<Vec<Change>> {
        self.pending = false;
        let mut bytes = 0u32;
        unsafe { GetOverlappedResult(self.handle, &*self.overlapped, &mut bytes, false) }.ok()?;
        if bytes == 0 {
            return Some(vec![Change::Overflow(self.path.clone())]);
        }
        let len = (bytes as usize).min(self.buffer.len() * 4);
        let raw = unsafe { std::slice::from_raw_parts(self.buffer.as_ptr().cast::<u8>(), len) };
        Some(notify::parse(&self.path, raw))
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        unsafe {
            if self.pending {
                // The buffer must outlive the read: cancel it and wait until it's over.
                _ = CancelIoEx(self.handle, Some(&*self.overlapped));
                let mut bytes = 0u32;
                _ = GetOverlappedResult(self.handle, &*self.overlapped, &mut bytes, true);
            }
            _ = CloseHandle(self.handle);
            _ = CloseHandle(self.event);
        }
    }
}

fn run(stop: HANDLE, folders: Vec<PathBuf>, tx: UnboundedSender<Change>) {
    let mut dirs: Vec<Dir> = folders.into_iter().take(MAX_FOLDERS).filter_map(Dir::open).collect();
    dirs.retain_mut(Dir::arm);
    loop {
        let handles: Vec<HANDLE> = std::iter::once(stop).chain(dirs.iter().map(|d| d.event)).collect();
        let woke = unsafe { WaitForMultipleObjects(&handles, false, INFINITE) };
        let index = woke.0.wrapping_sub(WAIT_OBJECT_0.0) as usize;
        if index == 0 || index >= handles.len() {
            return; // stopped, or the wait itself failed
        }
        let dir = &mut dirs[index - 1];
        let alive = match dir.take() {
            Some(changes) => {
                for change in changes {
                    if tx.send(change).is_err() {
                        return; // the module is gone
                    }
                }
                dir.arm()
            }
            None => false,
        };
        if !alive {
            eprintln!("folders: stopped watching {}", dir.path.display());
            dirs.remove(index - 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real folder, really watched: a file written, grown, renamed and deleted arrives as
    /// those changes, in order.
    #[test]
    fn a_watched_folder_reports_its_changes() {
        let dir = std::env::temp_dir().join(format!("reui-watch-test-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let watch = Watch::start("reui-watch-test", vec![dir.clone(), dir.join("missing")], tx).unwrap();
        // The thread arms its reads just after `start` returns; give it a moment.
        std::thread::sleep(std::time::Duration::from_millis(100));
        let part = dir.join("a.bin.part");
        std::fs::write(&part, [0u8; 10]).unwrap();
        std::fs::rename(&part, dir.join("a.bin")).unwrap();
        std::fs::remove_file(dir.join("a.bin")).unwrap();
        let mut seen = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !seen.contains(&Change::Removed(dir.join("a.bin"))) && std::time::Instant::now() < deadline {
            match rx.try_recv() {
                Ok(change) => seen.push(change),
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(10)),
            }
        }
        drop(watch);
        assert!(seen.contains(&Change::Added(part.clone())), "{seen:?}");
        assert!(seen.contains(&Change::Renamed { from: part, to: dir.join("a.bin") }), "{seen:?}");
        assert!(seen.contains(&Change::Removed(dir.join("a.bin"))), "{seen:?}");
        _ = std::fs::remove_dir_all(&dir);
    }
}
