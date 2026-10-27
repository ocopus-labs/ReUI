//! Now Playing: whatever Windows' media sessions (GSMTC) report, from any player that
//! integrates with the system media controls (Spotify, browsers, VLC, Media Player...).
//!
//! Event-driven: every session is subscribed, so "another player started" is seen even when
//! Windows keeps its "current" session; which one is shown is our own ranking (sessions.rs).
//! Bursts of events are coalesced (longer for metadata, which arrives in pieces). Artwork is
//! read off the hub thread when the track changes, and its most vivid colour becomes the
//! accent. The position is sent with a timestamp and rate and advanced by the island, so
//! playback doesn't cause per-second updates. Anything blocking (artwork, Core Audio, the
//! lyrics cache, settings) runs on the blocking pool.
//!
//! Files: `gsmtc.rs` (reading sessions), `sessions.rs` (which one is shown), `card.rs`
//! (the card and its view), `controls.rs` (its buttons), `cover.rs` and `artwork.rs`
//! (cover art), `lyrics.rs`, `player.rs` (the player app) and `settings.rs`.

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use reui_protocol::{unix_ms, Art, Element, ImageId, LyricLine, Pager, Priority, Rgb};
use tokio::sync::mpsc;
use tokio::time::Instant;
use windows::core::{Interface, Result};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session, GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionPlaybackControls as PlaybackControls, GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};

mod artwork;
mod card;
mod controls;
mod cover;
mod gsmtc;
mod lyrics;
pub(super) mod player;
mod sessions;
mod settings;

use self::card::{build_view, decorate, frozen, Card, Controls, Repeat, GLYPH_MUSIC};
use self::cover::load_cover;
use self::gsmtc::{notify, read_media, same_session, timeline, Event, Kind, Tracked};
use self::sessions::Candidate;
use self::settings::Settings;
use super::{audio, supervise, Module};
use crate::hub::{ModuleContext, ModuleInput, View};

const KEY: &str = "now-playing";
/// Players fire several events per change (metadata, then playback, then timeline).
const SETTLE: Duration = Duration::from_millis(60);
/// Metadata comes in pieces (a blank title, then the title, then the art): wait for it.
const SETTLE_METADATA: Duration = Duration::from_millis(150);
/// Players briefly report nothing between tracks; wait this long before taking Now Playing
/// away, so skipping doesn't close the card or flicker the pill.
const RETRACT_GRACE: Duration = Duration::from_millis(2000);
/// Apple Music-like pink when the artwork gives no colour.
const DEFAULT_ACCENT: Rgb = Rgb(255, 55, 95);
/// For covers without a vivid colour: light grey, so the card stays as quiet as the art.
const NEUTRAL_ACCENT: Rgb = Rgb(229, 229, 234);

pub struct NowPlaying;

impl Module for NowPlaying {
    fn name(&self) -> &'static str {
        "media"
    }

    fn run(self: Box<Self>, mut ctx: ModuleContext) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async move { supervise("media", &mut ctx, |ctx| Box::pin(run(ctx))).await })
    }
}

/// Results of work done off the loop.
enum Internal {
    /// A transport request finished; `ok` is the player's answer.
    Done { control: &'static str, ok: bool },
    Audio(AudioInfo),
    Lyrics { track: String, lines: Option<Vec<LyricLine>> },
    Settings(Settings),
}

/// Artwork for the current track. Players often change the title first and the cover a
/// moment later (browsers load it), so the cover is re-read on every metadata change and
/// replaced whenever its bytes differ, not only when the track does.
struct Artwork {
    track: String,
    hash: Option<u64>,
    image: Option<ImageId>,
    accent: Rgb,
}

/// The shown player's audio, read off the hub thread.
#[derive(Debug, Clone)]
struct AudioInfo {
    aumid: String,
    /// Root process, for capture and matching.
    pid: Option<u32>,
    /// Executable, for players whose AUMID doesn't name them.
    exe: Option<String>,
    app: Option<audio::AppAudio>,
    system: Option<(f32, bool)>,
    outputs: Vec<audio::Output>,
    can_switch: bool,
}

impl AudioInfo {
    fn read(aumid: String) -> Self {
        let pid = audio::process_for(&aumid);
        let exe = sessions::is_hash(&aumid).then(|| pid.and_then(player::exe_name).or_else(|| player::running(&player::HASH_BROWSERS))).flatten();
        Self { pid, exe, app: pid.and_then(audio::app_audio), system: audio::system_audio(), outputs: audio::outputs(), can_switch: audio::can_switch_output(), aumid }
    }

