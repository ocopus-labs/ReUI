//! Keyboard for a summoned panel without taking focus. The app in front keeps focus (so a
//! paste lands in it); while the panel is open a low-level keyboard hook on this thread
//! reads the keys the panel handles and swallows them. Installed only while such a panel is
//! open, removed as soon as it closes.
//!
//! The hook runs inside every keystroke on the system, so it only maps the key and queues
//! it; the window handles the queue on a posted [`WM_KEYS`]. Anything with Win held passes
//! (Win+Alt+V must reach the hub's hotkey to close the panel), as do keys we don't handle
//! and injected input (the hub's own paste).

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::features::keys::{map_key, Key, Mods};

/// Posted to the island window when keys are queued.
pub const WM_KEYS: u32 = WM_APP + 3;

/// "No key": sent to cover an Alt chord we swallowed, so the app doesn't see a bare Alt
/// press and release (which opens its menu bar).
const VK_MASK: u16 = 0xE8;

thread_local! {
    static HOOK: Cell<Option<HHOOK>> = const { Cell::new(None) };
    static WINDOW: Cell<isize> = const { Cell::new(0) };
    static PAUSED: Cell<bool> = const { Cell::new(false) };
    static QUEUE: RefCell<VecDeque<Key>> = const { RefCell::new(VecDeque::new()) };
    /// Keys whose press we swallowed: their release is swallowed too.
    static DOWN: RefCell<[bool; 256]> = const { RefCell::new([false; 256]) };
    static MASK: Cell<bool> = const { Cell::new(false) };
}

pub fn captured() -> bool {
    HOOK.get().is_some()
}

/// Starts reading keys for `hwnd`'s panel.
pub fn capture(hwnd: HWND) {
    if captured() {
        return;
    }
    WINDOW.set(hwnd.0 as isize);
    let hook = unsafe { GetModuleHandleW(None).and_then(|m| SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), Some(m.into()), 0)) };
    match hook {
        Ok(h) => HOOK.set(Some(h)),
        Err(e) => eprintln!("island: can't read the keyboard: {e}"),
    }
}

pub fn release() {
    if let Some(h) = HOOK.take() {
        unsafe { _ = UnhookWindowsHookEx(h) };
    }
    QUEUE.with_borrow_mut(VecDeque::clear);
    DOWN.with_borrow_mut(|d| *d = [false; 256]);
    PAUSED.set(false);
}

/// While a menu is up, keys belong to the menu.
pub fn pause(on: bool) {
    PAUSED.set(on);
}

/// Keys handled since the last call, in order. Also covers a swallowed Alt chord.
pub fn drain() -> Vec<Key> {
    if MASK.take() {
        let key = |flags| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(VK_MASK), wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
        };
        unsafe { SendInput(&[key(KEYBD_EVENT_FLAGS(0)), key(KEYEVENTF_KEYUP)], size_of::<INPUT>() as i32) };
    }
    QUEUE.with_borrow_mut(|q| q.drain(..).collect())
}

fn pressed(key: VIRTUAL_KEY) -> bool {
    unsafe { GetAsyncKeyState(key.0 as i32) as u16 & 0x8000 != 0 }
}

/// The character `info` types with the front app's keyboard layout and the modifiers held;
/// `dead` is set for dead keys. Flag 4 keeps the system's dead-key state untouched.
fn typed(info: &KBDLLHOOKSTRUCT, mods: Mods, dead: &Cell<bool>) -> Option<char> {
    let mut state = [0u8; 256];
    for (on, keys) in [(mods.shift, [VK_SHIFT, VK_LSHIFT]), (mods.ctrl, [VK_CONTROL, VK_LCONTROL]), (mods.alt, [VK_MENU, VK_RMENU])] {
        if on {
            for k in keys {
                state[k.0 as usize] = 0x80;
            }
        }
    }
    unsafe {
        if GetKeyState(VK_CAPITAL.0 as i32) & 1 != 0 {
            state[VK_CAPITAL.0 as usize] = 1;
        }
        let thread = GetWindowThreadProcessId(GetForegroundWindow(), None);
        let layout = GetKeyboardLayout(thread);
        let mut buf = [0u16; 8];
        let n = ToUnicodeEx(info.vkCode, info.scanCode, &state, &mut buf, 4, Some(layout));
        if n < 0 {
            dead.set(true);
            return None;
        }
        char::decode_utf16(buf[..n.max(0) as usize].iter().copied()).next()?.ok()
    }
}

/// Whether to swallow this key event.
fn handle(msg: u32, info: &KBDLLHOOKSTRUCT) -> bool {
    let code = (info.vkCode & 0xFF) as usize;
    if !matches!(msg, WM_KEYDOWN | WM_SYSKEYDOWN) {
        return DOWN.with_borrow_mut(|d| std::mem::take(&mut d[code]));
    }
    if PAUSED.get() || info.flags.0 & LLKHF_INJECTED.0 != 0 {
        return false;
    }
    if pressed(VK_LWIN) || pressed(VK_RWIN) || matches!(VIRTUAL_KEY(code as u16), VK_LWIN | VK_RWIN) {
        return false;
    }
    let mods = Mods { shift: pressed(VK_SHIFT), ctrl: pressed(VK_CONTROL), alt: pressed(VK_MENU) || info.flags.0 & LLKHF_ALTDOWN.0 != 0 };
    let dead = Cell::new(false);
    let key = map_key(code as u16, mods, || typed(info, mods, &dead));
    if key.is_none() && !dead.get() {
        return false;
    }
    DOWN.with_borrow_mut(|d| d[code] = true);
    if let Some(key) = key {
        QUEUE.with_borrow_mut(|q| q.push_back(key));
        if mods.alt && !mods.ctrl {
            MASK.set(true);
        }
        unsafe { _ = PostMessageW(Some(HWND(WINDOW.get() as *mut _)), WM_KEYS, WPARAM(0), LPARAM(0)) };
    }
    true
}

unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && handle(wparam.0 as u32, &*(lparam.0 as *const KBDLLHOOKSTRUCT)) {
        return LRESULT(1);
    }
    CallNextHookEx(None, code, wparam, lparam)
}
