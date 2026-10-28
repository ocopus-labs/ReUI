//! The window thread: a message-only window for copy notifications, hotkeys, the settle
//! and retry timers, the paste-stack keyboard hook, and clipboard writes.

use std::cell::RefCell;
use std::sync::mpsc as std_mpsc;

use tokio::sync::mpsc::UnboundedSender;
use windows::core::{w, Result, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use super::access::{read, write, Read};
use crate::modules::hotkey::Hotkey;
use super::target::{capture_target, exe_path, is_reui, CaretFinder};
use super::worker::Worker;
use super::{Clipboard, Formats, Job, OsEvent, OsStatus, Write};

pub(super) const WM_WRITE_READY: u32 = WM_APP + 3;
/// `wparam` 1: a paste stack started (install the keyboard hook); 0: it ended.
pub(super) const WM_STACK: u32 = WM_APP + 4;
const HOTKEY_ID: i32 = 1;
/// Ctrl+Alt+1…9 are `RECENT_ID + 1` … `RECENT_ID + 9`.
const RECENT_ID: i32 = 10;
const RETRY_TIMER: usize = 1;
const SETTLE_TIMER: usize = 2;
const STACK_TIMER: usize = 3;
/// After the user's paste chord, wait this long before the next stack item replaces the
/// clipboard: the target reads it when it handles the key, which can lag the hook a little.
const STACK_SETTLE_MS: u32 = 200;
/// Apps write the clipboard in passes (empty, text, then HTML...); read once it settles.
const SETTLE_MS: u32 = 75;
/// When another app still holds the clipboard: retry after 100 ms, then once more after 250 ms.
const RETRY_DELAYS_MS: [u32; 2] = [100, 250];

struct State {
    hwnd: HWND,
    events: UnboundedSender<OsEvent>,
    jobs: std_mpsc::Sender<Job>,
    writes: std_mpsc::Receiver<Write>,
    formats: Formats,
    /// Sequence number of our own last write, so pasting doesn't record itself as a copy.
    own_write: u32,
    /// Last sequence number fully handled (read, skipped or excluded): repeated
    /// `WM_CLIPBOARDUPDATE`s and stale timers for it are ignored.
    last_seq: u32,
    /// Retries already spent on a clipboard another app is holding.
    retries: usize,
    /// Foreground window at the last copy: the source when the clipboard has no owner.
    foreground: HWND,
    caret: Option<CaretFinder>,
    /// The paste-stack keyboard hook, while a stack is active.
    hook: Option<HHOOK>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Starts the listener and worker threads; `None` only if the window or threads couldn't be
/// created. A failed listener or a taken hotkey is reported as `OsEvent::Status` instead,
/// since pasting from history still works without them.
pub fn start_with(events: UnboundedSender<OsEvent>, hotkey: Hotkey) -> Option<Clipboard> {
    let format = |name: PCWSTR| unsafe { RegisterClipboardFormatW(name) };
    let formats = Formats {
        exclude: format(w!("ExcludeClipboardContentFromMonitorProcessing")),
        viewer_ignore: format(w!("Clipboard Viewer Ignore")),
        history: format(w!("CanIncludeInClipboardHistory")),
        png: format(w!("PNG")),
        html: format(w!("HTML Format")),
        rtf: format(w!("Rich Text Format")),
        drop_effect: format(w!("Preferred DropEffect")),
    };
    let (job_tx, job_rx) = std_mpsc::channel();
    let (write_tx, write_rx) = std_mpsc::channel();
    let (ready_tx, ready_rx) = std_mpsc::channel();
    let window_events = events.clone();
    let window_jobs = job_tx.clone();
    std::thread::Builder::new()
        .name("clipboard".into())
        .spawn(move || {
            // Caret rectangles in physical pixels, whatever the monitor's scale.
            unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
            let hwnd = match unsafe { create_window() } {
                Ok(hwnd) => hwnd,
                Err(e) => {
                    eprintln!("clipboard: can't create the listener window: {e}");
                    _ = ready_tx.send(None);
                    return;
                }
            };
            unsafe {
                if let Err(e) = AddClipboardFormatListener(hwnd) {
                    eprintln!("clipboard: can't listen for copies: {e}");
                    _ = window_events.send(OsEvent::Status(OsStatus::ListenerFailed));
                }
                if let Err(e) = RegisterHotKey(Some(hwnd), HOTKEY_ID, HOT_KEY_MODIFIERS(hotkey.modifiers) | MOD_NOREPEAT, hotkey.vk) {
                    eprintln!("clipboard: {} is taken by another app ({e}); history opens from the island only", hotkey.label);
                    _ = window_events.send(OsEvent::Status(OsStatus::HotkeyTaken));
                }
                for n in 1..=9u8 {
                    if let Err(e) = RegisterHotKey(Some(hwnd), RECENT_ID + i32::from(n), MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, u32::from(b'0' + n)) {
                        eprintln!("clipboard: Ctrl+Alt+{n} is taken by another app ({e}); recent paste {n} is off");
                    }
                }
            }
            STATE.with(|s| {
                *s.borrow_mut() = Some(State {
                    hwnd,
                    events: window_events.clone(),
                    jobs: window_jobs,
                    writes: write_rx,
                    formats,
                    own_write: 0,
                    last_seq: 0,
                    retries: 0,
                    foreground: HWND::default(),
                    caret: CaretFinder::start(),
                    hook: None,
                })
            });
            _ = ready_tx.send(Some(hwnd.0 as isize));
            let mut msg = MSG::default();
            unsafe {
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    DispatchMessageW(&msg);
                }
            }
        })
        .ok()?;
    let hwnd = ready_rx.recv().ok()??;
    std::thread::Builder::new()
        .name("clipboard-worker".into())
        .spawn(move || Worker::new(events, write_tx, hwnd, formats).run(job_rx))
        .ok()?;
    Some(Clipboard { jobs: job_tx, hwnd })
}

unsafe fn create_window() -> Result<HWND> {
    let instance = GetModuleHandleW(None)?;
    let class = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: instance.into(), lpszClassName: w!("ReUI.Clipboard"), ..Default::default() };
    RegisterClassW(&class);
    CreateWindowExW(WINDOW_EX_STYLE(0), w!("ReUI.Clipboard"), w!("ReUI clipboard"), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, Some(instance.into()), None)
}

