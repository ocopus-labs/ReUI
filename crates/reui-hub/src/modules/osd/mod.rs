//! System OSD: volume, brightness and a few system states shown in the island when they
//! change, adjustable right there (docs/PRD-system-osd.md).
//!
//! Event flow, all callbacks, no polling:
//! - volume, mute, default output and (opt-in) microphone mute: `audio::watch_events` on its
//!   own thread;
//! - built-in panel brightness, battery saver, brightness keys, lock keys, Explorer restarts
//!   and display changes: the shared hidden window (`modules/system_events`), on its own thread;
//! - brightness reads and writes (WMI, DDC/CI): the worker in `brightness.rs`;
//! - Windows' flyout (opt-in hiding, `flyout.rs`) and Core Audio setters: the blocking pool.
//!
//! Each HUD is a `Brief` activity with a fixed key (`volume`, `brightness`), so a change
//! updates the one that's showing instead of replaying its entrance. Publishes are coalesced
//! to ≈30 Hz and a HUD lingers 1.5 s after the last change (2.5 s after the user touched
//! it), staying while its card is open. Our own changes never raise a HUD.
//!
//! Files: `volume.rs` and `brightness_hud.rs` (each HUD's state), `view.rs` (the HUDs as
//! views), `brightness.rs` (the brightness worker), `flyout.rs` (Windows' own flyout) and
//! `settings.rs`.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use reui_protocol::{Priority, ACTION_COLLAPSED, ACTION_EXPANDED};
use tokio::sync::mpsc;
use tokio::time::{sleep_until, Instant};
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

mod brightness;
mod brightness_hud;
mod flyout;
mod settings;
mod view;
mod volume;

use self::brightness::Display;
use self::flyout::Hide;
use self::settings::Settings;
use self::view::{Hud, LINGER_STATE};
use super::audio::{self, AudioEvent, DefaultOutput, Output};
use super::system_events::{self, Subscription};
use super::media::player;
use super::{supervise, Module};
use crate::hub::{ModuleAction, ModuleContext, ModuleInput};

const VOLUME: &str = "volume";
const BRIGHTNESS: &str = "brightness";
const LOCK: &str = "lock";
const BATTERY_SAVER: &str = "battery-saver";
const MIC: &str = "mic";

pub struct SystemOsd;

impl Module for SystemOsd {
    fn name(&self) -> &'static str {
        "osd"
    }

    fn run(self: Box<Self>, mut ctx: ModuleContext) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async move { supervise("osd", &mut ctx, |ctx| Box::pin(run(ctx))).await })
    }
}

/// Whether osd.json asks to hide Windows' volume pop-up (for onboarding's offer).
pub fn replaces_windows_flyout() -> bool {
    Settings::load().replace_windows_flyout
}

/// For `--no-osd`: still put back a flyout an earlier run left hidden.
pub fn recover_flyout() {
    flyout::recover();
}

/// Results of work done off the hub thread.
enum Internal {
    Audio { level: Option<(f32, bool)>, device: Option<DefaultOutput>, mic: Option<bool> },
    Outputs { outputs: Vec<Output>, can_switch: bool },
    Settings(Settings),
    Watch { watch: Option<audio::Watch>, capture: bool },
    Os(Option<Subscription>),
    Flyout(Option<Hide>),
}

struct Osd {
    settings: Settings,
    internal: mpsc::UnboundedSender<Internal>,
    audio_tx: mpsc::UnboundedSender<AudioEvent>,
    watch: Option<audio::Watch>,
    watch_capture: bool,
    os: Option<Subscription>,
    worker: Option<brightness::Worker>,

    // Volume.
    level: f32,
    muted: bool,
    device: Option<DefaultOutput>,
    outputs: Vec<Output>,
    can_switch: bool,
    volume: Hud,
    /// Showing the device moment (after a switch) until then.
    device_until: Option<Instant>,
    switched_at: Option<Instant>,
    last_write: Option<Instant>,
    volume_pending: Option<f32>,
    volume_ready: Option<Instant>,

