//! The hidden top-level window behind [`super::subscribe`] (top-level, so it also gets the
//! `TaskbarCreated` broadcast and `PBT_APMRESUMEAUTOMATIC`), on its own thread. It receives
//! - power-setting notifications: the built-in panel's brightness, battery saver, the power
//!   source (AC / battery / UPS) and the battery's percentage;
//! - resume from sleep, and session unlock (`WTSRegisterSessionNotification`);
//! - volume and portable-device interface arrivals (`RegisterDeviceNotification`; the
//!   `DBT_DEVTYP_VOLUME` broadcasts aren't used);
//! - brightness keys as Raw Input (HID consumer page 0x0C, usages 0x6F / 0x70, sink mode);
//! - Caps Lock / Num Lock through a low-level keyboard hook, installed only while wanted;
//! - `TaskbarCreated` (Explorer restarted) and `WM_DISPLAYCHANGE`.
//!
//! Callbacks only, no timers except a one-shot after a lock key.

use std::cell::RefCell;
use std::hash::{DefaultHasher, Hash, Hasher};

use windows::core::{w, GUID};
use windows::Win32::Devices::PortableDevices::GUID_DEVINTERFACE_WPD;
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Ioctl::GUID_DEVINTERFACE_VOLUME;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::{RegisterPowerSettingNotification, POWERBROADCAST_SETTING};
use windows::Win32::System::RemoteDesktop::{WTSRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION};
use windows::Win32::System::SystemServices::{GUID_ACDC_POWER_SOURCE, GUID_BATTERY_PERCENTAGE_REMAINING, GUID_POWER_SAVING_STATUS, GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_NOREPEAT, VK_CAPITAL, VK_NUMLOCK};
use windows::Win32::UI::Input::{GetRawInputData, RegisterRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RIDEV_INPUTSINK, RID_INPUT, RIM_TYPEHID};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, KillTimer, PostMessageW, RegisterClassW, RegisterDeviceNotificationW, RegisterWindowMessageW, SetTimer,
    SetWindowsHookExW, UnhookWindowsHookEx, DBT_DEVICEARRIVAL, DBT_DEVICEREMOVECOMPLETE, DBT_DEVTYP_DEVICEINTERFACE, DEVICE_NOTIFY_WINDOW_HANDLE, DEV_BROADCAST_DEVICEINTERFACE_W,
    DEV_BROADCAST_HDR, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, MSG, PBT_APMRESUMEAUTOMATIC, PBT_POWERSETTINGCHANGE, WH_KEYBOARD_LL, WM_APP, WM_DEVICECHANGE, WM_DISPLAYCHANGE, WM_INPUT, WM_KEYUP,
    WM_POWERBROADCAST, WM_HOTKEY, WM_SYSKEYUP, WM_TIMECHANGE, WM_TIMER, WM_WTSSESSION_CHANGE, WNDCLASSW, WS_EX_TOOLWINDOW, WS_POPUP, WTS_SESSION_UNLOCK,
};

use super::{dispatch, dispatch_setting, LockKey, Setting, SystemEvent};

const WM_LOCK_HOOK: u32 = WM_APP + 1;
/// wparam: the hotkey's id; lparam: modifiers << 16 | virtual key, or 0 to remove it.
const WM_SET_HOTKEY: u32 = WM_APP + 2;
const LOCK_TIMER: usize = 1;
/// After a lock key's key-up, when its toggle state has settled.
const LOCK_SETTLE_MS: u32 = 30;

struct State {
    hwnd: HWND,
    taskbar_created: u32,
    hook: Option<HHOOK>,
    pending_lock: Option<LockKey>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|s| s.try_borrow_mut().ok().and_then(|mut s| s.as_mut().map(f)))
}

/// Starts the window thread and waits until it's registered for everything. The thread
/// runs for the rest of the process (registrations end with it).
pub(super) fn start() -> Option<isize> {
    let (ready_tx, ready) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new().name("reui-system-events".into()).spawn(move || unsafe {
        let Some(hwnd) = create_window() else {
            eprintln!("system events: can't create the window; brightness, power and drive events won't arrive");
            _ = ready_tx.send(None);
            return;
        };
        let taskbar_created = RegisterWindowMessageW(w!("TaskbarCreated"));
        STATE.with(|s| *s.borrow_mut() = Some(State { hwnd, taskbar_created, hook: None, pending_lock: None }));
        register(hwnd);
        _ = ready_tx.send(Some(hwnd.0 as isize));
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            DispatchMessageW(&msg);
        }
    });
    if spawned.is_err() {
        return None;
    }
    ready.recv().ok().flatten()
}

