//! Core Audio for Now Playing: which process a player is, its own volume, the system
//! volume, and output devices. Every call is blocking COM (it initialises COM on the calling
//! thread itself), so modules call these from `spawn_blocking`. Nothing here panics: failures
//! come back as `None` / `false` / empty, and the caller falls back (system volume, Sound
//! settings).

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::sync::{mpsc, OnceLock};
use std::thread::JoinHandle;

use tokio::sync::mpsc::UnboundedSender;
use windows::core::{Interface, GUID, PCWSTR, PWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, PROPERTYKEY};
use windows::Win32::Media::Audio::Endpoints::{IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl};
use windows::Win32::Media::Audio::{
    eCommunications, eConsole, eMultimedia, eRender, AudioSessionStateActive, AudioSessionStateExpired, EDataFlow, ERole,
    IAudioSessionControl2, IAudioSessionManager2, IMMDevice, IMMDeviceEnumerator, IMMNotificationClient, IMMNotificationClient_Impl,
    ISimpleAudioVolume, MMDeviceEnumerator, AUDIO_VOLUME_NOTIFICATION_DATA, DEVICE_STATE, DEVICE_STATE_ACTIVE,
};
use windows::Win32::Storage::Packaging::Appx::GetPackageFamilyName;
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PropVariantToStringAlloc};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ};
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
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

// ---- process matching -------------------------------------------------------------------

/// A process from the ToolHelp snapshot; `exe` is the lowercase file name without ".exe".
#[derive(Debug, Clone, PartialEq)]
struct Proc {
    pid: u32,
    parent: u32,
    exe: String,
}

fn exe_stem(name: &str) -> String {
    let name = name.rsplit(['\\', '/']).next().unwrap_or(name).to_ascii_lowercase();
    name.strip_suffix(".exe").map(str::to_owned).unwrap_or(name)
}

fn processes() -> Vec<Proc> {
    let Ok(snap) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else { return Vec::new() };
    let mut out = Vec::new();
    let mut e = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
    let mut ok = unsafe { Process32FirstW(snap, &mut e) }.is_ok();
    while ok {
        let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
        out.push(Proc { pid: e.th32ProcessID, parent: e.th32ParentProcessID, exe: exe_stem(&String::from_utf16_lossy(&e.szExeFile[..len])) });
        ok = unsafe { Process32NextW(snap, &mut e) }.is_ok();
    }
    _ = unsafe { CloseHandle(snap) };
    out
}

/// Gecko browsers register a 16-hex install hash as their AUMID instead of a name.
const GECKO: &[&str] = &["firefox", "librewolf", "waterfox", "zen", "floorp"];

/// The package family of a packaged AUMID ("Family_publisherid!App"), `None` for Win32 ones.
fn package_family(aumid: &str) -> Option<&str> {
    let (family, _) = aumid.split_once('!')?;
    family.contains('_').then_some(family)
}

/// Exe stems a GSMTC AUMID may be running as, most likely first. Win32 AUMIDs are usually
/// the exe ("Spotify.exe", "chrome", "MSEdge"), sometimes a known-folder path
/// ("{GUID}\VideoLAN\VLC\vlc.exe") or a dotted id ("com.squirrel.Discord.Discord",
/// "Chrome._crx_…" for PWAs). For packaged ones the app id after '!' is a last resort.
fn exe_candidates(aumid: &str) -> Vec<String> {
    let aumid = aumid.trim();
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: &str| {
        let s = exe_stem(s);
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
    };
    if aumid.len() == 16 && aumid.bytes().all(|b| b.is_ascii_hexdigit()) {
        GECKO.iter().for_each(|g| push(g));
        return out;
    }
    if let Some((_, app)) = aumid.split_once('!') {
        if !app.eq_ignore_ascii_case("app") {
            push(app);
        }
        return out;
    }
    let stem = exe_stem(aumid);
    push(&stem);
    if let Some((_, last)) = stem.rsplit_once('.') {
        push(last);
    }
    if let Some((first, _)) = stem.split_once('.') {
        push(first);
    }
    out
}

/// The root of the largest tree of matching processes: a match whose parent isn't itself a
/// match (Chrome's browser process, not its renderers). Ties go to the oldest-looking pid.
fn tree_root(procs: &[Proc], is_match: impl Fn(&Proc) -> bool) -> Option<u32> {
    let matching: HashSet<u32> = procs.iter().filter(|p| is_match(p)).map(|p| p.pid).collect();
    matching
        .iter()
        .copied()
        .filter(|&pid| procs.iter().find(|p| p.pid == pid).is_some_and(|p| p.parent == pid || !matching.contains(&p.parent)))
        .map(|root| (descendants(procs, root).iter().filter(|d| matching.contains(d)).count(), root))
        .max_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)))
        .map(|(_, root)| root)
}