/// Runs `f` on the window state. `try_borrow_mut`: clipboard calls can re-enter the wndproc
/// (EmptyClipboard sends WM_DESTROYCLIPBOARD to the previous owner, which may be us).
fn with_state(f: impl FnOnce(&mut State)) {
    STATE.with(|s| {
        if let Ok(mut s) = s.try_borrow_mut() {
            if let Some(state) = s.as_mut() {
                f(state);
            }
        }
    });
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_CLIPBOARDUPDATE => {
            let mut settle = true;
            with_state(|state| {
                // Our own write needs no read (and no timer).
                settle = GetClipboardSequenceNumber() != state.own_write;
                state.retries = 0;
                state.foreground = GetForegroundWindow();
            });
            if settle {
                // A fresh copy supersedes a pending retry; restarting the timer coalesces bursts.
                _ = KillTimer(Some(hwnd), RETRY_TIMER);
                SetTimer(Some(hwnd), SETTLE_TIMER, SETTLE_MS, None);
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == SETTLE_TIMER || wparam.0 == RETRY_TIMER => {
            _ = KillTimer(Some(hwnd), wparam.0);
            with_state(|state| capture(state));
            LRESULT(0)
        }
        WM_HOTKEY if wparam.0 as i32 == HOTKEY_ID => {
            with_state(|state| {
                // First, before anything can move focus: where the user was typing.
                let target = capture_target(state.caret.as_ref());
                // We got the hotkey, so we may pass the foreground on: the window (in the
                // island process) takes focus to search, then gives it back to the target.
                _ = AllowSetForegroundWindow(ASFW_ANY);
                _ = state.events.send(OsEvent::Hotkey(target));
            });
            LRESULT(0)
        }
        WM_HOTKEY if (RECENT_ID + 1..=RECENT_ID + 9).contains(&(wparam.0 as i32)) => {
            let n = (wparam.0 as i32 - RECENT_ID) as u32;
            let target = GetForegroundWindow().0 as isize;
            with_state(|state| _ = state.events.send(OsEvent::RecentPaste { n, target }));
            LRESULT(0)
        }
        WM_STACK => {
            with_state(|state| match (wparam.0 != 0, state.hook) {
                (true, None) => match SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), GetModuleHandleW(None).ok().map(Into::into), 0) {
                    Ok(hook) => state.hook = Some(hook),
                    Err(e) => eprintln!("clipboard: can't watch for pastes ({e}); the stack won't advance"),
                },
                (false, Some(hook)) => {
                    _ = UnhookWindowsHookEx(hook);
                    _ = KillTimer(Some(hwnd), STACK_TIMER);
                    state.hook = None;
                }
                _ => {}
            });
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == STACK_TIMER => {
            _ = KillTimer(Some(hwnd), STACK_TIMER);
            with_state(|state| {
                if state.hook.is_some() {
                    _ = state.events.send(OsEvent::StackPasted);
                }
            });
            LRESULT(0)
        }
        WM_WRITE_READY => {
            with_state(|state| {
                while let Ok(job) = state.writes.try_recv() {
                    let result = write(state.hwnd, &job.data);
                    if result.is_ok() {
                        state.own_write = GetClipboardSequenceNumber();
                        state.last_seq = state.own_write;
                    }
                    _ = job.reply.send(result.map_err(|e| e.to_string()));
                }
            });
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// The paste stack's keyboard hook: the user's own paste chord in another app (not ReUI's
/// window, not our injected keys) advances the stack once it has had time to land; Esc ends
/// it. Runs on the window thread, so it must stay quick.
unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && lparam.0 != 0 {
        let info = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        let message = wparam.0 as u32;
        if info.flags.0 & LLKHF_INJECTED.0 == 0 {
            let held = |vk: VIRTUAL_KEY| GetAsyncKeyState(i32::from(vk.0)) < 0;
            let up = message == WM_KEYUP || message == WM_SYSKEYUP;
            let chord = (info.vkCode == u32::from(b'V') && held(VK_CONTROL)) || (info.vkCode == u32::from(VK_INSERT.0) && held(VK_SHIFT));
            if up && chord && !is_reui(GetForegroundWindow()) {
                with_state(|state| _ = SetTimer(Some(state.hwnd), STACK_TIMER, STACK_SETTLE_MS, None));
            } else if (message == WM_KEYDOWN || message == WM_SYSKEYDOWN) && info.vkCode == u32::from(VK_ESCAPE.0) {
                with_state(|state| _ = state.events.send(OsEvent::StackCancel));
            }
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

/// Reads the current clipboard once and hands it to the worker. If another app still holds
/// it, a timer tries again (100 ms, then 250 ms) instead of sleeping on the message thread;
/// by then the content may have changed, which is fine: we read whatever is current.
unsafe fn capture(state: &mut State) {
    let seq = GetClipboardSequenceNumber();
    if seq == state.own_write || seq == state.last_seq {
        return;
    }
    match read(state.hwnd, state.formats) {
        Read::Busy => match RETRY_DELAYS_MS.get(state.retries) {
            Some(&delay) => {
                state.retries += 1;
                SetTimer(Some(state.hwnd), RETRY_TIMER, delay, None);
            }
            None => {
                state.retries = 0;
                eprintln!("clipboard: another app kept the clipboard open; copy {seq} not recorded");
            }
        },
        Read::Skip => state.last_seq = seq,
        Read::Got(raw, owner) => {
            state.last_seq = seq;
            // No owner (some apps write without owning the clipboard) or one we can't
            // query: credit the app that was in front when the copy happened.
            let owner = owner.and_then(|o| exe_path(o)).or_else(|| exe_path(state.foreground));
            _ = state.jobs.send(Job::Copied { raw, owner });
        }
    }
}