/// Every registration is best effort: one failing only loses its own events.
unsafe fn register(hwnd: HWND) {
    for guid in [&GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS, &GUID_POWER_SAVING_STATUS, &GUID_ACDC_POWER_SOURCE, &GUID_BATTERY_PERCENTAGE_REMAINING] {
        if let Err(e) = RegisterPowerSettingNotification(HANDLE(hwnd.0), guid, DEVICE_NOTIFY_WINDOW_HANDLE) {
            eprintln!("system events: can't watch power setting {guid:?} ({e})");
        }
    }
    let consumer = RAWINPUTDEVICE { usUsagePage: 0x0C, usUsage: 0x01, dwFlags: RIDEV_INPUTSINK, hwndTarget: hwnd };
    if let Err(e) = RegisterRawInputDevices(&[consumer], size_of::<RAWINPUTDEVICE>() as u32) {
        eprintln!("system events: can't watch brightness keys ({e}); every brightness change will show");
    }
    if let Err(e) = WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) {
        eprintln!("system events: can't watch session unlocks ({e})");
    }
    for class in [GUID_DEVINTERFACE_VOLUME, GUID_DEVINTERFACE_WPD] {
        let filter = DEV_BROADCAST_DEVICEINTERFACE_W {
            dbcc_size: size_of::<DEV_BROADCAST_DEVICEINTERFACE_W>() as u32,
            dbcc_devicetype: DBT_DEVTYP_DEVICEINTERFACE.0,
            dbcc_classguid: class,
            ..Default::default()
        };
        if let Err(e) = RegisterDeviceNotificationW(HANDLE(hwnd.0), (&filter as *const DEV_BROADCAST_DEVICEINTERFACE_W).cast(), DEVICE_NOTIFY_WINDOW_HANDLE) {
            eprintln!("system events: can't watch device arrivals {class:?} ({e})");
        }
    }
}

/// Watches the lock keys while `on`.
pub(super) fn set_lock_hook(hwnd: isize, on: bool) {
    _ = unsafe { PostMessageW(Some(HWND(hwnd as *mut _)), WM_LOCK_HOOK, WPARAM(usize::from(on)), LPARAM(0)) };
}

/// Registers (or with `None` removes) the hotkey `id` on the window's thread.
pub(super) fn set_hotkey(hwnd: isize, id: i32, key: Option<(u32, u32)>) {
    let packed = key.map_or(0, |(modifiers, vk)| (modifiers as isize) << 16 | (vk as isize & 0xFFFF));
    _ = unsafe { PostMessageW(Some(HWND(hwnd as *mut _)), WM_SET_HOTKEY, WPARAM(id as usize), LPARAM(packed)) };
}

unsafe fn create_window() -> Option<HWND> {
    let instance = GetModuleHandleW(None).ok()?;
    let class = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: instance.into(), lpszClassName: w!("ReUI.SystemEvents"), ..Default::default() };
    RegisterClassW(&class);
    // Never shown: a hidden top-level tool window (message-only windows miss broadcasts).
    CreateWindowExW(WS_EX_TOOLWINDOW, w!("ReUI.SystemEvents"), w!("ReUI system events"), WS_POPUP, 0, 0, 0, 0, None, None, Some(instance.into()), None).ok()
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

fn setting_for(guid: &GUID) -> Option<Setting> {
    match *guid {
        g if g == GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS => Some(Setting::Brightness),
        g if g == GUID_POWER_SAVING_STATUS => Some(Setting::BatterySaver),
        g if g == GUID_ACDC_POWER_SOURCE => Some(Setting::PowerSource),
        g if g == GUID_BATTERY_PERCENTAGE_REMAINING => Some(Setting::BatteryPercent),
        _ => None,
    }
}

/// A portable-device interface path → whether it's an Apple device. `None` for the
/// portable devices Windows makes for removable storage (`WPDBUSENUM`): drives have their
/// own moment.
pub fn portable_device(path: &str) -> Option<bool> {
    let path = path.to_ascii_lowercase();
    if path.contains("wpdbusenum") {
        return None;
    }
    Some(path.contains("vid_05ac"))
}

fn path_hash(path: &str) -> u64 {
    let mut h = DefaultHasher::new();
    path.to_ascii_lowercase().hash(&mut h);
    h.finish()
}

