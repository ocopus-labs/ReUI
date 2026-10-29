//! Core Audio for Now Playing and the system OSD: which process a player is, its own volume,
//! the system volume, output devices, and change notifications. Every call is blocking COM (it initialises COM on the calling
//! thread itself), so modules call these from `spawn_blocking`. Nothing here panics: failures
//! come back as `None` / `false` / empty, and the caller falls back (system volume, Sound
//! settings).
//!
//! Files: `process.rs` (which process a player is), `apps.rs` (per-app volume),
//! `events.rs` (change notifications), `sessions.rs` (every app's session, live, for the
//! mixer); devices, system volume and outputs are here.

mod apps;
mod events;
mod process;
mod sessions;

use std::ffi::c_void;
use std::sync::OnceLock;

use windows::core::{GUID, PCWSTR, PWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    eCommunications, eConsole, eMultimedia, eRender, ERole, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PropVariantToStringAlloc};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ};

pub use self::apps::{app_audio, set_app_muted, set_app_volume};
pub use self::events::{capture_muted, default_output, set_capture_muted, watch, watch_events, AudioEvent, DefaultOutput, DeviceKind, Watch};
pub use self::process::process_for;
pub use self::sessions::{watch_sessions, AppSession, Sessions};

/// A player's own audio session.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AppAudio {
    pub pid: u32,
    /// 0..=1
    pub volume: f32,
    pub muted: bool,
}

/// An active output device.
#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    pub id: String,
    pub name: String,
    pub default: bool,
}

/// Tags our own volume/mute changes so Windows' event context isn't null; other apps
/// (EarTrumpet, the mixer) see a foreign GUID and don't mistake them for their own.
const EVENT_CONTEXT: GUID = GUID::from_u128(0x5f1c0a52_9a3e_4c1b_8d2e_7b4e3a6c9d10);

/// The role whose default we report and follow. Windows' own picker sets all roles at once,
/// and media players render to the multimedia one.
const ROLE: ERole = eMultimedia;

fn com() {
    // RPC_E_CHANGED_MODE (an STA thread) is fine: Core Audio works from either apartment.
    _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
}

fn enumerator() -> Option<IMMDeviceEnumerator> {
    com();
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.ok()
}

fn default_device(en: &IMMDeviceEnumerator) -> Option<IMMDevice> {
    unsafe { en.GetDefaultAudioEndpoint(eRender, ROLE) }.ok()
}

fn active_devices(en: &IMMDeviceEnumerator) -> Vec<IMMDevice> {
    let Ok(list) = (unsafe { en.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) }) else { return Vec::new() };
    let n = unsafe { list.GetCount() }.unwrap_or(0);
    (0..n).filter_map(|i| unsafe { list.Item(i) }.ok()).collect()
}

/// Takes ownership of a CoTaskMem string.
fn take_pwstr(p: PWSTR) -> Option<String> {
    if p.is_null() {
        return None;
    }
    let s = unsafe { p.to_string() }.ok();
    unsafe { CoTaskMemFree(Some(p.0 as *const c_void)) };
    s
}

fn device_id(d: &IMMDevice) -> Option<String> {
    take_pwstr(unsafe { d.GetId() }.ok()?)
}

fn friendly_name(d: &IMMDevice) -> Option<String> {
    let store = unsafe { d.OpenPropertyStore(STGM_READ) }.ok()?;
    let mut v = unsafe { store.GetValue(&PKEY_Device_FriendlyName) }.ok()?;
    let s = unsafe { PropVariantToStringAlloc(&v) }.ok().and_then(take_pwstr);
    _ = unsafe { PropVariantClear(&mut v) };
    s.filter(|s| !s.is_empty())
}

// ---- system volume ----------------------------------------------------------------------

fn endpoint_volume() -> Option<IAudioEndpointVolume> {
    let dev = default_device(&enumerator()?)?;
    unsafe { dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) }.ok()
}

/// Default output's volume and mute.
pub fn system_audio() -> Option<(f32, bool)> {
    let ev = endpoint_volume()?;
    let volume = unsafe { ev.GetMasterVolumeLevelScalar() }.ok()?;
    let muted = unsafe { ev.GetMute() }.ok()?.as_bool();
    Some((volume, muted))
}

pub fn set_system_volume(volume: f32) -> bool {
    if !volume.is_finite() {
        return false;
    }
    endpoint_volume().is_some_and(|ev| unsafe { ev.SetMasterVolumeLevelScalar(volume.clamp(0.0, 1.0), &EVENT_CONTEXT) }.is_ok())
}

pub fn set_system_muted(muted: bool) -> bool {
    endpoint_volume().is_some_and(|ev| unsafe { ev.SetMute(muted, &EVENT_CONTEXT) }.is_ok())
}

// ---- outputs ----------------------------------------------------------------------------