    // Brightness.
    displays: Vec<Display>,
    front: usize,
    brightness: Hud,
    internal_write: Option<Instant>,
    last_key: Option<Instant>,
    keys_seen: bool,

    // One-off states: key → when it goes.
    states: HashMap<&'static str, Instant>,
    mic_muted: Option<bool>,
    flyout_missing_logged: bool,
}

async fn run(ctx: &mut ModuleContext) -> Result<(), String> {
    let settings = tokio::task::spawn_blocking(Settings::load).await.unwrap_or_default();
    // Always first: put back a flyout a crashed run left hidden, then decide.
    _ = tokio::task::spawn_blocking(flyout::recover).await;
    let _flyout = flyout::Guard;

    let (audio_tx, mut audio_rx) = mpsc::unbounded_channel();
    let (os_tx, mut os_rx) = mpsc::unbounded_channel();
    let (report_tx, mut reports) = mpsc::unbounded_channel();
    let (internal_tx, mut internal) = mpsc::unbounded_channel();
    let worker = brightness::Worker::start(report_tx);
    if let Some(w) = &worker {
        w.probe();
    }
    let mut osd = Osd {
        settings,
        internal: internal_tx.clone(),
        audio_tx,
        watch: None,
        watch_capture: false,
        os: None,
        worker,
        level: 0.0,
        muted: false,
        device: None,
        outputs: Vec::new(),
        can_switch: false,
        volume: Hud::default(),
        device_until: None,
        switched_at: None,
        last_write: None,
        volume_pending: None,
        volume_ready: None,
        displays: Vec::new(),
        front: 0,
        brightness: Hud::default(),
        internal_write: None,
        last_key: None,
        keys_seen: false,
        states: HashMap::new(),
        mic_muted: None,
        flyout_missing_logged: false,
    };
    osd.start_watch();
    osd.read_audio();
    osd.read_outputs();
    tokio::spawn(async move {
        if let Ok(window) = tokio::task::spawn_blocking(move || system_events::subscribe(os_tx)).await {
            _ = internal_tx.send(Internal::Os(window));
        }
    });
    osd.ensure_flyout();

    loop {
        let deadline = osd.next_deadline();
        tokio::select! {
            Some(event) = audio_rx.recv() => osd.on_audio(ctx, event),
            Some(event) = os_rx.recv() => osd.on_os(ctx, event),
            Some(report) = reports.recv() => osd.on_report(ctx, report),
            Some(message) = internal.recv() => osd.on_internal(ctx, message),
            _ = sleep_until(deadline.unwrap_or_else(Instant::now)), if deadline.is_some() => osd.tick(ctx),
            input = ctx.next() => match input {
                Some(ModuleInput::Action(action)) => osd.act(ctx, action),
                Some(_) => {}
                None => break,
            },
        }
    }
    // Unregistering joins the watcher threads; keep that off the hub thread.
    let (watch, os, worker) = (osd.watch.take(), osd.os.take(), osd.worker.take());
    tokio::task::spawn_blocking(move || drop((watch, os, worker)));
    Ok(())
}

impl Osd {
    fn next_deadline(&self) -> Option<Instant> {
        let throttle = self.volume_pending.and(self.volume_ready);
        [self.volume.next_deadline(), self.device_until, self.brightness.next_deadline(), throttle].into_iter().chain(self.states.values().copied().map(Some)).flatten().min()
    }

    // ---- off-thread work ---------------------------------------------------------------

    fn start_watch(&mut self) {
        let capture = self.settings.mic_mute;
        self.watch_capture = capture;
        let (tx, internal) = (self.audio_tx.clone(), self.internal.clone());
        let old = self.watch.take();
        tokio::spawn(async move {
            let watch = tokio::task::spawn_blocking(move || {
                drop(old);
                audio::watch_events(tx, capture)
            })
            .await
            .ok()
            .flatten();
            if watch.is_none() {
                eprintln!("osd: can't watch the system volume; no volume HUD");
            }
            _ = internal.send(Internal::Watch { watch, capture });
        });
    }