    /// The level the slider shows: the app's own, else the system's.
    fn level(&self) -> Option<(f32, bool)> {
        self.app.map(|a| (a.volume, a.muted)).or(self.system)
    }
}

/// What a read of the shown session gives.
enum Reading {
    Card(Box<Card>),
    /// No title right now (between tracks): keep what's shown for the grace.
    Blank,
    /// Gone or closed.
    Gone,
}

/// Keeps `audio::watch` alive on its own thread (its handle needn't be `Send`); dropping
/// this ends it.
struct AudioWatch(#[allow(dead_code)] std::sync::mpsc::Sender<()>);

fn watch_audio(changed: mpsc::UnboundedSender<()>) -> AudioWatch {
    let (stop, stopped) = std::sync::mpsc::channel::<()>();
    let started = std::thread::Builder::new().name("media-audio-watch".into()).spawn(move || {
        let _watch = audio::watch(changed);
        _ = stopped.recv();
    });
    if let Err(e) = started {
        eprintln!("media: can't watch audio devices: {e}");
    }
    AudioWatch(stop)
}

struct State {
    manager: Manager,
    events: mpsc::UnboundedSender<Event>,
    internal: mpsc::UnboundedSender<Internal>,
    settings: Settings,
    sessions: Vec<Tracked>,
    next_id: u64,
    /// Chosen with the switcher: `(session, when)`.
    picked: Option<(u64, u64)>,
    shown_id: Option<u64>,
    artwork: Option<Artwork>,
    /// When we first saw the shown session playing (this stretch). Players don't always
    /// re-stamp the position on resume; it can't have advanced before playback started.
    playing_since: Option<u64>,
    card: Option<Card>,
    /// What's on screen, to freeze during the retract grace.
    shown: Option<(Priority, View)>,
    retract_at: Option<Instant>,
    /// When the shown paused player has been paused long enough to step away.
    expire_at: Option<Instant>,
    audio: Option<AudioInfo>,
    lyrics: Option<(String, Option<Vec<LyricLine>>)>,
    lyrics_asked: Option<String>,
    /// Toggles waiting for the player's answer; a second press meanwhile is dropped.
    in_flight: HashSet<&'static str>,
    volume_pending: Option<f32>,
    volume_ready: Option<Instant>,
}

async fn run(ctx: &mut ModuleContext) -> Result<()> {
    let manager = manager().await;
    let (events_tx, mut events) = mpsc::unbounded_channel::<Event>();
    let (internal_tx, mut internal) = mpsc::unbounded_channel::<Internal>();
    let (watch_tx, mut watched) = mpsc::unbounded_channel::<()>();
    let current = manager.CurrentSessionChanged(&notify(&events_tx, None, Kind::Playback)).inspect_err(|e| eprintln!("media: can't watch media sessions: {e}")).ok();
    let list = manager.SessionsChanged(&notify(&events_tx, None, Kind::Playback)).inspect_err(|e| eprintln!("media: can't watch media sessions: {e}")).ok();
    let _watch = watch_audio(watch_tx);

    let settings = tokio::task::spawn_blocking(Settings::load).await.unwrap_or_default();
    let mut state = State {
        manager: manager.clone(),
        events: events_tx.clone(),
        internal: internal_tx,
        settings,
        sessions: Vec::new(),
        next_id: 0,
        picked: None,
        shown_id: None,
        artwork: None,
        playing_since: None,
        card: None,
        shown: None,
        retract_at: None,
        expire_at: None,
        audio: None,
        lyrics: None,
        lyrics_asked: None,
        in_flight: HashSet::new(),
        volume_pending: None,
        volume_ready: None,
    };
    _ = events_tx.send(Event { session: None, kind: Kind::Media });
    let result = loop {
        let (retract_at, expire_at, volume_ready) = (state.retract_at, state.expire_at, state.volume_ready);
        tokio::select! {
            event = events.recv() => {
                let Some(event) = event else { break Ok(()) };
                let mut batch = vec![event];
                tokio::time::sleep(SETTLE).await;
                while let Ok(more) = events.try_recv() {
                    batch.push(more);
                }
                if batch.iter().any(|e| e.kind == Kind::Media) {
                    tokio::time::sleep(SETTLE_METADATA - SETTLE).await;
                    while let Ok(more) = events.try_recv() {
                        batch.push(more);
                    }
                }
                state.changed(ctx, batch).await;
            }
            Some(message) = internal.recv() => state.finished(ctx, message),
            Some(()) = watched.recv() => {
                while watched.try_recv().is_ok() {}
                state.request_audio();
            }
            _ = async move { tokio::time::sleep_until(retract_at.unwrap()).await }, if retract_at.is_some() => {
                state.retract_at = None;
                state.shown = None;
                state.card = None;
                ctx.retract(KEY);
            }
            _ = async move { tokio::time::sleep_until(expire_at.unwrap()).await }, if expire_at.is_some() => {
                state.expire_at = None;
                state.update(ctx, false).await;
            }
            _ = async move { tokio::time::sleep_until(volume_ready.unwrap()).await }, if volume_ready.is_some() && state.volume_pending.is_some() => {
                if let Some(v) = state.volume_pending.take() {
                    state.apply_volume(v);
                }
            }
            input = ctx.next() => match input {
                Some(ModuleInput::Action(action)) => state.act(ctx, action).await,
                Some(_) => {}
                None => break Ok(()),
            },
        }
    };
    for t in state.sessions.drain(..) {
        t.unsubscribe();
    }
    if let Some(token) = current {
        _ = manager.RemoveCurrentSessionChanged(token);
    }
    if let Some(token) = list {
        _ = manager.RemoveSessionsChanged(token);
    }
    result
}

/// The session manager, waiting for it if media sessions aren't ready yet (early at login).
async fn manager() -> Manager {
    let mut wait = Duration::from_secs(1);
    loop {
        match async { Manager::RequestAsync()?.await }.await {
            Ok(manager) => return manager,
            Err(e) => eprintln!("media: media sessions unavailable ({e}); retrying in {}s", wait.as_secs()),
        }
        tokio::time::sleep(wait).await;
        wait = (wait * 2).min(Duration::from_secs(30));
    }
}

impl State {
    fn tracked(&self, id: u64) -> Option<&Tracked> {
        self.sessions.iter().find(|t| t.id == id)
    }

    fn shown_session(&self) -> Option<&Tracked> {
        self.shown_id.and_then(|id| self.tracked(id))
    }

    fn retract_ms(&self) -> u64 {
        u64::from(self.settings.retract_after_pause_min) * 60_000
    }

    fn exe_hint(&self, aumid: &str) -> Option<&str> {
        self.audio.as_ref().filter(|a| a.aumid == aumid).and_then(|a| a.exe.as_deref())
    }

    /// A batch of settled events.
    async fn changed(&mut self, ctx: &ModuleContext, batch: Vec<Event>) {
        // Another player's position moving changes nothing here.
        if batch.iter().all(|e| e.kind == Kind::Timeline && e.session.is_some() && e.session != self.shown_id) {
            return;
        }
        let now = unix_ms();
        if batch.iter().any(|e| e.session.is_none()) {
            self.sync_sessions(now).await;
        }
        let mut metadata = false;
        let mut seen = HashSet::new();
        for e in &batch {
            let Some(id) = e.session else { continue };
            let media = e.kind == Kind::Media;
            metadata |= media && Some(id) == self.shown_id;
            if !seen.insert((id, media)) {
                continue;
            }
            let shown = self.shown_id == Some(id);
            let Some(t) = self.sessions.iter_mut().find(|t| t.id == id) else { continue };
            t.observe(now);
            // The shown session's metadata is read in full below.
            if media && !shown {
                if let Some(m) = read_media(&t.session).await {
                    t.noted(&m);
                }
            }
        }
        self.update(ctx, metadata).await;
    }

    /// Subscribes to new sessions and lets go of ended ones. Sessions are matched by object,
    /// then by app (a new proxy for the same player isn't a new player).
    async fn sync_sessions(&mut self, now: u64) {
        let list: Vec<Session> = match self.manager.GetSessions() {
            Ok(list) => list.into_iter().collect(),
            Err(e) => {
                eprintln!("media: can't list media sessions: {e}");
                return;
            }
        };
        let mut keep = vec![false; self.sessions.len()];
        let mut fresh = Vec::new();
        for session in list {
            let aumid = session.SourceAppUserModelId().map(|a| a.to_string()).unwrap_or_default();
            let found = (0..self.sessions.len())
                .find(|&i| !keep[i] && self.sessions[i].session.as_raw() == session.as_raw())
                .or_else(|| (0..self.sessions.len()).find(|&i| !keep[i] && self.sessions[i].aumid == aumid));
            match found {
                Some(i) => keep[i] = true,
                None => fresh.push((session, aumid)),
            }
        }
        let mut kept = keep.into_iter();
        self.sessions.retain(|t| {
            let keep = kept.next().unwrap_or(false);
            if !keep {
                t.unsubscribe();
            }
            keep
        });
        for (session, aumid) in fresh {
            self.next_id += 1;
            match Tracked::subscribe(self.next_id, session, aumid, &self.events, now) {
                Ok(mut t) => {
                    t.observe(now);
                    if let Some(m) = read_media(&t.session).await {
                        t.noted(&m);
                    }
                    self.sessions.push(t);
                }
                Err(e) => eprintln!("media: can't follow a player: {e}"),
            }
        }
    }

    /// Ranks the sessions and shows the winner (or steps away).
    async fn update(&mut self, ctx: &ModuleContext, mut metadata: bool) {
        let now = unix_ms();
        let retract = self.retract_ms();
        let current = self.manager.GetCurrentSession().ok();
        let candidates: Vec<Candidate> = self.sessions.iter().map(|t| t.candidate(current.as_ref().is_some_and(|c| same_session(c, t)), self.settings.show_video)).collect();
        let chosen = sessions::rank(&candidates, self.picked, now, retract);
        if self.picked.is_some_and(|(id, _)| Some(id) != chosen) {
            self.picked = None;
        }
        if chosen != self.shown_id {
            metadata = true;
            self.shown_id = chosen;
            self.playing_since = None;
            self.in_flight.clear();
            self.volume_pending = None;
            self.request_audio();
        }
        self.expire_at = chosen
            .and_then(|id| candidates.iter().find(|c| c.id == id))
            .and_then(|c| sessions::expires_at(c, self.picked, retract))
            .map(|at| Instant::now() + Duration::from_millis(at.saturating_sub(now)));
        let reading = match chosen {
            Some(id) => self.read(ctx, id, metadata).await.unwrap_or_else(|e| {
                eprintln!("media: can't read the session: {e}");
                Reading::Gone
            }),
            None => Reading::Gone,
        };
        self.show(ctx, reading);
    }

    /// Reads the shown session into a card.
    async fn read(&mut self, ctx: &ModuleContext, id: u64, metadata_changed: bool) -> Result<Reading> {
        let Some((session, aumid)) = self.tracked(id).map(|t| (t.session.clone(), t.aumid.clone())) else { return Ok(Reading::Gone) };
        let Some(media) = read_media(&session).await else { return Ok(Reading::Blank) };
        if let Some(t) = self.sessions.iter_mut().find(|t| t.id == id) {
            t.noted(&media);
        }
        if media.title.trim().is_empty() {
            return Ok(Reading::Blank);
        }
        if session.GetPlaybackInfo()?.PlaybackStatus()? == Status::Closed {
            return Ok(Reading::Gone);
        }

        let track = format!("{aumid}\u{1}{}\u{1}{}", media.title, media.artist);
        let new_track = self.artwork.as_ref().is_none_or(|a| a.track != track);
        if new_track || metadata_changed {
            let cover = match media.thumbnail.clone() {
                Some(reference) => load_cover(reference).await,
                None => None,
            };
            let hash = cover.as_ref().map(|c| c.hash);
            if new_track || self.artwork.as_ref().map(|a| a.hash) != Some(hash) {
                // A cover with no vivid colour (black and white) gets a neutral accent, not the pink.
                let accent = match &cover {
                    Some(c) => c.accent.unwrap_or(NEUTRAL_ACCENT),
                    None => DEFAULT_ACCENT,
                };
                let image = cover.map(|c| ctx.image(c.png));
                self.artwork = Some(Artwork { track: track.clone(), hash, image, accent });
            }
        }
        let Some(Artwork { image, accent, .. }) = self.artwork.as_ref() else { return Ok(Reading::Gone) };
        let art = Element::Art(match image {
            Some(id) => Art::Image(*id),
            None => Art::Glyph(GLYPH_MUSIC),
        });

        // Read again after the cover's wait: the player may have moved on meanwhile.
        let playback = session.GetPlaybackInfo()?;
        let playing = playback.PlaybackStatus()? == Status::Playing;
        let now = unix_ms();
        let since = if playing { Some(*self.playing_since.get_or_insert(now)) } else { None };
        if !playing {
            self.playing_since = None;
        }
        let controls = playback.Controls().ok();
        let can = |check: fn(&PlaybackControls) -> Result<bool>| controls.as_ref().is_some_and(|c| check(c).unwrap_or(false));
        let rate = playback.PlaybackRate().and_then(|r| r.Value()).ok().filter(|r| r.is_finite() && *r > 0.0).unwrap_or(1.0) as f32;
        let controls = Controls {
            prev: can(|c| c.IsPreviousEnabled()),
            play_pause: can(|c| c.IsPlayPauseToggleEnabled()),
            next: can(|c| c.IsNextEnabled()),
            shuffle: can(|c| c.IsShuffleEnabled()).then(|| playback.IsShuffleActive().and_then(|v| v.Value()).unwrap_or(false)),
            repeat: can(|c| c.IsRepeatEnabled()).then(|| playback.AutoRepeatMode().and_then(|v| v.Value()).map_or(Repeat::Off, Repeat::from_mode)),
        };
        let timeline = timeline(&session, playing, since, now, can(|c| c.IsPlaybackPositionEnabled()), rate);
        let app = sessions::app_name(&aumid, self.exe_hint(&aumid));
        let card = Card {
            ad: sessions::is_ad(&aumid, &media.title, &media.artist),
            aumid,
            track,
            app,
            title: media.title,
            artist: media.artist,
            album: media.album,
            video: media.video,
            art,
            accent: *accent,
            playing,
            timeline,
            controls,
            volume: None,
            lyrics: Vec::new(),
            pager: None,
            visualizer: self.settings.visualizer,
            audio_pid: None,
        };
        self.ask_lyrics(&card);
        Ok(Reading::Card(Box::new(card)))
    }

    /// Publishes a reading, or starts stepping away when there's nothing to show.
    fn show(&mut self, ctx: &ModuleContext, reading: Reading) {
        match reading {
            Reading::Card(card) => {
                self.card = Some(*card);
                self.retract_at = None;
                self.publish(ctx);
            }
            // Between tracks: what's shown stays as it is; gone if no title comes back.
            Reading::Blank => {
                if self.shown.is_some() && self.retract_at.is_none() {
                    self.retract_at = Some(Instant::now() + RETRACT_GRACE);
                }
            }
            Reading::Gone => {
                if self.retract_at.is_some() {
                    return;
                }
                match &self.shown {
                    // Still on screen for the grace, but not pretending to play.
                    Some((priority, view)) if self.card.as_ref().is_some_and(|c| c.playing) => {
                        ctx.publish(KEY, *priority, frozen(view));
                        self.retract_at = Some(Instant::now() + RETRACT_GRACE);
                    }
                    // Paused (or paused long enough): nothing moving to wait for.
                    Some(_) => {
                        ctx.retract(KEY);
                        self.shown = None;
                        self.card = None;
                    }
                    None => {}
                }
            }
        }
    }

    /// Sessions the switcher cycles through, in Windows' order.
    fn pager(&self) -> Option<Pager> {
        let order = self.switchable();
        let index = order.iter().position(|&id| Some(id) == self.shown_id)?;
        (order.len() > 1).then(|| Pager { index: index as u32, count: order.len() as u32, prev: "source:prev".into(), next: "source:next".into() })
    }

    fn switchable(&self) -> Vec<u64> {
        self.sessions.iter().filter(|t| t.eligible(self.settings.show_video)).map(|t| t.id).collect()
    }

    /// (Re)publishes the card with the latest audio, lyrics, settings and switcher.
    fn publish(&mut self, ctx: &ModuleContext) {
        let pager = self.pager();
        let Some(card) = &mut self.card else { return };
        decorate(card, &self.settings, self.audio.as_ref(), self.lyrics.as_ref(), pager);
        let (priority, view) = build_view(card);
        ctx.publish(KEY, priority, view.clone());
        self.shown = Some((priority, view));
    }

    /// Reads the shown player's audio (and executable) on the blocking pool.
    fn request_audio(&self) {
        let Some(aumid) = self.shown_session().map(|t| t.aumid.clone()) else { return };
        let tx = self.internal.clone();
        tokio::spawn(async move {
            if let Ok(info) = tokio::task::spawn_blocking(move || AudioInfo::read(aumid)).await {
                _ = tx.send(Internal::Audio(info));
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

    /// Prefetches lyrics when the track changes, if they're wanted and could be synced.
    fn ask_lyrics(&mut self, card: &Card) {
        let Some(length_ms) = card.timeline.map(|t| t.length_ms).filter(|&l| l > 0) else { return };
        // Video sources too: music is often played as YouTube videos, and LRCLIB only answers
        // when title, artist and length match a real song (other videos simply find nothing).
        if !self.settings.lyrics || card.ad || self.lyrics_asked.as_deref() == Some(card.track.as_str()) {
            return;
        }
        let Some(query) = lyrics::Query::new(&card.title, &card.artist, &card.album, length_ms, sessions::is_browser(&card.app)) else { return };
        self.lyrics_asked = Some(card.track.clone());
        let (tx, track) = (self.internal.clone(), card.track.clone());
        tokio::spawn(async move {
            let lines = lyrics::lookup(query).await;
            _ = tx.send(Internal::Lyrics { track, lines });
        });
    }

    /// Something finished off the loop.
    fn finished(&mut self, ctx: &ModuleContext, message: Internal) {
        match message {
            Internal::Done { control, ok } => {
                self.in_flight.remove(control);
                if !ok {
                    // Undo the optimistic glyph: show what the player really says.
                    if let Some(id) = self.shown_id {
                        _ = self.events.send(Event { session: Some(id), kind: Kind::Playback });
                    }
                }
            }
            Internal::Audio(info) => {
                if self.shown_session().is_some_and(|t| t.aumid == info.aumid) {
                    self.audio = Some(info);
                    self.publish(ctx);
                }
            }
            Internal::Lyrics { track, lines } => {
                let shown = self.card.as_ref().is_some_and(|c| c.track == track);
                self.lyrics = Some((track, lines));
                if shown {
                    self.publish(ctx);
                }
            }
            Internal::Settings(settings) => {
                if settings != self.settings {
                    self.settings = settings;
                    // Ranking (retract time, video), lyrics and bars may all change.
                    _ = self.events.send(Event { session: None, kind: Kind::Playback });
                }
            }
        }
    }

}
