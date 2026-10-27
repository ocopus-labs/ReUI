//! The OSD's window thread: a hidden top-level window (top-level, so it also gets the
//! `TaskbarCreated` broadcast) that receives
//! - power-setting notifications: the built-in panel's brightness
//!   (`GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS`) and battery saver (`GUID_POWER_SAVING_STATUS`);
//! - brightness keys as Raw Input (HID consumer page 0x0C, usages 0x6F / 0x70, sink mode);
//! - Caps Lock / Num Lock through a low-level keyboard hook, installed only while wanted;
//! - `TaskbarCreated` (Explorer restarted) and `WM_DISPLAYCHANGE`.
//!
//! Callbacks only, no timers except a one-shot after a lock key. Everything is forwarded to
//! the module as an [`OsEvent`].

use std::cell::RefCell;

use tokio::sync::mpsc::UnboundedSender;
use windows::core::{w, GUID};
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::{RegisterPowerSettingNotification, UnregisterPowerSettingNotification, HPOWERNOTIFY, POWERBROADCAST_SETTING};
use windows::Win32::System::SystemServices::{GUID_POWER_SAVING_STATUS, GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CAPITAL, VK_NUMLOCK};
use windows::Win32::UI::Input::{GetRawInputData, RegisterRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RIDEV_INPUTSINK, RID_INPUT, RIM_TYPEHID};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, KillTimer, PostMessageW, PostQuitMessage, RegisterClassW, RegisterWindowMessageW,
    SetTimer, SetWindowsHookExW, UnhookWindowsHookEx, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, MSG, PBT_POWERSETTINGCHANGE, WH_KEYBOARD_LL, WM_APP, WM_DISPLAYCHANGE,
    WM_INPUT, WM_KEYUP, WM_POWERBROADCAST, WM_SYSKEYUP, WM_TIMER, WNDCLASSW, WS_EX_TOOLWINDOW, WS_POPUP, DEVICE_NOTIFY_WINDOW_HANDLE,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockKey {
    Caps,
    Num,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OsEvent {
    /// The built-in panel's brightness in percent. `initial` for the value Windows sends
    /// right after registering (not a change).
    Brightness { percent: u32, initial: bool },
    /// A brightness key was pressed (only on keyboards that report them as HID).
    BrightnessKey,
    BatterySaver { on: bool, initial: bool },
    Lock { key: LockKey, on: bool },
    /// Explorer (re)started: the taskbar and Windows' flyout host are new.
    TaskbarCreated,
    DisplaysChanged,
}

const WM_LOCK_HOOK: u32 = WM_APP + 1;
const WM_STOP: u32 = WM_APP + 2;
const LOCK_TIMER: usize = 1;
/// After a lock key's key-up, when its toggle state has settled.
const LOCK_SETTLE_MS: u32 = 30;

struct State {
    events: UnboundedSender<OsEvent>,
    hwnd: HWND,
    taskbar_created: u32,
    notifications: Vec<HPOWERNOTIFY>,
    seen_brightness: bool,
    seen_saver: bool,
    hook: Option<HHOOK>,
    pending_lock: Option<LockKey>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|s| s.try_borrow_mut().ok().and_then(|mut s| s.as_mut().map(f)))
}

/// The window thread; dropping it closes the window and ends the thread.
pub struct OsWindow {
    hwnd: isize,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl OsWindow {
    pub fn start(events: UnboundedSender<OsEvent>) -> Option<Self> {
        let (ready_tx, ready) = std::sync::mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("reui-osd-window".into())
            .spawn(move || unsafe {
                let Some(hwnd) = create_window() else {
                    eprintln!("osd: can't create the OSD window; brightness and states won't show");
                    _ = ready_tx.send(None);
                    return;
                };
                let taskbar_created = RegisterWindowMessageW(w!("TaskbarCreated"));
                let mut notifications = Vec::new();
                for guid in [&GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS, &GUID_POWER_SAVING_STATUS] {
                    match RegisterPowerSettingNotification(HANDLE(hwnd.0), guid, DEVICE_NOTIFY_WINDOW_HANDLE) {
                        Ok(n) => notifications.push(n),
                        Err(e) => eprintln!("osd: can't watch power setting {guid:?} ({e})"),
                    }
                }
                let consumer = RAWINPUTDEVICE { usUsagePage: 0x0C, usUsage: 0x01, dwFlags: RIDEV_INPUTSINK, hwndTarget: hwnd };
                if let Err(e) = RegisterRawInputDevices(&[consumer], size_of::<RAWINPUTDEVICE>() as u32) {
                    eprintln!("osd: can't watch brightness keys ({e}); every brightness change will show");
                }
                STATE.with(|s| {
                    *s.borrow_mut() = Some(State { events, hwnd, taskbar_created, notifications, seen_brightness: false, seen_saver: false, hook: None, pending_lock: None });
                });
                _ = ready_tx.send(Some(hwnd.0 as isize));
                let mut msg = MSG::default();
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    DispatchMessageW(&msg);
                }
                if let Some(state) = STATE.with(|s| s.borrow_mut().take()) {
                    if let Some(hook) = state.hook {
                        _ = UnhookWindowsHookEx(hook);
                    }
                    for n in state.notifications {
                        _ = UnregisterPowerSettingNotification(n);
                    }
                }
            })
            .ok()?;
        match ready.recv().ok().flatten() {
            Some(hwnd) => Some(Self { hwnd, thread: Some(thread) }),
            None => {
                _ = thread.join();
                None
            }
        }
    }

