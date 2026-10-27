//! What the card's buttons do: transport (optimistic, snapping back if the player says
//! no), the switcher, volume, mute and the output menu.

use std::time::Duration;

use reui_protocol::{unix_ms, ACTION_EXPANDED, ACTION_SWIPE_NEXT, ACTION_SWIPE_PREV};
use tokio::time::Instant;

use super::card::Repeat;
use super::gsmtc::seek_ticks;
use super::settings::Settings;
use super::gsmtc::{Event, Kind};
use super::{audio, player, sessions, AudioInfo, Internal, State, KEY};
use crate::hub::{ModuleAction, ModuleContext};

const CONTROL_TIMEOUT: Duration = Duration::from_secs(5);
/// Volume drags are applied at most this often (20 Hz), the latest value winning.
const VOLUME_INTERVAL: Duration = Duration::from_millis(50);

impl State {
    pub(super) async fn act(&mut self, ctx: &ModuleContext, action: ModuleAction) {
        // Tray commands come without an activity.
        if action.key.is_empty() {
            let mut settings = self.settings.clone();
            if settings.apply(&action.action) {
                self.settings = settings.clone();
                tokio::task::spawn_blocking(move || settings.save_to(&Settings::path()));
                _ = self.events.send(Event { session: None, kind: Kind::Playback });
            }
            return;
        }
        if action.key != KEY {
            return;
        }
        let a = action.action.as_str();
        match a {
            ACTION_EXPANDED => {
                self.reload_settings();
                self.request_audio();
            }
            "source:next" | "source:prev" => {
                let Some(shown) = self.shown_id else { return };
                if let Some(id) = sessions::step(&self.switchable(), shown, if a == "source:next" { 1 } else { -1 }) {
                    self.picked = Some((id, unix_ms()));
                    self.update(ctx, false).await;
                }
            }
            "open" => {
                let Some(aumid) = self.shown_session().map(|t| t.aumid.clone()) else { return };
                let pid = self.audio.as_ref().filter(|i| i.aumid == aumid).and_then(|i| i.pid);
                tokio::task::spawn_blocking(move || player::open(&aumid, pid));
            }
            "mute" => self.mute(ctx),
            "output:settings" => {
                tokio::task::spawn_blocking(player::open_sound_settings);
            }
            _ if a.starts_with("output:") => self.output(ctx, a["output:".len()..].to_string()),
            _ if a.starts_with("volume:") => {
                if let Some(v) = a["volume:".len()..].parse::<f32>().ok().filter(|v| v.is_finite()) {
                    self.volume(ctx, v.clamp(0.0, 1.0));
                }
            }
            _ => self.control(ctx, a),
        }
    }

