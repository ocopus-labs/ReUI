//! Change notifications: default output and device changes, volume and mute (ours told
//! apart by their event context), and the microphone's mute, on a watcher thread.

use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;

use tokio::sync::mpsc::UnboundedSender;
use windows::core::{GUID, PCWSTR};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::Endpoints::{IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl};
use windows::Win32::Media::Audio::{
    eCapture, eConsole, eRender, EDataFlow, ERole, IMMDevice, IMMNotificationClient, IMMNotificationClient_Impl, AUDIO_VOLUME_NOTIFICATION_DATA, PKEY_AudioEndpoint_FormFactor, DEVICE_STATE,
};
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PropVariantToUInt32};
use windows::Win32::System::Com::{CLSCTX_ALL, STGM_READ};

use super::{default_device, device_id, enumerator, friendly_name, EVENT_CONTEXT, ROLE};

/// What kind of output a device is, from its form factor (for the OSD's device glyph).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeviceKind {
    #[default]
    Speakers,
    Headphones,
    /// Headphones with a microphone (most Bluetooth and USB headsets).
    Headset,
    /// A monitor or TV over HDMI / DisplayPort.
    Display,
    /// Line out, S/PDIF, remote and unknown devices.
    Other,
}

/// `EndpointFormFactor` → [`DeviceKind`].
pub fn kind_from_form_factor(form_factor: u32) -> DeviceKind {
    match form_factor {
        1 => DeviceKind::Speakers,
        3 => DeviceKind::Headphones,
        5 | 6 => DeviceKind::Headset,
        9 => DeviceKind::Display,
        _ => DeviceKind::Other,
    }
}

fn device_kind(d: &IMMDevice) -> DeviceKind {
    let Ok(store) = (unsafe { d.OpenPropertyStore(STGM_READ) }) else { return DeviceKind::Other };
    let Ok(mut v) = (unsafe { store.GetValue(&PKEY_AudioEndpoint_FormFactor) }) else { return DeviceKind::Other };
    let ff = unsafe { PropVariantToUInt32(&v) };
    _ = unsafe { PropVariantClear(&mut v) };
    ff.map_or(DeviceKind::Other, kind_from_form_factor)
}

/// The default output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultOutput {
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
}

fn describe(d: &IMMDevice) -> Option<DefaultOutput> {
    Some(DefaultOutput { id: device_id(d)?, name: friendly_name(d).unwrap_or_else(|| "Output".to_owned()), kind: device_kind(d) })
}

pub fn default_output() -> Option<DefaultOutput> {
    describe(&default_device(&enumerator()?)?)
}

/// The microphone role we follow: what Sound settings calls the default input.
const CAPTURE_ROLE: ERole = eConsole;

/// Whether the default microphone is muted; `None` without one.
pub fn capture_muted() -> Option<bool> {
    let dev = unsafe { enumerator()?.GetDefaultAudioEndpoint(eCapture, CAPTURE_ROLE) }.ok()?;
    let ev = unsafe { dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) }.ok()?;
    Some(unsafe { ev.GetMute() }.ok()?.as_bool())
}

/// Mutes or unmutes the default microphone (the privacy indicator's mute). `false` without
/// one. Tagged as ours, like the output's volume changes.
pub fn set_capture_muted(muted: bool) -> bool {
    let Some(dev) = enumerator().and_then(|en| unsafe { en.GetDefaultAudioEndpoint(eCapture, CAPTURE_ROLE) }.ok()) else { return false };
    let Ok(ev) = (unsafe { dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) }) else { return false };
    unsafe { ev.SetMute(muted, &EVENT_CONTEXT) }.is_ok()
}

/// A change [`watch_events`] reports.
#[derive(Debug, Clone, PartialEq)]
pub enum AudioEvent {
    /// The default output's volume or mute changed. `ours` when ReUI made the change (it
    /// carries [`EVENT_CONTEXT`]).
    Volume { level: f32, muted: bool, ours: bool },
    /// Another output became the default (the volume callback has moved to it).
    DefaultDevice { id: String, name: String, kind: DeviceKind },
    /// An output was added, removed, enabled or disabled.
    Outputs,
    /// The default microphone's mute or level changed (only when watched with `capture`).
    CaptureMute { muted: bool },
}