/// Active render endpoints, the default marked.
pub fn outputs() -> Vec<Output> {
    let Some(en) = enumerator() else { return Vec::new() };
    let default_id = default_device(&en).and_then(|d| device_id(&d));
    active_devices(&en)
        .iter()
        .filter_map(|d| {
            let id = device_id(d)?;
            let name = friendly_name(d).unwrap_or_else(|| "Output".to_owned());
            Some(Output { default: Some(&id) == default_id.as_ref(), id, name })
        })
        .collect()
}

/// `PolicyConfigClient`: the undocumented coclass behind Sound settings' "default device",
/// also used by SoundSwitch and EarTrumpet.
const CLSID_POLICY_CONFIG: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

/// COM names are PascalCase; keep the lint quiet for the generated vtable too.
#[allow(non_snake_case)]
mod policy {
    use std::ffi::c_void;

    use windows::core::{HRESULT, PCWSTR};
    use windows::Win32::Foundation::PROPERTYKEY;
    use windows::Win32::Media::Audio::ERole;
    use windows_core::{IUnknown, IUnknown_Vtbl};

    /// Undocumented, stable since Windows 7 (IID unchanged through Windows 11 24H2). Only
    /// `SetDefaultEndpoint` is called; the rest exist to keep the vtable slots in order, so their
    /// parameter types are loose.
    #[windows_core::interface("f8679f50-850a-41cf-9c72-430f290290c8")]
    pub unsafe trait IPolicyConfig: IUnknown {
        fn GetMixFormat(&self, id: PCWSTR, format: *mut *mut c_void) -> HRESULT;
        fn GetDeviceFormat(&self, id: PCWSTR, default: i32, format: *mut *mut c_void) -> HRESULT;
        fn ResetDeviceFormat(&self, id: PCWSTR) -> HRESULT;
        fn SetDeviceFormat(&self, id: PCWSTR, endpoint: *mut c_void, mix: *mut c_void) -> HRESULT;
        fn GetProcessingPeriod(&self, id: PCWSTR, default: i32, default_period: *mut i64, min_period: *mut i64) -> HRESULT;
        fn SetProcessingPeriod(&self, id: PCWSTR, period: *mut i64) -> HRESULT;
        fn GetShareMode(&self, id: PCWSTR, mode: *mut c_void) -> HRESULT;
        fn SetShareMode(&self, id: PCWSTR, mode: *mut c_void) -> HRESULT;
        fn GetPropertyValue(&self, id: PCWSTR, fx_store: i32, key: *const PROPERTYKEY, value: *mut c_void) -> HRESULT;
        fn SetPropertyValue(&self, id: PCWSTR, fx_store: i32, key: *const PROPERTYKEY, value: *mut c_void) -> HRESULT;
        pub fn SetDefaultEndpoint(&self, id: PCWSTR, role: ERole) -> HRESULT;
        fn SetEndpointVisibility(&self, id: PCWSTR, visible: i32) -> HRESULT;
    }
}
use policy::IPolicyConfig;

fn policy_config() -> Option<IPolicyConfig> {
    com();
    unsafe { CoCreateInstance(&CLSID_POLICY_CONFIG, None, CLSCTX_ALL) }.ok()
}

/// Makes `id` the default output for all roles. `false` if that isn't possible here (the
/// caller then offers Sound settings instead).
pub fn set_default_output(id: &str) -> bool {
    let Some(pc) = policy_config() else { return false };
    let wide: Vec<u16> = id.encode_utf16().chain([0]).collect();
    let set = |role| unsafe { pc.SetDefaultEndpoint(PCWSTR(wide.as_ptr()), role) }.is_ok();
    // Console and multimedia are what playback follows; communications is best effort.
    let ok = set(eConsole) & set(eMultimedia);
    if ok && !set(eCommunications) {
        eprintln!("audio: couldn't make the output the communications default too");
    }
    ok
}

/// Whether switching outputs works on this machine (checked once).
pub fn can_switch_output() -> bool {
    static CAN: OnceLock<bool> = OnceLock::new();
    *CAN.get_or_init(|| {
        let ok = policy_config().is_some();
        if !ok {
            eprintln!("audio: output switching unavailable; the output button opens Sound settings");
        }
        ok
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads real devices (read-only; never sets anything). Run with `--ignored`.
    #[test]
    #[ignore = "reads the machine's audio devices"]
    fn audio_read_devices() {
        let outs = outputs();
        assert!(outs.iter().filter(|o| o.default).count() <= 1);
        _ = system_audio();
        _ = can_switch_output();
        for aumid in ["chrome", "MSEdge", "Spotify.exe", "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify", "308046B0AF4A39CB", "MicrosoftWindows.Client.CBS_cw5n1h2txyewy!CrossDevice", "Microsoft.Windows.StartMenuExperienceHost_cw5n1h2txyewy!App"] {
            let pid = process_for(aumid);
            println!("{aumid}: {pid:?} {:?}", pid.and_then(app_audio));
        }
        println!("{outs:#?} {:?} switch={}", system_audio(), can_switch_output());
    }
}