    fn read_audio(&self) {
        let (tx, mic) = (self.internal.clone(), self.settings.mic_mute);
        tokio::spawn(async move {
            let read = tokio::task::spawn_blocking(move || (audio::system_audio(), audio::default_output(), if mic { audio::capture_muted() } else { None })).await;
            if let Ok((level, device, mic)) = read {
                _ = tx.send(Internal::Audio { level, device, mic });
            }
        });
    }

    fn read_outputs(&self) {
        let tx = self.internal.clone();
        tokio::spawn(async move {
            if let Ok((outputs, can_switch)) = tokio::task::spawn_blocking(|| (audio::outputs(), audio::can_switch_output())).await {
                _ = tx.send(Internal::Outputs { outputs, can_switch });
            }
        });
    }

    fn reload_settings(&self) {
        let tx = self.internal.clone();
        tokio::spawn(async move {
            if let Ok(settings) = tokio::task::spawn_blocking(Settings::load).await {
                _ = tx.send(Internal::Settings(settings));
            }
        });
    }

    /// Before a HUD shows: Windows' flyout hidden if wanted (Explorer may have made a new
    /// one), and given back while something fullscreen is in front.
    fn ensure_flyout(&self) {
        if !self.settings.replace_windows_flyout {
            return;
        }
        let tx = self.internal.clone();
        tokio::spawn(async move {
            let done = tokio::task::spawn_blocking(|| {
                if flyout::fullscreen() {
                    flyout::restore();
                    None
                } else {
                    Some(flyout::hide())
                }
            })
            .await;
            if let Ok(result) = done {
                _ = tx.send(Internal::Flyout(result));
            }
        });
    }

    fn on_internal(&mut self, ctx: &ModuleContext, message: Internal) {
        match message {
            Internal::Audio { level, device, mic } => {
                if let Some((level, muted)) = level {
                    if view::ours_applies(self.last_write, Instant::now()) {
                        (self.level, self.muted) = (level, muted);
                    }
                }
                if device.is_some() {
                    self.device = device;
                }
                if mic.is_some() {
                    self.mic_muted = mic;
                }
                self.refresh_volume(ctx);
            }
            Internal::Outputs { outputs, can_switch } => {
                (self.outputs, self.can_switch) = (outputs, can_switch);
                self.refresh_volume(ctx);
            }
            Internal::Settings(settings) => self.apply_settings(ctx, settings),
            Internal::Watch { watch, capture } => {
                let old = std::mem::replace(&mut self.watch, watch);
                self.watch_capture = capture;
                if old.is_some() {
                    tokio::task::spawn_blocking(move || drop(old));
                }
            }
            Internal::Os(window) => {
                if let Some(w) = &window {
                    w.watch_lock_keys(self.settings.lock_keys);
                }
                self.os = window;
            }
            Internal::Flyout(Some(Hide::NotFound)) if !self.flyout_missing_logged => {
                self.flyout_missing_logged = true;
                tokio::task::spawn_blocking(|| eprintln!("osd: Windows' flyout host not found on build {}; both flyouts will show", flyout::windows_build()));
            }
            Internal::Flyout(_) => {}
        }
    }

    fn show_state(&mut self, ctx: &ModuleContext, key: &'static str, glyph: char, text: &str) {
        ctx.publish(key, Priority::Brief, view::state_view(glyph, text));
        self.states.insert(key, Instant::now() + LINGER_STATE);
    }