unsafe fn device_change(arrived: bool, lparam: LPARAM) {
    if lparam.0 == 0 {
        return;
    }
    let header = &*(lparam.0 as *const DEV_BROADCAST_HDR);
    if header.dbch_devicetype != DBT_DEVTYP_DEVICEINTERFACE || (header.dbch_size as usize) < size_of::<DEV_BROADCAST_DEVICEINTERFACE_W>() {
        return;
    }
    let iface = &*(lparam.0 as *const DEV_BROADCAST_DEVICEINTERFACE_W);
    if iface.dbcc_classguid == GUID_DEVINTERFACE_VOLUME {
        dispatch(SystemEvent::Volume);
    } else if iface.dbcc_classguid == GUID_DEVINTERFACE_WPD {
        // The name runs to a NUL within the structure's size.
        let max = (header.dbch_size as usize - std::mem::offset_of!(DEV_BROADCAST_DEVICEINTERFACE_W, dbcc_name)) / 2;
        let name = std::slice::from_raw_parts(iface.dbcc_name.as_ptr(), max);
        let len = name.iter().position(|&c| c == 0).unwrap_or(max);
        let path = String::from_utf16_lossy(&name[..len]);
        if let Some(apple) = portable_device(&path) {
            dispatch(SystemEvent::PortableDevice { arrived, apple, id: path_hash(&path) });
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_POWERBROADCAST => {
            let kind = wparam.0 as u32;
            if kind == PBT_POWERSETTINGCHANGE && lparam.0 != 0 {
                if let Some((guid, value)) = setting_value(&*(lparam.0 as *const POWERBROADCAST_SETTING)) {
                    if let Some(setting) = setting_for(&guid) {
                        dispatch_setting(setting, value);
                    }
                }
            } else if kind == PBT_APMRESUMEAUTOMATIC {
                dispatch(SystemEvent::Resumed);
            }
            LRESULT(1)
        }
        WM_WTSSESSION_CHANGE => {
            if wparam.0 as u32 == WTS_SESSION_UNLOCK {
                dispatch(SystemEvent::Unlocked);
            }
            LRESULT(0)
        }
        WM_DEVICECHANGE => {
            match wparam.0 as u32 {
                DBT_DEVICEARRIVAL => device_change(true, lparam),
                DBT_DEVICEREMOVECOMPLETE => device_change(false, lparam),
                _ => {}
            }
            LRESULT(1)
        }
        WM_INPUT => {
            if raw_input_has_brightness_key(HRAWINPUT(lparam.0 as *mut _)) {
                dispatch(SystemEvent::BrightnessKey);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_DISPLAYCHANGE => {
            dispatch(SystemEvent::DisplaysChanged);
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_TIMECHANGE => {
            dispatch(SystemEvent::TimeChanged);
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_SET_HOTKEY => {
            let id = wparam.0 as i32;
            _ = UnregisterHotKey(Some(hwnd), id);
            let packed = lparam.0;
            if packed != 0 {
                let (modifiers, vk) = ((packed >> 16) as u32, (packed & 0xFFFF) as u32);
                if RegisterHotKey(Some(hwnd), id, HOT_KEY_MODIFIERS(modifiers) | MOD_NOREPEAT, vk).is_err() {
                    dispatch(SystemEvent::HotkeyTaken { id });
                }
            }
            LRESULT(0)
        }
        WM_HOTKEY => {
            dispatch(SystemEvent::Hotkey { id: wparam.0 as i32 });
            LRESULT(0)
        }
        WM_LOCK_HOOK => {
            with_state(|s| match (wparam.0 != 0, s.hook) {
                (true, None) => match SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), GetModuleHandleW(None).ok().map(Into::into), 0) {
                    Ok(hook) => s.hook = Some(hook),
                    Err(e) => eprintln!("system events: can't watch the lock keys ({e})"),
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
                dispatch(SystemEvent::Lock { key, on });
            }
            LRESULT(0)
        }
        _ => {
            if msg != 0 && with_state(|s| s.taskbar_created == msg).unwrap_or(false) {
                dispatch(SystemEvent::TaskbarCreated);
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
        assert_eq!(setting_for(&GUID_ACDC_POWER_SOURCE), Some(Setting::PowerSource));
        assert_eq!(setting_for(&GUID::zeroed()), None);
        full.len = 0;
        assert_eq!(setting_value(setting(&full)), None);
    }

    #[test]
    fn system_events_portable_devices() {
        assert_eq!(portable_device(r"\\?\USB#VID_05AC&PID_12A8#00008110000A#{6ac27878-a6fa-4155-ba85-f98f491d4f33}"), Some(true));
        assert_eq!(portable_device(r"\\?\usb#vid_04e8&pid_6860&ms_comp_mtp&samsung_android#6&1#0000#{6ac27878-a6fa-4155-ba85-f98f491d4f33}"), Some(false));
        // Windows' portable device for a USB stick's volume: drives have their own moment.
        assert_eq!(portable_device(r"\\?\SWD#WPDBUSENUM#_??_USBSTOR#DISK&VEN_X#1#{6ac27878-a6fa-4155-ba85-f98f491d4f33}"), None);
        assert_eq!(path_hash("ABC"), path_hash("abc"));
    }
}