/// `root` and every process below it.
fn descendants(procs: &[Proc], root: u32) -> HashSet<u32> {
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for p in procs.iter().filter(|p| p.pid != p.parent) {
        children.entry(p.parent).or_default().push(p.pid);
    }
    let mut seen = HashSet::from([root]);
    let mut stack = vec![root];
    while let Some(pid) = stack.pop() {
        for &c in children.get(&pid).into_iter().flatten() {
            if seen.insert(c) {
                stack.push(c);
            }
        }
    }
    seen
}

fn process_family(pid: u32) -> Option<String> {
    let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buf = [0u16; 256];
    let mut len = buf.len() as u32;
    let rc = unsafe { GetPackageFamilyName(h, &mut len, Some(PWSTR(buf.as_mut_ptr()))) };
    _ = unsafe { CloseHandle(h) };
    (rc == ERROR_SUCCESS).then(|| String::from_utf16_lossy(&buf[..(len as usize).saturating_sub(1).min(buf.len())]))
}

/// The root process of a player from its GSMTC `SourceAppUserModelId` ("Spotify.exe",
/// "Chrome", "SpotifyAB.SpotifyMusic_…!Spotify"). For browsers, the root browser process,
/// so its audio child is included in process-tree capture.
pub fn process_for(aumid: &str) -> Option<u32> {
    let procs = processes();
    if let Some(family) = package_family(aumid) {
        // Only packaged processes answer; everything else (and protected processes) fails fast.
        let families: HashMap<u32, String> = procs.iter().filter(|p| p.pid > 4).filter_map(|p| Some((p.pid, process_family(p.pid)?))).collect();
        let hit = tree_root(&procs, |p| families.get(&p.pid).is_some_and(|f| f.eq_ignore_ascii_case(family)));
        if hit.is_some() {
            return hit;
        }
    }
    exe_candidates(aumid).iter().find_map(|exe| tree_root(&procs, |p| &p.exe == exe))
}

// ---- per-app volume ---------------------------------------------------------------------

struct Session {
    volume: ISimpleAudioVolume,
    pid: u32,
    active: bool,
    on_default: bool,
}

/// Audio sessions on every active output belonging to `pid` or its descendants (browsers
/// play from a child audio-service process).
fn sessions_for(pid: u32) -> Vec<Session> {
    if pid == 0 {
        return Vec::new(); // the system-sounds session
    }
    let tree = descendants(&processes(), pid);
    let Some(en) = enumerator() else { return Vec::new() };
    let default_id = default_device(&en).and_then(|d| device_id(&d));
    let mut out = Vec::new();
    for dev in active_devices(&en) {
        let on_default = default_id.is_some() && device_id(&dev) == default_id;
        let Ok(mgr) = (unsafe { dev.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) }) else { continue };
        let Ok(list) = (unsafe { mgr.GetSessionEnumerator() }) else { continue };
        for i in 0..unsafe { list.GetCount() }.unwrap_or(0) {
            let Ok(ctl) = (unsafe { list.GetSession(i) }) else { continue };
            let Ok(ctl2) = ctl.cast::<IAudioSessionControl2>() else { continue };
            let Ok(spid) = (unsafe { ctl2.GetProcessId() }) else { continue };
            if !tree.contains(&spid) {
                continue;
            }
            let state = unsafe { ctl.GetState() }.unwrap_or(AudioSessionStateExpired);
            if state == AudioSessionStateExpired {
                continue;
            }
            if let Ok(volume) = ctl.cast::<ISimpleAudioVolume>() {
                out.push(Session { volume, pid: spid, active: state == AudioSessionStateActive, on_default });
            }
        }
    }
    out
}

/// The player's audio session (volume, mute), matching `pid` or its child processes. Of
/// several, the one playing on the default output, then the loudest.
pub fn app_audio(pid: u32) -> Option<AppAudio> {
    sessions_for(pid)
        .iter()
        .filter_map(|s| {
            let volume = unsafe { s.volume.GetMasterVolume() }.ok()?;
            let muted = unsafe { s.volume.GetMute() }.ok()?.as_bool();
            Some(((s.active, s.on_default), AppAudio { pid: s.pid, volume, muted }))
        })
        .max_by(|a, b| a.0.cmp(&b.0).then(a.1.volume.total_cmp(&b.1.volume)))
        .map(|(_, a)| a)
}