    /// Watches Caps Lock / Num Lock (a keyboard hook) only while wanted.
    pub fn watch_lock_keys(&self, on: bool) {
        _ = unsafe { PostMessageW(Some(HWND(self.hwnd as *mut _)), WM_LOCK_HOOK, WPARAM(usize::from(on)), LPARAM(0)) };
    }
}

impl Drop for OsWindow {
    fn drop(&mut self) {
        _ = unsafe { PostMessageW(Some(HWND(self.hwnd as *mut _)), WM_STOP, WPARAM(0), LPARAM(0)) };
        if let Some(t) = self.thread.take() {
            _ = t.join();
        }
    }
}

unsafe fn create_window() -> Option<HWND> {
    let instance = GetModuleHandleW(None).ok()?;
    let class = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: instance.into(), lpszClassName: w!("ReUI.Osd"), ..Default::default() };
    RegisterClassW(&class);
    // Never shown: a hidden top-level tool window (message-only windows miss broadcasts).
    CreateWindowExW(WS_EX_TOOLWINDOW, w!("ReUI.Osd"), w!("ReUI OSD"), WS_POPUP, 0, 0, 0, 0, None, None, Some(instance.into()), None).ok()
}

fn send(event: OsEvent) {
    with_state(|s| _ = s.events.send(event));
}