    /// Transport. Toggles show their new state at once and snap back if the player says no.
    fn control(&mut self, ctx: &ModuleContext, action: &str) {
        let (Some(card), Some(session)) = (&self.card, self.shown_session().map(|t| t.session.clone())) else { return };
        let c = card.controls.clone();
        let skip = self.settings.swipe_to_skip;
        // One toggle request in flight at a time: a second press would undo the first. Checked
        // before asking, since asking already does it.
        let busy = |control| self.in_flight.contains(control);
        let (control, op) = match action {
            "play" if c.play_pause && !busy("play") => ("play", session.TryTogglePlayPauseAsync()),
            "prev" if c.prev => ("prev", session.TrySkipPreviousAsync()),
            "next" if c.next => ("next", session.TrySkipNextAsync()),
            ACTION_SWIPE_PREV if skip && c.prev => ("prev", session.TrySkipPreviousAsync()),
            ACTION_SWIPE_NEXT if skip && c.next => ("next", session.TrySkipNextAsync()),
            "shuffle" if c.shuffle.is_some() && !busy("shuffle") => ("shuffle", session.TryChangeShuffleActiveAsync(!c.shuffle.unwrap_or(false))),
            "repeat" if c.repeat.is_some() && !busy("repeat") => ("repeat", session.TryChangeAutoRepeatModeAsync(c.repeat.unwrap_or(Repeat::Off).next().mode())),
            seek if seek.starts_with("seek:") => {
                let Ok(ms) = seek["seek:".len()..].parse::<u64>() else { return };
                ("seek", session.TryChangePlaybackPositionAsync(seek_ticks(&session, ms)))
            }
            _ => return,
        };
        let toggle = matches!(control, "play" | "shuffle" | "repeat");
        if toggle {
            self.in_flight.insert(control);
        }
        let now = unix_ms();
        if let Some(card) = &mut self.card {
            match action {
                "play" => card.set_playing(!card.playing, now),
                "shuffle" => card.controls.shuffle = card.controls.shuffle.map(|on| !on),
                "repeat" => card.controls.repeat = card.controls.repeat.map(Repeat::next),
                seek if seek.starts_with("seek:") => {
                    if let (Some(t), Ok(ms)) = (&mut card.timeline, seek["seek:".len()..].parse::<u64>()) {
                        t.position_ms = ms.min(t.length_ms);
                        t.at_unix_ms = now;
                    }
                }
                _ => {}
            }
        }
        if toggle || control == "seek" {
            self.publish(ctx);
        }
        let tx = self.internal.clone();
        tokio::spawn(async move {
            let ok = match op {
                Ok(op) => matches!(tokio::time::timeout(CONTROL_TIMEOUT, async { op.await }).await, Ok(Ok(true))),
                Err(_) => false,
            };
            if !ok {
                eprintln!("media: the player didn't do {control}");
            }
            _ = tx.send(Internal::Done { control, ok });
        });
    }

    fn shown_audio(&mut self) -> Option<&mut AudioInfo> {
        let aumid = self.shown_session()?.aumid.clone();
        self.audio.as_mut().filter(|a| a.aumid == aumid)
    }

    fn volume(&mut self, ctx: &ModuleContext, v: f32) {
        let Some(info) = self.shown_audio() else { return };
        match (&mut info.app, &mut info.system) {
            (Some(app), _) => app.volume = v,
            (None, Some(system)) => system.0 = v,
            (None, None) => return,
        }
        self.publish(ctx);
        if self.volume_ready.is_some_and(|at| Instant::now() < at) {
            self.volume_pending = Some(v);
        } else {
            self.apply_volume(v);
        }
    }

    pub(super) fn apply_volume(&mut self, v: f32) {
        let app = self.audio.as_ref().and_then(|a| a.app.map(|app| app.pid));
        tokio::task::spawn_blocking(move || match app {
            Some(pid) => audio::set_app_volume(pid, v),
            None => audio::set_system_volume(v),
        });
        self.volume_ready = Some(Instant::now() + VOLUME_INTERVAL);
    }

    fn mute(&mut self, ctx: &ModuleContext) {
        let Some(info) = self.shown_audio() else { return };
        let (app, muted) = match (&mut info.app, &mut info.system) {
            (Some(app), _) => {
                app.muted = !app.muted;
                (Some(app.pid), app.muted)
            }
            (None, Some(system)) => {
                system.1 = !system.1;
                (None, system.1)
            }
            (None, None) => return,
        };
        self.publish(ctx);
        tokio::task::spawn_blocking(move || match app {
            Some(pid) => audio::set_app_muted(pid, muted),
            None => audio::set_system_muted(muted),
        });
    }

    /// Switches the default output; where that isn't possible, Sound settings opens.
    fn output(&mut self, ctx: &ModuleContext, id: String) {
        if let Some(info) = self.shown_audio() {
            if info.can_switch && info.outputs.iter().any(|o| o.id == id) {
                for o in &mut info.outputs {
                    o.default = o.id == id;
                }
                self.publish(ctx);
            }
        }
        tokio::task::spawn_blocking(move || {
            if !audio::set_default_output(&id) {
                player::open_sound_settings();
            }
        });
    }
}
