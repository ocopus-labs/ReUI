//! The mixer's side of the module: the app-session watcher runs only while the volume card
//! is open with the mixer on (so a closed card costs nothing), follows the default output,
//! and the card's levels and mutes go back through it. Names and icons are read once per
//! exe on the blocking pool.

use reui_protocol::Channel;

use super::{mixer, Internal, Osd};
use crate::hub::ModuleContext;
use crate::modules::audio;

impl Osd {
    /// Whether the mixer should be watching now.
    fn mixer_wanted(&self) -> bool {
        self.settings.mixer && self.settings.volume && self.volume.open
    }

    /// Starts or stops the session watcher to match [`mixer_wanted`]; `restart` follows a
    /// new default output.
    pub(super) fn sync_mixer(&mut self, restart: bool) {
        let wanted = self.mixer_wanted();
        if wanted && self.sessions.is_some() && !restart {
            return;
        }
        if let Some(old) = self.sessions.take() {
            // Unregistering joins its thread.
            tokio::task::spawn_blocking(move || drop(old));
        }
        if !wanted {
            self.mixer.clear();
            self.mixer_generation += 1;
            return;
        }
        self.mixer_generation += 1;
        let (tx, internal, generation) = (self.sessions_tx.clone(), self.internal.clone(), self.mixer_generation);
        tokio::spawn(async move {
            let sessions = tokio::task::spawn_blocking(move || audio::watch_sessions(tx)).await.ok().flatten();
            if sessions.is_none() {
                eprintln!("osd: can't watch the apps' audio sessions; no mixer");
            }
            _ = internal.send(Internal::Sessions { sessions, generation });
        });
    }

    /// The watcher started (or failed); one that's no longer wanted is let go.
    pub(super) fn on_sessions(&mut self, sessions: Option<audio::Sessions>, generation: u64) {
        if generation != self.mixer_generation || !self.mixer_wanted() {
            if let Some(s) = sessions {
                tokio::task::spawn_blocking(move || drop(s));
            }
            return;
        }
        self.sessions = sessions;
    }

    pub(super) fn on_apps(&mut self, ctx: &ModuleContext, apps: Vec<audio::AppSession>) {
        // The first list can arrive before the watcher's handle does.
        if !self.mixer_wanted() {
            return;
        }
        self.mixer.update(&apps);
        for exe in self.mixer.lines().iter().map(|l| l.exe.clone()).collect::<Vec<_>>() {
            self.load_app(exe);
        }
        self.refresh_volume(ctx);
    }

    fn load_app(&mut self, exe: String) {
        if self.app_names.contains_key(&exe) || !self.app_loading.insert(exe.clone()) {
            return;
        }
        let tx = self.internal.clone();
        tokio::spawn(async move {
            let path = exe.clone();
            let read = tokio::task::spawn_blocking(move || (crate::modules::clipboard::app_name(&path), crate::modules::clipboard::exe_icon(&path))).await;
            if let Ok((name, png)) = read {
                _ = tx.send(Internal::App { exe, name, png });
            }
        });
    }

    pub(super) fn on_app(&mut self, ctx: &ModuleContext, exe: String, name: String, png: Option<Vec<u8>>) {
        self.app_loading.remove(&exe);
        self.app_icons.insert(exe.clone(), png.map(|b| ctx.image(b)));
        self.app_names.insert(exe, name);
        self.refresh_volume(ctx);
    }

    /// The card's mixer button: on or off, remembered in osd.json.
    pub(super) fn toggle_mixer(&mut self, ctx: &ModuleContext) {
        self.settings.mixer = !self.settings.mixer;
        let saved = self.settings.clone();
        tokio::task::spawn_blocking(move || saved.save_to(&super::Settings::path()));
        self.sync_mixer(false);
        self.publish_volume(ctx);
    }

    /// `app:<key>:<level>` and `app-mute:<key>` from the card. `false` if it's neither.
    pub(super) fn mixer_action(&mut self, ctx: &ModuleContext, action: &str) -> bool {
        let Some(sessions) = &self.sessions else { return mixer::parse_level(action).is_some() || mixer::parse_mute(action).is_some() };
        if let Some((key, level)) = mixer::parse_level(action) {
            self.mixer.set_volume(key, level);
            sessions.set_volume(key, level);
        } else if let Some(key) = mixer::parse_mute(action) {
            let muted = !self.mixer.muted(key).unwrap_or(false);
            self.mixer.set_muted(key, muted);
            sessions.set_muted(key, muted);
        } else {
            return false;
        }
        self.volume.interacted(tokio::time::Instant::now());
        self.publish_volume(ctx);
        true
    }

    /// What the card's mixer shows: `None` while it's off.
    pub(super) fn channels(&self) -> Option<Vec<Channel>> {
        self.settings.mixer.then(|| {
            mixer::channels(
                self.mixer.lines(),
                |exe| self.app_names.get(exe).cloned().unwrap_or_else(|| exe.rsplit('\\').next().unwrap_or(exe).trim_end_matches(".exe").to_owned()),
                |exe| self.app_icons.get(exe).copied().flatten(),
            )
        })
    }
}