/// The DWORD a power-setting notification carries.
fn setting_value(setting: &POWERBROADCAST_SETTING) -> Option<(GUID, u32)> {
    if setting.DataLength < 4 {
        return None;
    }
    // SAFETY: `Data` holds `DataLength` bytes.
    let bytes = unsafe { std::slice::from_raw_parts(setting.Data.as_ptr(), 4) };
    Some((setting.PowerSetting, u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])))
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_POWERBROADCAST if wparam.0 as u32 == PBT_POWERSETTINGCHANGE && lparam.0 != 0 => {
            if let Some((guid, value)) = setting_value(&*(lparam.0 as *const POWERBROADCAST_SETTING)) {
                if guid == GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS {
                    let initial = with_state(|s| !std::mem::replace(&mut s.seen_brightness, true)).unwrap_or(true);
                    send(OsEvent::Brightness { percent: value.min(100), initial });
                } else if guid == GUID_POWER_SAVING_STATUS {
                    let initial = with_state(|s| !std::mem::replace(&mut s.seen_saver, true)).unwrap_or(true);
                    send(OsEvent::BatterySaver { on: value != 0, initial });
                }
            }
            LRESULT(1)
        }
        WM_INPUT => {
            if raw_input_has_brightness_key(HRAWINPUT(lparam.0 as *mut _)) {
                send(OsEvent::BrightnessKey);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_DISPLAYCHANGE => {
            send(OsEvent::DisplaysChanged);
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_LOCK_HOOK => {
            with_state(|s| match (wparam.0 != 0, s.hook) {
                (true, None) => match SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), GetModuleHandleW(None).ok().map(Into::into), 0) {
                    Ok(hook) => s.hook = Some(hook),
                    Err(e) => eprintln!("osd: can't watch the lock keys ({e})"),
                },
                (false, Some(hook)) => {
                    _ = UnhookWindowsHookEx(hook);
                    s.hook = None;
                }
                _ => {}
            });
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == LOCK_TIMER => {
            _ = KillTimer(Some(hwnd), LOCK_TIMER);
            if let Some(Some(key)) = with_state(|s| s.pending_lock.take()) {
                let vk = if key == LockKey::Caps { VK_CAPITAL } else { VK_NUMLOCK };
                let on = GetKeyState(i32::from(vk.0)) & 1 != 0;
                send(OsEvent::Lock { key, on });
            }
            LRESULT(0)
        }
        WM_STOP => {
            _ = DestroyWindow(hwnd);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => {
            if msg != 0 && with_state(|s| s.taskbar_created == msg).unwrap_or(false) {
                send(OsEvent::TaskbarCreated);
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
    }
}

/// Lock-key key-ups; the toggle state is read a moment later, once Windows has applied it.
/// Runs on the window thread, so it must stay quick.
unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && lparam.0 != 0 {
        let info = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        let up = matches!(wparam.0 as u32, WM_KEYUP | WM_SYSKEYUP);
        let key = match info.vkCode {
            vk if vk == u32::from(VK_CAPITAL.0) => Some(LockKey::Caps),
            vk if vk == u32::from(VK_NUMLOCK.0) => Some(LockKey::Num),
            _ => None,
        };
        if let (true, Some(key)) = (up, key) {
            with_state(|s| {
                s.pending_lock = Some(key);
                SetTimer(Some(s.hwnd), LOCK_TIMER, LOCK_SETTLE_MS, None);
            });
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

unsafe fn raw_input_has_brightness_key(input: HRAWINPUT) -> bool {
    let header = size_of::<RAWINPUTHEADER>() as u32;
    let mut size = 0u32;
    if GetRawInputData(input, RID_INPUT, None, &mut size, header) != 0 || size == 0 || size > 4096 {
        return false;
    }
    // u64 storage keeps the RAWINPUT header aligned.
    let mut buf = vec![0u64; (size as usize).div_ceil(8)];
    if GetRawInputData(input, RID_INPUT, Some(buf.as_mut_ptr().cast()), &mut size, header) != size {
        return false;
    }
    let raw = &*(buf.as_ptr() as *const RAWINPUT);
    if raw.header.dwType != RIM_TYPEHID.0 {
        return false;
    }
    let hid = raw.data.hid;
    let (each, count) = (hid.dwSizeHid as usize, hid.dwCount as usize);
    let offset = std::mem::offset_of!(RAWINPUT, data) + std::mem::offset_of!(windows::Win32::UI::Input::RAWHID, bRawData);
    let total = each.saturating_mul(count);
    if each == 0 || offset + total > size as usize {
        return false;
    }
    let bytes = std::slice::from_raw_parts((buf.as_ptr() as *const u8).add(offset), total);
    bytes.chunks(each).any(report_has_brightness_key)
}

/// Whether a HID consumer-control report holds Brightness Up (0x6F) or Down (0x70). Reports
/// are device-specific; nearly all consumer keyboards send a report ID byte and then 16-bit
/// little-endian usages, which is what this looks for.
pub fn report_has_brightness_key(report: &[u8]) -> bool {
    report.get(1..).is_some_and(|usages| usages.chunks_exact(2).any(|u| u == [0x6F, 0x00] || u == [0x70, 0x00]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osd_brightness_keys_in_consumer_reports() {
        assert!(report_has_brightness_key(&[0x02, 0x6F, 0x00]));
        assert!(report_has_brightness_key(&[0x03, 0x00, 0x00, 0x70, 0x00]));
        // Release (all zero), volume up (0xE9), and a lone report ID.
        assert!(!report_has_brightness_key(&[0x02, 0x00, 0x00]));
        assert!(!report_has_brightness_key(&[0x02, 0xE9, 0x00]));
        assert!(!report_has_brightness_key(&[0x6F]));
        assert!(!report_has_brightness_key(&[]));
        // Not at an odd offset: 0x6F as the high byte of another usage isn't a match.
        assert!(!report_has_brightness_key(&[0x02, 0x00, 0x6F, 0x00]));
    }

    #[test]
    fn osd_power_setting_values() {
        /// A whole notification: the header and its four data bytes.
        #[repr(C)]
        struct Full {
            guid: GUID,
            len: u32,
            data: [u8; 4],
        }
        let mut full = Full { guid: GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS, len: 4, data: 70u32.to_le_bytes() };
        fn setting(f: &Full) -> &POWERBROADCAST_SETTING {
            // SAFETY: same layout as the header, with the data bytes behind it.
            unsafe { &*(f as *const Full as *const POWERBROADCAST_SETTING) }
        }
        assert_eq!(setting_value(setting(&full)), Some((GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS, 70)));
        full.len = 0;
        assert_eq!(setting_value(setting(&full)), None);
    }
}