/// Whether a volume notification came from one of our own `Set…` calls.
pub fn is_ours(context: &GUID) -> bool {
    *context == EVENT_CONTEXT
}

fn volume_event(data: &AUDIO_VOLUME_NOTIFICATION_DATA) -> AudioEvent {
    AudioEvent::Volume { level: data.fMasterVolume.clamp(0.0, 1.0), muted: data.bMuted.as_bool(), ours: is_ours(&data.guidEventContext) }
}

type Sink = Arc<dyn Fn(AudioEvent) + Send + Sync>;

enum Cmd {
    /// The default output changed: move the volume callback to the new one.
    Rebind,
    /// The default microphone changed.
    RebindCapture,
    Stop,
}

#[windows_core::implement(IMMNotificationClient)]
struct DeviceEvents {
    sink: Sink,
    cmd: mpsc::Sender<Cmd>,
    capture: bool,
}

/// Runs on an audio-service thread where Core Audio calls can deadlock, so it only signals;
/// the watch thread does the re-registering.
impl IMMNotificationClient_Impl for DeviceEvents_Impl {
    fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> windows_core::Result<()> {
        (self.sink)(AudioEvent::Outputs);
        Ok(())
    }
    fn OnDeviceAdded(&self, _: &PCWSTR) -> windows_core::Result<()> {
        (self.sink)(AudioEvent::Outputs);
        Ok(())
    }
    fn OnDeviceRemoved(&self, _: &PCWSTR) -> windows_core::Result<()> {
        (self.sink)(AudioEvent::Outputs);
        Ok(())
    }
    fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, _: &PCWSTR) -> windows_core::Result<()> {
        // Fires once per role; follow the one we report.
        if flow == eRender && role == ROLE {
            _ = self.cmd.send(Cmd::Rebind);
        } else if self.capture && flow == eCapture && role == CAPTURE_ROLE {
            _ = self.cmd.send(Cmd::RebindCapture);
        }
        Ok(())
    }
    /// Very chatty (levels, formats); nothing we show depends on it.
    fn OnPropertyValueChanged(&self, _: &PCWSTR, _: &PROPERTYKEY) -> windows_core::Result<()> {
        Ok(())
    }
}

#[windows_core::implement(IAudioEndpointVolumeCallback)]
struct VolumeEvents {
    sink: Sink,
    capture: bool,
}

