//! A player's own volume and mute: its audio sessions (and its child processes') on every
//! active output.

use windows::core::Interface;
use windows::Win32::Media::Audio::{AudioSessionStateActive, AudioSessionStateExpired, IAudioSessionControl2, IAudioSessionManager2, ISimpleAudioVolume};
use windows::Win32::System::Com::CLSCTX_ALL;

use super::process::{descendants, processes};
use super::{active_devices, default_device, device_id, enumerator, AppAudio, EVENT_CONTEXT};

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
