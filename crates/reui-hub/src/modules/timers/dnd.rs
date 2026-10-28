//! Windows' Do Not Disturb, for focus sessions (T1). Windows has no public API for it, so
//! this uses what the shell itself uses, and checks every change:
//! - **Reading:** the WNF state `WNF_SHEL_QUIETHOURS_ACTIVE_PROFILE_CHANGED` through
//!   `ntdll!NtQueryWnfStateData` (0 off, 1 priority only, 2 alarms only).
//! - **Turning it on or off:** the shell's `IQuietHoursSettings` COM object (CLSID
//!   `F53321FA-…`), `put_UserSelectedProfile`, called through its raw vtable on a
//!   short-lived STA thread. (`FocusSessionManager` would be the documented way, but
//!   starting a session is a Limited Access Feature and fails with E_ACCESSDENIED.)
//!
//! Both are undocumented and may change in any Windows build, so the module only trusts a
//! change it has read back, and stops trying for the session when one doesn't stick.

use std::ffi::c_void;

use windows::core::{s, w, Interface, GUID, HRESULT, IUnknown, PCWSTR, PWSTR};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

const WNF_SHEL_QUIETHOURS_ACTIVE_PROFILE_CHANGED: u64 = 0x0D83_063E_A3BF_1C75;
const CLSID_QUIET_HOURS_SETTINGS: GUID = GUID::from_u128(0xf53321fa_34f8_4b7f_b9a3_361877cb94cf);
const IID_QUIET_HOURS_SETTINGS: GUID = GUID::from_u128(0x6bff4732_81ec_4ffb_ae67_b6c1bc29631f);
const PROFILE_ON: &str = "Microsoft.QuietHoursProfile.PriorityOnly";
const PROFILE_OFF: &str = "Microsoft.QuietHoursProfile.Unrestricted";

type QueryWnf = unsafe extern "system" fn(state: *const u64, type_id: *const c_void, scope: *const c_void, stamp: *mut u32, buffer: *mut c_void, size: *mut u32) -> i32;

/// Whether Do Not Disturb (any profile but "off") is on now; `None` if it can't be read.
pub fn is_on() -> Option<bool> {
    unsafe {
        let ntdll = GetModuleHandleW(w!("ntdll.dll")).ok()?;
        let query: QueryWnf = std::mem::transmute(GetProcAddress(ntdll, s!("NtQueryWnfStateData"))?);
        let (mut stamp, mut value, mut size) = (0u32, 0u32, 4u32);
        let status = query(&WNF_SHEL_QUIETHOURS_ACTIVE_PROFILE_CHANGED, std::ptr::null(), std::ptr::null(), &mut stamp, (&mut value as *mut u32).cast(), &mut size);
        (status >= 0).then_some(size >= 4 && value != 0)
    }
}

/// Asks Windows to turn Do Not Disturb on or off. Blocks (COM, a thread of its own): call
/// it off the hub thread, then [`is_on`] a moment later to see if it took.
pub fn set(on: bool) -> Result<(), String> {
    std::thread::spawn(move || unsafe {
        let init = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let result = put_profile(if on { PROFILE_ON } else { PROFILE_OFF });
        if init.is_ok() {
            CoUninitialize();
        }
        result
    })
    .join()
    .map_err(|_| "panicked".to_string())?
}

/// `IQuietHoursSettings` after IUnknown: `get_UserSelectedProfile`, `put_UserSelectedProfile`.
#[repr(C)]
struct QuietHoursVtbl {
    unknown: [usize; 3],
    get_profile: unsafe extern "system" fn(this: *mut c_void, profile: *mut PWSTR) -> HRESULT,
    put_profile: unsafe extern "system" fn(this: *mut c_void, profile: PCWSTR) -> HRESULT,
}

unsafe fn put_profile(profile: &str) -> Result<(), String> {
    let unknown: IUnknown = CoCreateInstance(&CLSID_QUIET_HOURS_SETTINGS, None, CLSCTX_LOCAL_SERVER).map_err(|e| format!("not available ({e})"))?;
    let mut raw = std::ptr::null_mut();
    unknown.query(&IID_QUIET_HOURS_SETTINGS, &mut raw).ok().map_err(|e| format!("interface missing ({e})"))?;
    // SAFETY: `raw` is an IQuietHoursSettings we own one reference to; dropped below.
    let settings = IUnknown::from_raw(raw);
    let vtbl = &**(raw as *const *const QuietHoursVtbl);
    // Read first: a sanity check that the slots are what we think before writing to one.
    let mut current = PWSTR::null();
    (vtbl.get_profile)(raw, &mut current).ok().map_err(|e| format!("can't read the profile ({e})"))?;
    let looks_right = !current.is_null() && current.to_string().is_ok_and(|p| p.starts_with("Microsoft.QuietHoursProfile."));
    if !current.is_null() {
        CoTaskMemFree(Some(current.0 as *const c_void));
    }
    if !looks_right {
        return Err("unexpected profile interface".into());
    }
    let wide: Vec<u16> = profile.encode_utf16().chain([0]).collect();
    let result = (vtbl.put_profile)(raw, PCWSTR(wide.as_ptr())).ok().map_err(|e| format!("refused ({e})"));
    drop(settings);
    result
}

#[cfg(test)]
mod tests {
    /// Reads, turns DND on, checks, and puts it back off: changes the machine's real
    /// setting for a moment, so it only runs by hand (`cargo test -- --ignored dnd`), and
    /// only when DND starts off.
    #[test]
    #[ignore]
    fn dnd_round_trip_on_this_machine() {
        let before = super::is_on().expect("WNF state readable");
        println!("DND before: {before}");
        if before {
            return;
        }
        super::set(true).expect("turned on");
        std::thread::sleep(std::time::Duration::from_millis(600));
        let during = super::is_on();
        super::set(false).expect("turned off");
        std::thread::sleep(std::time::Duration::from_millis(600));
        let after = super::is_on();
        println!("DND during: {during:?}, after: {after:?}");
        assert_eq!((during, after), (Some(true), Some(false)));
    }

    #[test]
    #[ignore]
    fn dnd_reads() {
        println!("DND now: {:?}", super::is_on());
    }
}
