//! Every app's audio session on the default output, live, for the volume card's mixer.
//! A thread of its own holds the Core Audio objects: it registers for new sessions
//! (`IAudioSessionNotification`) and for each session's volume, mute and state
//! (`IAudioSessionEvents`), and on any of them reads the list again and sends it. Nothing
//! is polled. Callbacks run on audio-service threads where Core Audio calls can deadlock,
//! so they only signal; the thread does the reading, and the levels set from the card.
//!
//! Sessions are grouped by the exe that plays them (a browser's tabs share its audio
//! process; an app may open several sessions), so one app is one line and setting it sets
//! all of its sessions. ReUI's own (the timer's finish sound) and Windows' system sounds
//! are left out.

use std::collections::HashMap;
use std::sync::mpsc;
use std::thread::JoinHandle;

use tokio::sync::mpsc::UnboundedSender;
use windows::core::{Interface, BOOL, GUID, PCWSTR, PWSTR};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Media::Audio::{
    AudioSessionDisconnectReason, AudioSessionState, AudioSessionStateActive, AudioSessionStateExpired, IAudioSessionControl, IAudioSessionControl2, IAudioSessionEvents, IAudioSessionEvents_Impl,
    IAudioSessionManager2, IAudioSessionNotification, IAudioSessionNotification_Impl, ISimpleAudioVolume,
};
use windows::Win32::System::Com::CLSCTX_ALL;
use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};

use super::{default_device, enumerator, take_pwstr, EVENT_CONTEXT};

/// One app's sessions, as the mixer shows them.
#[derive(Debug, Clone, PartialEq)]
pub struct AppSession {
    /// Stable for the app (a hash of its exe path), so a line keeps its identity while its
    /// sessions come and go.
    pub key: u32,
    /// Full exe path, for the name and icon.
    pub exe: String,
    /// 0..=1: the loudest of its sessions.
    pub volume: f32,
    /// Only when every session is muted.
    pub muted: bool,
    /// Something is playing now (any session active).
    pub active: bool,
}

/// The key for an exe path: FNV-1a over it, case-folded (Windows paths).
pub fn app_key(exe: &str) -> u32 {
    let hash = exe.to_lowercase().bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3));
    hash as u32
}

/// Sessions read one by one, merged per app. Pure.
pub fn group(sessions: impl IntoIterator<Item = (String, f32, bool, bool)>) -> Vec<AppSession> {
    let mut out: Vec<AppSession> = Vec::new();
    for (exe, volume, muted, active) in sessions {
        let key = app_key(&exe);
        match out.iter_mut().find(|a| a.key == key) {
            Some(a) => {
                a.volume = a.volume.max(volume);
                a.muted &= muted;
                a.active |= active;
            }
            None => out.push(AppSession { key, exe, volume, muted, active }),
        }
    }
    out
}

/// ReUI's own programs (the hub plays the timer's sound) never show.
fn own(exe: &str) -> bool {
    let file = exe.rsplit(['\\', '/']).next().unwrap_or(exe).to_ascii_lowercase();
    file.starts_with("reui-")
}

enum Cmd {
    Changed,
    Volume(u32, f32),
    Muted(u32, bool),
    Stop,
}

#[windows_core::implement(IAudioSessionNotification)]
struct NewSessions {
    cmd: mpsc::Sender<Cmd>,
}

impl IAudioSessionNotification_Impl for NewSessions_Impl {
    fn OnSessionCreated(&self, _: windows_core::Ref<IAudioSessionControl>) -> windows_core::Result<()> {
        _ = self.cmd.send(Cmd::Changed);
        Ok(())
    }
}

#[windows_core::implement(IAudioSessionEvents)]
struct SessionEvents {
    cmd: mpsc::Sender<Cmd>,
}

