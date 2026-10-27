//! The hotkey recorder's Windows side. While recording, a low-level keyboard hook takes
//! every key (so Win+… chords reach us instead of the shell or the app that owns them, and
//! a recorded Win doesn't open Start); it lets go once all keys are up. Also: whether a
//! chord is free, and the keyboard layout's punctuation.

use std::cell::Cell;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{MapVirtualKeyW, RegisterHotKey, UnregisterHotKey, VkKeyScanW, HOT_KEY_MODIFIERS, MAPVK_VK_TO_CHAR, MOD_NOREPEAT};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::model::hotkey::{modifier_bit, Hotkey};

/// Posted when a chord was pressed: `wparam` the modifier bits, `lparam` the key; key 0
/// means Esc (cancelled).
pub const WM_CHORD: u32 = WM_APP + 1;
/// Posted once every key is up after a chord: time to remove the hook.
pub const WM_RECORD_END: u32 = WM_APP + 2;

const VK_ESCAPE: u32 = 0x1B;
const PROBE_ID: i32 = 0xBFFF;

thread_local! {
    static HOOK: Cell<Option<HHOOK>> = const { Cell::new(None) };
    static WINDOW: Cell<isize> = const { Cell::new(0) };
    static HELD: Cell<u32> = const { Cell::new(0) };
    /// The chord was sent; the rest is swallowed until everything is released.
    static DONE: Cell<bool> = const { Cell::new(false) };
}

pub fn start(hwnd: HWND) {
    stop();
    WINDOW.set(hwnd.0 as isize);
    HELD.set(0);
    DONE.set(false);
    let hook = unsafe { GetModuleHandleW(None).ok().and_then(|module| SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), Some(module.into()), 0).ok()) };
    HOOK.set(hook);
}

pub fn stop() {
    if let Some(hook) = HOOK.take() {
        unsafe { _ = UnhookWindowsHookEx(hook) };
    }
}

fn post(msg: u32, wparam: usize, lparam: isize) {
    unsafe { _ = PostMessageW(Some(HWND(WINDOW.get() as *mut _)), msg, WPARAM(wparam), LPARAM(lparam)) };
}

unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return CallNextHookEx(None, code, wparam, lparam);
    }
    let info = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
    let vk = info.vkCode;
    let down = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
    if let Some(bit) = modifier_bit(vk) {
        HELD.set(if down { HELD.get() | bit } else { HELD.get() & !bit });
    } else if down && !DONE.get() {
        DONE.set(true);
        if vk == VK_ESCAPE && HELD.get() == 0 {
            post(WM_CHORD, 0, 0);
        } else {
            post(WM_CHORD, HELD.get() as usize, vk as isize);
        }
    }
    if DONE.get() && !down && HELD.get() == 0 {
        post(WM_RECORD_END, 0, 0);
    }
    LRESULT(1)
}

/// Whether nobody holds this chord: register it for a moment and let it go.
pub fn hotkey_free(key: &Hotkey) -> bool {
    unsafe {
        let free = RegisterHotKey(None, PROBE_ID, HOT_KEY_MODIFIERS(key.mods) | MOD_NOREPEAT, key.vk).is_ok();
        if free {
            _ = UnregisterHotKey(None, PROBE_ID);
        }
        free
    }
}

/// The character a key types unshifted on the current layout.
pub fn char_of(vk: u32) -> Option<char> {
    let mapped = unsafe { MapVirtualKeyW(vk, MAPVK_VK_TO_CHAR) };
    // The high bit marks a dead key.
    (mapped != 0 && mapped & 0x8000_0000 == 0).then(|| char::from_u32(mapped & 0xFFFF)).flatten()
}

/// The key a character is on, if it types without Shift/Ctrl/Alt.
pub fn vk_of(c: char) -> Option<u32> {
    let mut units = [0u16; 2];
    if c.encode_utf16(&mut units).len() != 1 {
        return None;
    }
    let scan = unsafe { VkKeyScanW(units[0]) } as u16;
    let (vk, shift_state) = (scan & 0xFF, scan >> 8);
    (vk != 0xFF && shift_state == 0).then_some(u32::from(vk))
}

pub use crate::system::running_apps;
