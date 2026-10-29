//! The volume HUD's state: Core Audio events in (ours told apart), and the slider, mute
//! and output menu written back, at most 20 times a second.

use reui_protocol::Priority;
use tokio::time::Instant;

use super::view::{self, VolumeHud, LINGER};
use super::{Osd, MIC, VOLUME};
use crate::hub::ModuleContext;
use crate::modules::audio::{self, AudioEvent, DefaultOutput};
use crate::modules::devices::handoff;
use crate::modules::media::player;

/// Volume writes at most this often (20 Hz), the latest value winning.
const VOLUME_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);

impl Osd {
    pub(super) fn on_audio(&mut self, ctx: &ModuleContext, event: AudioEvent) {
        let now = Instant::now();
        match event {
            AudioEvent::Volume { level, muted, ours } => {
                // Our own echo while a drag is still sending would pull the bar back.
                if ours && !view::ours_applies(self.last_write, now) {
                    return;
                }
                (self.level, self.muted) = (level, muted);
                if !self.settings.volume {
                    return;
                }
                if view::volume_shows(ours, now, self.switched_at) {
                    crate::modules::onboarding::note(crate::modules::onboarding::Moment::Volume { replaced: self.settings.replace_windows_flyout });
                    self.device_until = None;
                    self.show_volume(ctx, now);
                } else {
                    self.refresh_volume(ctx);
                }
            }
            AudioEvent::DefaultDevice { id, name, kind } => {
                self.switched_at = Some(now);
                for o in &mut self.outputs {
                    o.default = o.id == id;
                }
                self.device = Some(DefaultOutput { id, name, kind });
                self.read_audio();
                self.read_outputs();
                // The mixer lists the default output's apps.
                if self.sessions.is_some() {
                    self.mixer.clear();
                    self.sync_mixer(true);
                }

                // A Bluetooth audio device that just connected has the Devices card (S8).
                if self.settings.volume && !handoff::audio_connect_claimed() {
                    self.device_until = Some(now + LINGER);
                    self.show_volume(ctx, now);
                }
            }
            AudioEvent::Outputs => self.read_outputs(),
            AudioEvent::CaptureMute { muted } => {
                let before = self.mic_muted.replace(muted);
                if self.settings.mic_mute && before.is_some_and(|b| b != muted) {
                    let (glyph, text) = if muted { (view::GLYPH_MIC_OFF, "Microphone muted") } else { (view::GLYPH_MIC, "Microphone on") };
                    self.show_state(ctx, MIC, glyph, text);
                }
            }
        }
    }

    fn show_volume(&mut self, ctx: &ModuleContext, now: Instant) {
        if !self.volume.visible {
            self.ensure_flyout();
        }
        if self.volume.changed(now, LINGER) {
            self.publish_volume(ctx);
        }
    }

    /// A value changed: update the HUD if it's showing, without keeping it longer.
    pub(super) fn refresh_volume(&mut self, ctx: &ModuleContext) {
        if self.volume.refresh(Instant::now()) {
            self.publish_volume(ctx);
        }
    }

    pub(super) fn publish_volume(&self, ctx: &ModuleContext) {
        if !self.volume.visible {
            return;
        }
        let device = self.device.as_ref();
        let view = if self.device_until.is_some() {
            view::device_view(device.map_or("Output", |d| d.name.as_str()), device.map(|d| d.kind).unwrap_or_default())
        } else {
            view::volume_view(&VolumeHud {
                level: self.level,
                muted: self.muted,
                device: device.map(|d| d.name.clone()).or_else(|| self.outputs.iter().find(|o| o.default).map(|o| o.name.clone())).unwrap_or_default(),
                kind: device.map(|d| d.kind).unwrap_or_default(),
                outputs: self.outputs.clone(),
                can_switch: self.can_switch,
                show_percentage: self.settings.show_percentage,
                mixer: self.channels(),
            })
        };
        ctx.publish(VOLUME, Priority::Brief, view);
    }

    pub(super) fn set_volume(&mut self, ctx: &ModuleContext, level: f32) {
        let now = Instant::now();
        self.level = level;
        self.last_write = Some(now);
        self.device_until = None;
        self.volume.interacted(now);
        self.publish_volume(ctx);
        if self.volume_ready.is_some_and(|at| now < at) {
            self.volume_pending = Some(level);
        } else {
            self.apply_volume(level);
        }
    }

    pub(super) fn apply_volume(&mut self, level: f32) {
        tokio::task::spawn_blocking(move || audio::set_system_volume(level));
        self.volume_ready = Some(Instant::now() + VOLUME_INTERVAL);
        self.last_write = Some(Instant::now());
    }

    pub(super) fn toggle_mute(&mut self, ctx: &ModuleContext) {
        let now = Instant::now();
        self.muted = !self.muted;
        self.last_write = Some(now);
        self.volume.interacted(now);
        self.publish_volume(ctx);
        let muted = self.muted;
        tokio::task::spawn_blocking(move || audio::set_system_muted(muted));
    }

    /// Switches the default output; where that isn't possible, Sound settings opens.
    pub(super) fn set_output(&mut self, ctx: &ModuleContext, id: String) {
        if self.can_switch && self.outputs.iter().any(|o| o.id == id) {
            for o in &mut self.outputs {
                o.default = o.id == id;
            }
            self.volume.interacted(Instant::now());
            self.publish_volume(ctx);
        }
        tokio::task::spawn_blocking(move || {
            if !audio::set_default_output(&id) {
                player::open_sound_settings();
            }
        });
    }
}