/// Sets every matching session, so a browser's tabs and audio child stay in step.
pub fn set_app_volume(pid: u32, volume: f32) -> bool {
    let v = if volume.is_finite() { volume.clamp(0.0, 1.0) } else { return false };
    sessions_for(pid).iter().fold(false, |ok, s| unsafe { s.volume.SetMasterVolume(v, &EVENT_CONTEXT) }.is_ok() | ok)
}

pub fn set_app_muted(pid: u32, muted: bool) -> bool {
    sessions_for(pid).iter().fold(false, |ok, s| unsafe { s.volume.SetMute(muted, &EVENT_CONTEXT) }.is_ok() | ok)
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

// ---- change notifications ---------------------------------------------------------------

enum Cmd {
    /// The default output changed: move the volume callback to the new one.
    Rebind,
    Stop,
}

#[windows_core::implement(IMMNotificationClient)]
struct DeviceEvents {
    changed: UnboundedSender<()>,
    cmd: mpsc::Sender<Cmd>,
}

/// Runs on an audio-service thread where Core Audio calls can deadlock, so it only signals;
/// the watch thread does the re-registering.
impl IMMNotificationClient_Impl for DeviceEvents_Impl {
    fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> windows_core::Result<()> {
        _ = self.changed.send(());
        Ok(())
    }
    fn OnDeviceAdded(&self, _: &PCWSTR) -> windows_core::Result<()> {
        _ = self.changed.send(());
        Ok(())
    }
    fn OnDeviceRemoved(&self, _: &PCWSTR) -> windows_core::Result<()> {
        _ = self.changed.send(());
        Ok(())
    }
    fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, _: &PCWSTR) -> windows_core::Result<()> {
        // Fires once per role; follow the one we report.
        if flow == eRender && role == ROLE {
            _ = self.cmd.send(Cmd::Rebind);
        }
        Ok(())
    }
    /// Very chatty (levels, formats); nothing we show depends on it.
    fn OnPropertyValueChanged(&self, _: &PCWSTR, _: &PROPERTYKEY) -> windows_core::Result<()> {
        Ok(())
    }
}

#[windows_core::implement(IAudioEndpointVolumeCallback)]
struct VolumeEvents(UnboundedSender<()>);