impl IAudioEndpointVolumeCallback_Impl for VolumeEvents_Impl {
    fn OnNotify(&self, data: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> windows_core::Result<()> {
        // SAFETY: Core Audio passes a valid notification for the duration of the call.
        let Some(data) = (unsafe { data.as_ref() }) else { return Ok(()) };
        (self.sink)(if self.capture { AudioEvent::CaptureMute { muted: data.bMuted.as_bool() } } else { volume_event(data) });
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
    start_watch(Arc::new(move |_| _ = changed.send(())), false)
}

/// Like [`watch`], saying what changed. `capture` also follows the default microphone's
/// mute ([`AudioEvent::CaptureMute`]). Blocks until the callbacks are registered; each call
/// has its own thread.
pub fn watch_events(events: UnboundedSender<AudioEvent>, capture: bool) -> Option<Watch> {
    start_watch(Arc::new(move |e| _ = events.send(e)), capture)
}

fn start_watch(sink: Sink, capture: bool) -> Option<Watch> {
    let (cmd, cmd_rx) = mpsc::channel();
    let (ready_tx, ready) = mpsc::channel();
    let worker_cmd = cmd.clone();
    let thread = std::thread::Builder::new()
        .name("reui-audio-watch".into())
        .spawn(move || run_watch(sink, capture, worker_cmd, cmd_rx, ready_tx))
        .ok()?;
    if ready.recv().unwrap_or(false) {
        Some(Watch { cmd, thread: Some(thread) })
    } else {
        _ = thread.join();
        None
    }
}

fn bind_volume(dev: Option<IMMDevice>, cb: &IAudioEndpointVolumeCallback) -> Option<IAudioEndpointVolume> {
    let ev = unsafe { dev?.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) }.ok()?;
    unsafe { ev.RegisterControlChangeNotify(cb) }.ok()?;
    Some(ev)
}

fn unbind_volume(ev: Option<IAudioEndpointVolume>, cb: &IAudioEndpointVolumeCallback) {
    if let Some(ev) = ev {
        _ = unsafe { ev.UnregisterControlChangeNotify(cb) };
    }
}

fn run_watch(sink: Sink, capture: bool, cmd: mpsc::Sender<Cmd>, cmd_rx: mpsc::Receiver<Cmd>, ready: mpsc::Sender<bool>) {
    let Some(en) = enumerator() else {
        eprintln!("audio: can't watch output devices");
        _ = ready.send(false);
        return;
    };
    let client: IMMNotificationClient = DeviceEvents { sink: sink.clone(), cmd, capture }.into();
    if unsafe { en.RegisterEndpointNotificationCallback(&client) }.is_err() {
        eprintln!("audio: can't watch output devices");
        _ = ready.send(false);
        return;
    }
    _ = ready.send(true);
    let cb: IAudioEndpointVolumeCallback = VolumeEvents { sink: sink.clone(), capture: false }.into();
    let mic_cb: IAudioEndpointVolumeCallback = VolumeEvents { sink: sink.clone(), capture: true }.into();
    let mic_device = || unsafe { en.GetDefaultAudioEndpoint(eCapture, CAPTURE_ROLE) }.ok();
    let mut bound = bind_volume(default_device(&en), &cb);
    let mut mic = if capture { bind_volume(mic_device(), &mic_cb) } else { None };
    while let Ok(first) = cmd_rx.recv() {
        let (mut stop, mut render, mut microphone) = (false, false, false);
        // Several roles change together; one rebind covers them.
        for c in std::iter::once(first).chain(std::iter::from_fn(|| cmd_rx.try_recv().ok())) {
            match c {
                Cmd::Stop => stop = true,
                Cmd::Rebind => render = true,
                Cmd::RebindCapture => microphone = true,
            }
        }
        if stop {
            break;
        }
        if render {
            unbind_volume(bound.take(), &cb);
            let dev = default_device(&en);
            if let Some(DefaultOutput { id, name, kind }) = dev.as_ref().and_then(describe) {
                sink(AudioEvent::DefaultDevice { id, name, kind });
            }
            bound = bind_volume(dev, &cb);
        }
        if microphone && capture {
            unbind_volume(mic.take(), &mic_cb);
            mic = bind_volume(mic_device(), &mic_cb);
        }
    }
    unbind_volume(bound, &cb);
    unbind_volume(mic, &mic_cb);
    _ = unsafe { en.UnregisterEndpointNotificationCallback(&client) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_form_factors_and_echoes() {
        assert_eq!(kind_from_form_factor(1), DeviceKind::Speakers);
        assert_eq!(kind_from_form_factor(3), DeviceKind::Headphones);
        assert_eq!(kind_from_form_factor(5), DeviceKind::Headset);
        assert_eq!(kind_from_form_factor(9), DeviceKind::Display);
        assert_eq!(kind_from_form_factor(8), DeviceKind::Other);
        let mut data = AUDIO_VOLUME_NOTIFICATION_DATA { fMasterVolume: 0.42, bMuted: true.into(), guidEventContext: EVENT_CONTEXT, ..Default::default() };
        assert_eq!(volume_event(&data), AudioEvent::Volume { level: 0.42, muted: true, ours: true });
        data.guidEventContext = GUID::zeroed();
        data.fMasterVolume = 1.5;
        assert_eq!(volume_event(&data), AudioEvent::Volume { level: 1.0, muted: true, ours: false });
    }

    /// Registers and drops the callbacks (no device changes). Run with `--ignored`.
    #[test]
    #[ignore = "registers callbacks on the machine's audio devices"]
    fn audio_watch_registers() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let w = watch(tx);
        assert!(w.is_some());
        drop(w);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        assert!(watch_events(tx, true).is_some());
        println!("{:?} mic muted {:?}", default_output(), capture_muted());
    }
}