    fn tick(&mut self, ctx: &ModuleContext) {
        let now = Instant::now();
        if self.device_until.is_some_and(|at| now >= at) {
            self.device_until = None;
            self.publish_volume(ctx);
        }
        let due = self.volume.tick(now);
        if due.retract {
            self.device_until = None;
            ctx.retract(VOLUME);
        } else if due.publish {
            self.publish_volume(ctx);
        }
        let due = self.brightness.tick(now);
        if due.retract {
            ctx.retract(BRIGHTNESS);
        } else if due.publish {
            self.publish_brightness(ctx);
        }
        let gone: Vec<&'static str> = self.states.iter().filter(|(_, at)| now >= **at).map(|(k, _)| *k).collect();
        for key in gone {
            self.states.remove(key);
            ctx.retract(key);
        }
        if self.volume_ready.is_some_and(|at| now >= at) {
            self.volume_ready = None;
            if let Some(level) = self.volume_pending.take() {
                self.apply_volume(level);
            }
        }
    }

    fn act(&mut self, ctx: &ModuleContext, action: ModuleAction) {
        let a = action.action.as_str();
        // Tray commands come without an activity.
        if action.key.is_empty() {
            // Desktops have no brightness keys: the tray can bring the HUD up.
            if a == "brightness:show" {
                if !self.displays.is_empty() {
                    self.brightness.interacted(Instant::now());
                    self.publish_brightness(ctx);
                }
                return;
            }
            // The Settings window wrote osd.json.
            if a == "reload" {
                self.reload_settings();
                return;
            }
            let mut settings = self.settings.clone();
            if settings.apply(a) {
                let saved = settings.clone();
                tokio::task::spawn_blocking(move || saved.save_to(&Settings::path()));
                self.apply_settings(ctx, settings);
            }
            return;
        }
        let now = Instant::now();
        let hud = match action.key.as_str() {
            VOLUME => &mut self.volume,
            BRIGHTNESS => &mut self.brightness,
            _ => return,
        };
        match a {
            ACTION_EXPANDED => {
                hud.expanded();
                self.reload_settings();
                if action.key == VOLUME {
                    self.read_outputs();
                }
            }
            ACTION_COLLAPSED => hud.collapsed(now),
            "mute" => self.toggle_mute(ctx),
            "output:settings" => {
                self.volume.interacted(now);
                tokio::task::spawn_blocking(player::open_sound_settings);
            }
            _ if a.starts_with("output:") => self.set_output(ctx, a["output:".len()..].to_string()),
            "display-settings" => {
                self.brightness.interacted(now);
                tokio::task::spawn_blocking(open_display_settings);
            }
            _ => {
                if let Some(level) = view::parse_volume_action(a) {
                    self.set_volume(ctx, level);
                } else if let Some((id, level)) = view::parse_brightness_action(a) {
                    let id = id.to_owned();
                    self.set_brightness(ctx, &id, level);
                }
            }
        }
    }

    fn apply_settings(&mut self, ctx: &ModuleContext, settings: Settings) {
        if settings == self.settings {
            return;
        }
        let old = std::mem::replace(&mut self.settings, settings);
        let new = &self.settings;
        if old.volume && !new.volume && self.volume.visible {
            self.volume.hide();
            ctx.retract(VOLUME);
        }
        if old.brightness && !new.brightness && self.brightness.visible {
            self.brightness.hide();
            ctx.retract(BRIGHTNESS);
        }
        if old.replace_windows_flyout != new.replace_windows_flyout {
            if new.replace_windows_flyout {
                self.ensure_flyout();
            } else {
                tokio::task::spawn_blocking(flyout::restore);
            }
        }
        if old.lock_keys != new.lock_keys {
            if let Some(w) = &self.os {
                w.watch_lock_keys(new.lock_keys);
            }
        }
        if new.mic_mute != self.watch_capture {
            self.mic_muted = None;
            self.start_watch();
            if self.settings.mic_mute {
                self.read_audio();
            }
        }
        self.refresh_volume(ctx);
        self.refresh_brightness(ctx);
    }
}

/// Settings › Display, from the brightness slider's sun.
fn open_display_settings() {
    unsafe {
        _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let result = ShellExecuteW(None, &HSTRING::from("open"), &HSTRING::from("ms-settings:display"), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
        if result.0 as isize <= 32 {
            eprintln!("osd: can't open Display settings ({})", result.0 as isize);
        }
    }
}