impl IAudioEndpointVolumeCallback_Impl for VolumeEvents_Impl {
    fn OnNotify(&self, _: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> windows_core::Result<()> {
        _ = self.0.send(());
        Ok(())
    }
}

/// Keeps the device and volume callbacks registered; dropping it unregisters them. The COM
/// objects live on a small thread of their own so re-registering after a default-device
/// change never happens inside a Core Audio callback.
pub struct Watch {
    cmd: mpsc::Sender<Cmd>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Watch {
    fn drop(&mut self) {
        _ = self.cmd.send(Cmd::Stop);
        if let Some(t) = self.thread.take() {
            _ = t.join();
        }
    }
}

/// Calls `changed` when the default device, the device list or the system volume changes.
/// Keep the returned value alive to keep watching.
pub fn watch(changed: UnboundedSender<()>) -> Option<Watch> {
    let (cmd, cmd_rx) = mpsc::channel();
    let (ready_tx, ready) = mpsc::channel();
    let worker_cmd = cmd.clone();
    let thread = std::thread::Builder::new()
        .name("reui-audio-watch".into())
        .spawn(move || run_watch(changed, worker_cmd, cmd_rx, ready_tx))
        .ok()?;
    if ready.recv().unwrap_or(false) {
        Some(Watch { cmd, thread: Some(thread) })
    } else {
        _ = thread.join();
        None
    }
}

fn bind_volume(en: &IMMDeviceEnumerator, cb: &IAudioEndpointVolumeCallback) -> Option<IAudioEndpointVolume> {
    let dev = default_device(en)?;
    let ev = unsafe { dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) }.ok()?;
    unsafe { ev.RegisterControlChangeNotify(cb) }.ok()?;
    Some(ev)
}

fn unbind_volume(ev: Option<IAudioEndpointVolume>, cb: &IAudioEndpointVolumeCallback) {
    if let Some(ev) = ev {
        _ = unsafe { ev.UnregisterControlChangeNotify(cb) };
    }
}

fn run_watch(changed: UnboundedSender<()>, cmd: mpsc::Sender<Cmd>, cmd_rx: mpsc::Receiver<Cmd>, ready: mpsc::Sender<bool>) {
    let Some(en) = enumerator() else {
        eprintln!("audio: can't watch output devices");
        _ = ready.send(false);
        return;
    };
    let client: IMMNotificationClient = DeviceEvents { changed: changed.clone(), cmd }.into();
    if unsafe { en.RegisterEndpointNotificationCallback(&client) }.is_err() {
        eprintln!("audio: can't watch output devices");
        _ = ready.send(false);
        return;
    }
    _ = ready.send(true);
    let cb: IAudioEndpointVolumeCallback = VolumeEvents(changed.clone()).into();
    let mut bound = bind_volume(&en, &cb);
    while let Ok(c) = cmd_rx.recv() {
        let mut stop = matches!(c, Cmd::Stop);
        // Several roles change together; one rebind covers them.
        while let Ok(more) = cmd_rx.try_recv() {
            stop |= matches!(more, Cmd::Stop);
        }
        if stop {
            break;
        }
        unbind_volume(bound.take(), &cb);
        bound = bind_volume(&en, &cb);
        _ = changed.send(());
    }
    unbind_volume(bound, &cb);
    _ = unsafe { en.UnregisterEndpointNotificationCallback(&client) };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn procs(list: &[(u32, u32, &str)]) -> Vec<Proc> {
        list.iter().map(|&(pid, parent, exe)| Proc { pid, parent, exe: exe_stem(exe) }).collect()
    }

    #[test]
    fn audio_exe_candidates() {
        assert_eq!(exe_candidates("Spotify.exe"), ["spotify"]);
        assert_eq!(exe_candidates("Chrome"), ["chrome"]);
        assert_eq!(exe_candidates("MSEdge"), ["msedge"]);
        assert_eq!(exe_candidates(r"{6D809377-6AF0-444B-8957-A3773F02200E}\VideoLAN\VLC\vlc.exe"), ["vlc"]);
        assert_eq!(exe_candidates("com.squirrel.Discord.Discord"), ["com.squirrel.discord.discord", "discord", "com"]);
        assert_eq!(exe_candidates("Chrome._crx_abcdef"), ["chrome._crx_abcdef", "_crx_abcdef", "chrome"]);
        assert_eq!(exe_candidates("308046B0AF4A39CB")[0], "firefox");
        assert_eq!(exe_candidates("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"), ["spotify"]);
        assert!(exe_candidates("Microsoft.ZuneMusic_8wekyb3d8bbwe!App").is_empty());
        assert!(exe_candidates("").is_empty());
    }

    #[test]
    fn audio_package_family() {
        assert_eq!(package_family("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"), Some("SpotifyAB.SpotifyMusic_zpdnekdrzrea0"));
        assert_eq!(package_family("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"), Some("Microsoft.ZuneMusic_8wekyb3d8bbwe"));
        assert_eq!(package_family("Spotify.exe"), None);
        assert_eq!(package_family("308046B0AF4A39CB"), None);
    }

    #[test]
    fn audio_tree_root() {
        let p = procs(&[
            (4, 0, "System"),
            (100, 4, "explorer.exe"),
            (200, 100, "chrome.exe"), // browser
            (201, 200, "chrome.exe"), // audio service
            (202, 200, "chrome.exe"),
            (300, 100, "chrome.exe"), // a second, smaller instance
            (400, 999, "Spotify.exe"),
            (401, 400, "Spotify.exe"),
        ]);
        assert_eq!(tree_root(&p, |x| x.exe == "chrome"), Some(200));
        assert_eq!(tree_root(&p, |x| x.exe == "spotify"), Some(400));
        assert_eq!(tree_root(&p, |x| x.exe == "firefox"), None);
        // A lone child whose parent exited still counts as a root.
        assert_eq!(tree_root(&procs(&[(7, 3, "vlc.exe")]), |x| x.exe == "vlc"), Some(7));
    }

    #[test]
    fn audio_descendants() {
        let p = procs(&[(1, 0, "a"), (2, 1, "b"), (3, 2, "c"), (4, 9, "d"), (0, 0, "idle")]);
        assert_eq!(descendants(&p, 1), HashSet::from([1, 2, 3]));
        assert_eq!(descendants(&p, 4), HashSet::from([4]));
    }

    #[test]
    fn audio_process_for_doesnt_panic() {
        // Read-only: a process snapshot, no devices.
        _ = process_for("explorer.exe");
        _ = process_for("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic");
        _ = process_for("308046B0AF4A39CB");
        assert_eq!(process_for("reui-no-such-player-xyz.exe"), None);
    }

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

    /// Registers and drops the callbacks (no device changes). Run with `--ignored`.
    #[test]
    #[ignore = "registers callbacks on the machine's audio devices"]
    fn audio_watch_registers() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let w = watch(tx);
        assert!(w.is_some());
        drop(w);
    }
}
