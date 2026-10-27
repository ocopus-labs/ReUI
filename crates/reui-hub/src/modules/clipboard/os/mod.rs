//! The Windows side of the clipboard: one thread with a message-only window that receives
//! `WM_CLIPBOARDUPDATE` (an event per copy, no polling) and the hotkeys, reads what was
//! copied, and writes items back and pastes them.
//!
//! Hotkeys: the history hotkey opens the clipboard window. Before anything else it records
//! the *target* (the foreground window, its focused control and the text caret on screen),
//! so the window can open at the caret and paste back into that field; the caret comes from
//! UI Automation, then MSAA, then the classic system caret (see `target.rs`).
//! Ctrl+Alt+1…9 paste the Nth most recent item straight into the foreground app.
//!
//! A paste stack watches for the user's own Ctrl+V (or Shift+Insert) with a low-level
//! keyboard hook, installed only while a stack is active.
//!
//! Two threads. The window thread only ever holds the clipboard long enough to copy raw
//! bytes out (or a prepared item in), and never sleeps for more than a few ms, so other
//! apps aren't left waiting on us. Everything slow (image decode/encode, app icons, paste
//! prep, waiting for modifiers, the paste chord) runs on a worker thread. Copies and pastes
//! go through the worker's single queue, so events leave in the order they happened.
//!
//! Timers (all one-shot, so an idle clipboard costs nothing): a copy starts (or restarts)
//! the 75 ms settle timer, because apps often write in several passes; the clipboard is read
//! once when it fires. If another app is still holding it then, the retry timer tries again
//! after 100 ms and 250 ms. The sequence number of the last copy handled, and of our own
//! last write, make repeated notifications and stale timers no-ops.
//!
//! Privacy: content a password manager or other app marks with the standard exclusion
//! formats is never read, and our own writes carry the same marks so Win+V and other
//! clipboard managers don't record a paste-back.
//!
//! Files: `window.rs` (the window thread), `access.rs` (reading and writing the clipboard),
//! `worker.rs` (the worker thread), `image.rs` and `formats.rs` (images), `paste.rs` (typing
//! the paste), `target.rs` (target and caret) and `hotkey.rs`.

mod access;
mod formats;
mod hotkey;
mod image;
mod paste;
mod target;
mod window;
mod worker;

use std::ffi::c_void;
use std::sync::mpsc as std_mpsc;
use std::sync::Arc;

use windows::Win32::Foundation::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub use self::hotkey::Hotkey;
pub(crate) use self::image::thumbnail;
pub use self::window::start_with;
use self::window::WM_STACK;

#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    /// Plain text, and the rich versions the app offered alongside it (CF_HTML as UTF-8,
    /// RTF bytes) so it pastes back formatted.
    Text { text: String, html: Option<String>, rtf: Option<Vec<u8>> },
    /// Full image (PNG, alpha kept) and a small thumbnail (PNG).
    Image { png: Vec<u8>, thumb: Vec<u8> },
    /// `cut`: Explorer marked it a move ("Preferred DropEffect"); pasted back as a copy.
    Files { paths: Vec<String>, cut: bool },
}

pub struct Captured {
    pub content: Content,
    /// The app it came from (display name), its executable's file name in lower case
    /// ("keepass.exe", for the ignored-apps list), and its icon (PNG).
    pub source: Option<String>,
    pub exe: Option<String>,
    pub icon: Option<Vec<u8>>,
    /// Pixel size of a copied image.
    pub size: Option<(u32, u32)>,
}

/// Where a paste goes back to: the window that was in front when the history hotkey was
/// pressed, its focused control, and the text caret in screen pixels (`None` when no caret
/// was found; the window then opens at the mouse). Windows are `HWND`s as integers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Target {
    pub hwnd: isize,
    pub focus: isize,
    /// Left, top, right, bottom.
    pub caret: Option<[i32; 4]>,
}

impl Target {
    /// The foreground window right now, without looking for the caret (a click on the
    /// island: the mouse is where the user is).
    pub fn now() -> Target {
        unsafe { target::capture_target(None) }
    }
}

pub enum OsEvent {
    Copied(Captured),
    /// The history hotkey, with the target captured before anything else could move focus.
    Hotkey(Target),
    /// Ctrl+Alt+`n`: paste the `n`th most recent item into `target`.
    RecentPaste { n: u32, target: isize },
    /// The user pasted (Ctrl+V / Shift+Insert outside ReUI) while a stack was active.
    StackPasted,
    /// Esc while a stack was active.
    StackCancel,
    /// A paste couldn't be typed into the focused app (it runs elevated, or focus moved);
    /// the content is on the clipboard for the user to paste themselves.
    PasteBlocked,
    /// Something the user should know about the OS side.
    Status(OsStatus),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsStatus {
    /// No clipboard listener: nothing is being recorded.
    ListenerFailed,
    /// The history hotkey is taken by another app.
    HotkeyTaken,
}

pub enum Command {
    /// Put this on the clipboard and paste it into `target` (an `HWND`; `None`: whatever is
    /// in front now). `plain`: text only (no HTML/RTF).
    Paste { content: Content, plain: bool, target: Option<isize> },
    /// Put this on the clipboard only.
    Copy(Content),
}

/// Handle to the clipboard threads.
pub struct Clipboard {
    jobs: std_mpsc::Sender<Job>,
    hwnd: isize,
}

impl Clipboard {
    /// A paste waits (briefly) for its target to be in front; if it never is, nothing is
    /// typed (`OsEvent::PasteBlocked`) and the item stays on the clipboard.
    pub fn send(&self, command: Command) {
        let job = match command {
            Command::Paste { content, plain, target } => Job::Write { content, plain, target: Some(target.unwrap_or_else(|| unsafe { GetForegroundWindow() }.0 as isize)) },
            Command::Copy(content) => Job::Write { content, plain: false, target: None },
        };
        _ = self.jobs.send(job);
    }

    /// Starts or stops watching for the user's paste chord (a paste stack). The hook lives
    /// only while a stack does: a keyboard hook sees every key, so it's never left running.
    pub fn watch_pastes(&self, on: bool) {
        _ = unsafe { PostMessageW(Some(HWND(self.hwnd as *mut c_void)), WM_STACK, WPARAM(on as usize), LPARAM(0)) };
    }
}

/// Registered clipboard formats (ids are global, so both threads share them; 0 = not
/// registered, never read or written).
#[derive(Clone, Copy)]
struct Formats {
    exclude: u32,
    viewer_ignore: u32,
    history: u32,
    png: u32,
    html: u32,
    rtf: u32,
    drop_effect: u32,
}

/// What the window thread copied out, before any decoding.
enum Raw {
    /// UTF-16LE bytes, and CF_HTML / RTF bytes; all without their NUL terminators.
    Text { utf16: Vec<u8>, html: Option<Vec<u8>>, rtf: Option<Vec<u8>> },
    Png(Vec<u8>),
    /// CF_DIBV5 or CF_DIB.
    Dib(Vec<u8>),
    Files { paths: Vec<String>, cut: bool },
}

/// Work for the worker thread, in arrival order.
enum Job {
    Copied { raw: Raw, owner: Option<String> },
    /// Put `content` on the clipboard; with a `target` window (`HWND` as an integer), also
    /// paste it there.
    Write { content: Content, plain: bool, target: Option<isize> },
}

/// A fully prepared clipboard write, handed to the window thread to set in one go.
struct Write {
    data: Arc<Vec<(u32, Vec<u8>)>>,
    reply: std_mpsc::Sender<std::result::Result<(), String>>,
}
