//! A cheap "a toast came or went" signal without package identity: the shell's WNF state
//! that holds the notification count (`0x0D83063EA3BC1035`, a u32).
//!
//! Without identity `UserNotificationListener::NotificationChanged` fails (0x80070490), so
//! the alternative was polling, whose cost grows with every toast waiting in the
//! notification centre. Found on Windows 11 26200 by comparing the change stamps of all
//! 1,605 well-known WNF states (`HKLM\SYSTEM\CurrentControlSet\Control\Notifications`)
//! across a quiet wait and a test toast (`examples/notifications_probe.rs wnf`): this one
//! and its twin `0x0D83063EA3B8D035` moved by one with every toast (data 1, 2, 3) and went
//! to 0 when the toasts were removed, 30–220 ms after each, and never in a quiet minute.
//!
//! Undocumented, like the Do Not Disturb state the timers read (`timers/dnd.rs`), so it's
//! only ever a trigger to list: what's shown always comes from the documented listener, and
//! `schedule.rs` drops to polling if the count ever misses a toast.

use std::ffi::c_void;

use windows::core::{s, w};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

pub const NOTIFICATION_COUNT: u64 = 0x0D83_063E_A3BC_1035;

/// What the callback calls with the new count.
pub type OnChange = Box<dyn Fn(u32) + Send + Sync>;

type Query = unsafe extern "system" fn(state: *const u64, type_id: *const c_void, scope: *const c_void, stamp: *mut u32, buffer: *mut c_void, size: *mut u32) -> i32;
type Callback = unsafe extern "system" fn(state: u64, stamp: u32, type_id: *const c_void, context: *const c_void, buffer: *const u8, length: u32) -> i32;
type Subscribe = unsafe extern "system" fn(subscription: *mut *mut c_void, state: u64, stamp: u32, callback: Callback, context: *const c_void, type_id: *const c_void, group: u32, flags: u32) -> i32;
type Unsubscribe = unsafe extern "system" fn(subscription: *mut c_void) -> i32;

/// A live subscription; dropping it unsubscribes, then frees what the callback used.
pub struct Subscription {
    handle: *mut c_void,
    unsubscribe: Unsubscribe,
    context: *mut OnChange,
}

// The handle and context are only touched by `Drop` (and the callback through ntdll).
unsafe impl Send for Subscription {}

impl Drop for Subscription {
    fn drop(&mut self) {
        unsafe {
            // Waits for a callback in flight, so the context is free to drop after it.
            (self.unsubscribe)(self.handle);
            drop(Box::from_raw(self.context));
        }
    }
}

/// Each change of the count calls `changes` with its new value. `None` if ntdll doesn't have the WNF
/// functions or the state can't be read (another Windows build): the caller polls.
pub fn subscribe(state: u64, changes: OnChange) -> Option<Subscription> {
    unsafe {
        let ntdll = GetModuleHandleW(w!("ntdll.dll")).ok()?;
        let query: Query = std::mem::transmute(GetProcAddress(ntdll, s!("NtQueryWnfStateData"))?);
        let subscribe: Subscribe = std::mem::transmute(GetProcAddress(ntdll, s!("RtlSubscribeWnfStateChangeNotification"))?);
        let unsubscribe: Unsubscribe = std::mem::transmute(GetProcAddress(ntdll, s!("RtlUnsubscribeWnfStateChangeNotification"))?);
        // From the current stamp, so only later changes call back.
        let (mut stamp, mut value, mut size) = (0u32, 0u32, 4u32);
        if query(&state, std::ptr::null(), std::ptr::null(), &mut stamp, (&mut value as *mut u32).cast(), &mut size) < 0 {
            return None;
        }
        let context = Box::into_raw(Box::new(changes));
        let mut handle = std::ptr::null_mut();
        if subscribe(&mut handle, state, stamp, on_change, context as *const c_void, std::ptr::null(), 0, 0) < 0 || handle.is_null() {
            drop(Box::from_raw(context));
            return None;
        }
        Some(Subscription { handle, unsubscribe, context })
    }
}

/// On an ntdll thread-pool thread: hand the value on and return at once.
unsafe extern "system" fn on_change(_state: u64, _stamp: u32, _type_id: *const c_void, context: *const c_void, buffer: *const u8, length: u32) -> i32 {
    let changes = &*(context as *const OnChange);
    let value = if !buffer.is_null() && length >= 4 { u32::from_le_bytes(*(buffer as *const [u8; 4])) } else { 0 };
    changes(value);
    0
}