impl IAudioSessionEvents_Impl for SessionEvents_Impl {
    fn OnDisplayNameChanged(&self, _: &PCWSTR, _: *const GUID) -> windows_core::Result<()> {
        Ok(())
    }
    fn OnIconPathChanged(&self, _: &PCWSTR, _: *const GUID) -> windows_core::Result<()> {
        Ok(())
    }
    fn OnSimpleVolumeChanged(&self, _: f32, _: BOOL, _: *const GUID) -> windows_core::Result<()> {
        _ = self.cmd.send(Cmd::Changed);
        Ok(())
    }
    fn OnChannelVolumeChanged(&self, _: u32, _: *const f32, _: u32, _: *const GUID) -> windows_core::Result<()> {
        Ok(())
    }
    fn OnGroupingParamChanged(&self, _: *const GUID, _: *const GUID) -> windows_core::Result<()> {
        Ok(())
    }
    fn OnStateChanged(&self, _: AudioSessionState) -> windows_core::Result<()> {
        _ = self.cmd.send(Cmd::Changed);
        Ok(())
    }
    fn OnSessionDisconnected(&self, _: AudioSessionDisconnectReason) -> windows_core::Result<()> {
        _ = self.cmd.send(Cmd::Changed);
        Ok(())
    }
}

/// Watching the default output's sessions; dropping it unregisters everything. Set levels
/// through it: the thread already holds the sessions.
pub struct Sessions {
    cmd: mpsc::Sender<Cmd>,
    thread: Option<JoinHandle<()>>,
}

impl Sessions {
    pub fn set_volume(&self, key: u32, volume: f32) {
        if volume.is_finite() {
            _ = self.cmd.send(Cmd::Volume(key, volume.clamp(0.0, 1.0)));
        }
    }

    pub fn set_muted(&self, key: u32, muted: bool) {
        _ = self.cmd.send(Cmd::Muted(key, muted));
    }
}

impl Drop for Sessions {
    fn drop(&mut self) {
        _ = self.cmd.send(Cmd::Stop);
        if let Some(t) = self.thread.take() {
            _ = t.join();
        }
    }
}

/// Sends the default output's apps now and whenever they change. Blocks until registered
/// (call it off the hub thread); `None` without an output. Follow a default-output change
/// by watching again.
pub fn watch_sessions(apps: UnboundedSender<Vec<AppSession>>) -> Option<Sessions> {
    let (cmd, cmd_rx) = mpsc::channel();
    let (ready_tx, ready) = mpsc::channel();
    let worker_cmd = cmd.clone();
    let thread = std::thread::Builder::new().name("reui-audio-sessions".into()).spawn(move || run(apps, worker_cmd, cmd_rx, ready_tx)).ok()?;
    if ready.recv().unwrap_or(false) {
        Some(Sessions { cmd, thread: Some(thread) })
    } else {
        _ = thread.join();
        None
    }
}

/// A session being watched.
struct Watched {
    control: IAudioSessionControl,
    events: IAudioSessionEvents,
    volume: ISimpleAudioVolume,
    exe: String,
}

fn exe_path(pid: u32) -> Option<String> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buffer = [0u16; 1024];
        let mut len = buffer.len() as u32;
        let named = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &mut len);
        _ = CloseHandle(process);
        named.ok()?;
        Some(String::from_utf16_lossy(&buffer[..len as usize]))
    }
}

fn run(apps: UnboundedSender<Vec<AppSession>>, cmd: mpsc::Sender<Cmd>, cmd_rx: mpsc::Receiver<Cmd>, ready: mpsc::Sender<bool>) {
    let manager = enumerator().as_ref().and_then(default_device).and_then(|d| unsafe { d.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) }.ok());
    let Some(manager) = manager else {
        _ = ready.send(false);
        return;
    };
    let notify: IAudioSessionNotification = NewSessions { cmd: cmd.clone() }.into();
    if unsafe { manager.RegisterSessionNotification(&notify) }.is_err() {
        _ = ready.send(false);
        return;
    }
    _ = ready.send(true);
    let mut watched: HashMap<String, Watched> = HashMap::new();
    // New-session notifications start only after a first enumeration (Microsoft Learn,
    // RegisterSessionNotification), which this is.
    _ = apps.send(read(&manager, &mut watched, &cmd));
    while let Ok(first) = cmd_rx.recv() {
        let mut changed = false;
        for c in std::iter::once(first).chain(std::iter::from_fn(|| cmd_rx.try_recv().ok())) {
            match c {
                Cmd::Stop => {
                    for w in watched.into_values() {
                        _ = unsafe { w.control.UnregisterAudioSessionNotification(&w.events) };
                    }
                    _ = unsafe { manager.UnregisterSessionNotification(&notify) };
                    return;
                }
                Cmd::Changed => changed = true,
                Cmd::Volume(key, v) => {
                    for w in watched.values().filter(|w| app_key(&w.exe) == key) {
                        _ = unsafe { w.volume.SetMasterVolume(v, &EVENT_CONTEXT) };
                    }
                }
                Cmd::Muted(key, m) => {
                    for w in watched.values().filter(|w| app_key(&w.exe) == key) {
                        _ = unsafe { w.volume.SetMute(m, &EVENT_CONTEXT) };
                    }
                }
            }
        }
        // Our own sets come back as volume events, which read the list once more.
        if changed {
            _ = apps.send(read(&manager, &mut watched, &cmd));
        }
    }
}

/// The sessions now: new ones registered for events, expired ones let go.
fn read(manager: &IAudioSessionManager2, watched: &mut HashMap<String, Watched>, cmd: &mpsc::Sender<Cmd>) -> Vec<AppSession> {
    let mut seen = Vec::new();
    let mut rows = Vec::new();
    if let Ok(list) = unsafe { manager.GetSessionEnumerator() } {
        for i in 0..unsafe { list.GetCount() }.unwrap_or(0) {
            let Ok(control) = (unsafe { list.GetSession(i) }) else { continue };
            let Ok(control2) = control.cast::<IAudioSessionControl2>() else { continue };
            let state = unsafe { control.GetState() }.unwrap_or(AudioSessionStateExpired);
            // IsSystemSoundsSession is S_OK for Windows' own sounds, S_FALSE otherwise.
            if state == AudioSessionStateExpired || unsafe { control2.IsSystemSoundsSession() }.0 == 0 {
                continue;
            }
            let Some(id) = unsafe { control2.GetSessionInstanceIdentifier() }.ok().and_then(take_pwstr) else { continue };
            if !watched.contains_key(&id) {
                let pid = unsafe { control2.GetProcessId() }.unwrap_or(0);
                let Some(exe) = (pid != 0).then(|| exe_path(pid)).flatten().filter(|e| !own(e)) else { continue };
                let Ok(volume) = control.cast::<ISimpleAudioVolume>() else { continue };
                let events: IAudioSessionEvents = SessionEvents { cmd: cmd.clone() }.into();
                if unsafe { control.RegisterAudioSessionNotification(&events) }.is_err() {
                    continue;
                }
                watched.insert(id.clone(), Watched { control, events, volume, exe });
            }
            let Some(w) = watched.get(&id) else { continue };
            let volume = unsafe { w.volume.GetMasterVolume() }.unwrap_or(1.0);
            let muted = unsafe { w.volume.GetMute() }.is_ok_and(|m| m.as_bool());
            rows.push((w.exe.clone(), volume, muted, state == AudioSessionStateActive));
            seen.push(id);
        }
    }
    watched.retain(|id, w| {
        let keep = seen.contains(id);
        if !keep {
            _ = unsafe { w.control.UnregisterAudioSessionNotification(&w.events) };
        }
        keep
    });
    group(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_sessions_group_by_app() {
        let chrome = r"C:\Program Files\Google\Chrome\Application\chrome.exe";
        let apps = group([(chrome.to_owned(), 0.4, true, false), ("C:\\Spotify\\Spotify.exe".to_owned(), 1.0, false, true), (chrome.to_uppercase(), 0.8, false, true)]);
        assert_eq!(apps.len(), 2);
        assert_eq!((apps[0].volume, apps[0].muted, apps[0].active), (0.8, false, true));
        assert_eq!(apps[0].key, app_key(chrome));
        assert!(own(r"D:\ReUI\target\debug\reui-hub.exe") && !own(chrome));
    }

    /// Lists this machine's sessions (read-only). Run with `--ignored`.
    #[test]
    #[ignore = "reads the machine's audio sessions"]
    fn audio_sessions_read() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let s = watch_sessions(tx);
        assert!(s.is_some());
        println!("{:#?}", rx.blocking_recv());
    }
}
