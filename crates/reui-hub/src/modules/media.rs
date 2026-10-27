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

use std::collections::HashSet;
use std::future::Future;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::pin::Pin;
use std::time::Duration;

use reui_protocol::{
    unix_ms, Art, Button, ButtonContent, ButtonRole, CompactView, Element, ExpandedView, Headline, ImageId, LyricLine, Menu, MenuOption, Pager, Priority, Rgb, Row, ACTION_EXPANDED, ACTION_SWIPE_NEXT,
    ACTION_SWIPE_PREV,
};
use tokio::sync::mpsc;
use tokio::time::Instant;
use windows::core::{AgileReference, Interface, Result, RuntimeType};
use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session, GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionPlaybackControls as PlaybackControls, GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};
use windows::Media::{MediaPlaybackAutoRepeatMode as RepeatMode, MediaPlaybackType};
use windows::Storage::Streams::{DataReader, IRandomAccessStreamReference};
use windows_future::IAsyncOperation;

use super::media_settings::{Settings, Visualizer};
use super::sessions::{self, Candidate};
use super::{artwork, audio, lyrics, player, supervise, Module};
use crate::hub::{ModuleAction, ModuleContext, ModuleInput, View};

const KEY: &str = "now-playing";
/// Players fire several events per change (metadata, then playback, then timeline).
const SETTLE: Duration = Duration::from_millis(60);
/// Metadata comes in pieces (a blank title, then the title, then the art): wait for it.
const SETTLE_METADATA: Duration = Duration::from_millis(150);
/// Covers bigger than this are refused; everything is scaled to `ARTWORK_PX` before use.
const MAX_ARTWORK: u64 = 16 * 1024 * 1024;
const ARTWORK_PX: u32 = 512;
/// A player that hangs while handing over its cover or metadata must not hold anything up.
const ARTWORK_TIMEOUT: Duration = Duration::from_secs(2);
const READ_TIMEOUT: Duration = Duration::from_secs(2);
const CONTROL_TIMEOUT: Duration = Duration::from_secs(5);
/// Players briefly report nothing between tracks; wait this long before taking Now Playing
/// away, so skipping doesn't close the card or flicker the pill.
const RETRACT_GRACE: Duration = Duration::from_millis(2000);
/// Volume drags are applied at most this often (20 Hz), the latest value winning.
const VOLUME_INTERVAL: Duration = Duration::from_millis(50);
/// Apple Music-like pink when the artwork gives no colour.
const DEFAULT_ACCENT: Rgb = Rgb(255, 55, 95);
/// For covers without a vivid colour: light grey, so the card stays as quiet as the art.
const NEUTRAL_ACCENT: Rgb = Rgb(229, 229, 234);
/// 100 ns ticks between 1601-01-01 (Windows) and 1970-01-01 (Unix).
const EPOCH_DIFFERENCE: i64 = 116_444_736_000_000_000;

const GLYPH_PREV: char = '\u{E892}';
const GLYPH_NEXT: char = '\u{E893}';
const GLYPH_PLAY: char = '\u{E768}';
const GLYPH_PAUSE: char = '\u{E769}';
const GLYPH_MUSIC: char = '\u{E8D6}';
const GLYPH_SHUFFLE: char = '\u{E8B1}';
const GLYPH_REPEAT_ALL: char = '\u{E8EE}';
const GLYPH_REPEAT_ONE: char = '\u{E8ED}';
const GLYPH_VOLUME: char = '\u{E767}';
const GLYPH_MUTED: char = '\u{E74F}';
const GLYPH_OUTPUT: char = '\u{E7F3}';

pub struct NowPlaying;

impl Module for NowPlaying {
    fn name(&self) -> &'static str {
        "media"
    }

    fn run(self: Box<Self>, mut ctx: ModuleContext) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async move { supervise("media", &mut ctx, |ctx| Box::pin(run(ctx))).await })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Media,
    Playback,
    Timeline,
}

/// "Something changed": in one session (`Some(id)`), or in the list of sessions (`None`).
#[derive(Debug, Clone, Copy)]
struct Event {
    session: Option<u64>,
    kind: Kind,
}

/// Results of work done off the loop.
enum Internal {
    /// A transport request finished; `ok` is the player's answer.
    Done { control: &'static str, ok: bool },
    Audio(AudioInfo),
    Lyrics { track: String, lines: Option<Vec<LyricLine>> },
    Settings(Settings),
}

/// A session we're subscribed to, and what we've seen of it.
struct Tracked {
    id: u64,
    session: Session,
    aumid: String,
    tokens: [i64; 3],
    playing: bool,
    closed: bool,
    started_at: u64,
    last_played: u64,
    /// Has had a title (kept through the blank moment between tracks).
    titled: bool,
    video: bool,
}

impl Tracked {
    /// Registers for the session's changes, undoing partial registrations on failure.
    fn subscribe(id: u64, session: Session, aumid: String, tx: &mpsc::UnboundedSender<Event>, now: u64) -> Result<Self> {
        let media = session.MediaPropertiesChanged(&notify(tx, Some(id), Kind::Media))?;
        let playback = session.PlaybackInfoChanged(&notify(tx, Some(id), Kind::Playback)).inspect_err(|_| {
            _ = session.RemoveMediaPropertiesChanged(media);
        })?;
        let timeline = session.TimelinePropertiesChanged(&notify(tx, Some(id), Kind::Timeline)).inspect_err(|_| {
            _ = session.RemoveMediaPropertiesChanged(media);
            _ = session.RemovePlaybackInfoChanged(playback);
        })?;
        Ok(Self { id, session, aumid, tokens: [media, playback, timeline], playing: false, closed: false, started_at: now, last_played: now, titled: false, video: false })
    }

    fn unsubscribe(&self) {
        let [media, playback, timeline] = self.tokens;
        _ = self.session.RemoveMediaPropertiesChanged(media);
        _ = self.session.RemovePlaybackInfoChanged(playback);
        _ = self.session.RemoveTimelinePropertiesChanged(timeline);
    }

    /// Notes the playback status: when it started playing, when it last played.
    fn observe(&mut self, now: u64) {
        let status = self.session.GetPlaybackInfo().and_then(|p| p.PlaybackStatus());
        let playing = matches!(status, Ok(Status::Playing));
        self.closed = matches!(status, Ok(Status::Closed));
        if playing && !self.playing {
            self.started_at = now;
        }
        if playing || self.playing {
            self.last_played = now;
        }
        self.playing = playing;
    }

    fn noted(&mut self, media: &Media) {
        self.titled |= !media.title.trim().is_empty();
        self.video = media.video;
    }

    fn eligible(&self, show_video: bool) -> bool {
        self.titled && !self.closed && (show_video || !self.video)
    }

    fn candidate(&self, current: bool, show_video: bool) -> Candidate {
        Candidate { id: self.id, playing: self.playing, started_at: self.started_at, last_played: self.last_played, current, eligible: self.eligible(show_video) }
    }
}

/// What a player says about its track.
struct Media {
    title: String,
    artist: String,
    album: String,
    thumbnail: Option<AgileReference<IRandomAccessStreamReference>>,
    video: bool,
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

/// A cover read and prepared off the hub thread.
struct Cover {
    png: Vec<u8>,
    hash: u64,
    accent: Option<Rgb>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Repeat {
    Off,
    All,
    One,
}

impl Repeat {
    fn from_mode(mode: RepeatMode) -> Self {
        match mode {
            RepeatMode::List => Self::All,
            RepeatMode::Track => Self::One,
            _ => Self::Off,
        }
    }

    fn mode(self) -> RepeatMode {
        match self {
            Self::Off => RepeatMode::None,
            Self::All => RepeatMode::List,
            Self::One => RepeatMode::Track,
        }
    }

    /// None → List → Track → None, as players cycle it.
    fn next(self) -> Self {
        match self {
            Self::Off => Self::All,
            Self::All => Self::One,
            Self::One => Self::Off,
        }
    }
}

/// Transport controls the player advertises; shuffle and repeat carry their state.
#[derive(Debug, Clone, Default, PartialEq)]
struct Controls {
    prev: bool,
    play_pause: bool,
    next: bool,
    shuffle: Option<bool>,
    repeat: Option<Repeat>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Timeline {
    position_ms: u64,
    /// 0 for a live stream.
    length_ms: u64,
    at_unix_ms: u64,
    seekable: bool,
    rate: f32,
}

#[derive(Debug, Clone, PartialEq)]
struct Volume {
    value: f32,
    muted: bool,
    outputs: Vec<audio::Output>,
    can_switch: bool,
}

/// Everything the card shows, so it can be rebuilt without asking the player again (an
/// optimistic play/pause, a volume drag, lyrics arriving).
#[derive(Debug, Clone, PartialEq)]
struct Card {
    aumid: String,
    track: String,
    app: String,
    title: String,
    artist: String,
    album: String,
    ad: bool,
    video: bool,
    art: Element,
    accent: Rgb,
    playing: bool,
    timeline: Option<Timeline>,
    controls: Controls,
    volume: Option<Volume>,
    lyrics: Vec<LyricLine>,
    pager: Option<Pager>,
    visualizer: Visualizer,
    audio_pid: Option<u32>,
}

impl Card {
    /// Playing or paused as of `now`, the timeline continuing from where it is.
    fn set_playing(&mut self, playing: bool, now: u64) {
        if let Some(t) = &mut self.timeline {
            t.position_ms = Row::timeline_position(t.position_ms, t.length_ms, self.playing, t.at_unix_ms, t.rate, now);
            t.at_unix_ms = now;
        }
        self.playing = playing;
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

    async fn act(&mut self, ctx: &ModuleContext, action: ModuleAction) {
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

    fn apply_volume(&mut self, v: f32) {
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

/// Fills in what doesn't come from the player itself.
fn decorate(card: &mut Card, settings: &Settings, audio: Option<&AudioInfo>, lyrics: Option<&(String, Option<Vec<LyricLine>>)>, pager: Option<Pager>) {
    let audio = audio.filter(|a| a.aumid == card.aumid);
    card.app = sessions::app_name(&card.aumid, audio.and_then(|a| a.exe.as_deref()));
    card.visualizer = settings.visualizer;
    card.audio_pid = audio.and_then(|a| a.pid);
    card.volume = audio.and_then(|a| {
        let (value, muted) = a.level()?;
        Some(Volume { value, muted, outputs: a.outputs.clone(), can_switch: a.can_switch })
    });
    let wanted = settings.lyrics && !card.ad;
    card.lyrics = lyrics.filter(|(track, _)| wanted && *track == card.track).and_then(|(_, lines)| lines.clone()).unwrap_or_default();
    card.pager = pager;
}

fn glyph_button(action: &str, glyph: char, role: ButtonRole) -> Button {
    Button { action: action.into(), content: ButtonContent::Glyph(glyph), role }
}

/// Shuffle · prev · play/pause · next · repeat, each only when the player offers it.
fn controls_row(c: &Controls, playing: bool) -> Vec<Button> {
    let lit = |on: bool| if on { ButtonRole::Accent } else { ButtonRole::Plain };
    let mut buttons = Vec::new();
    if let Some(on) = c.shuffle {
        buttons.push(glyph_button("shuffle", GLYPH_SHUFFLE, lit(on)));
    }
    if c.prev {
        buttons.push(glyph_button("prev", GLYPH_PREV, ButtonRole::Plain));
    }
    if c.play_pause {
        buttons.push(glyph_button("play", if playing { GLYPH_PAUSE } else { GLYPH_PLAY }, ButtonRole::Primary));
    }
    if c.next {
        buttons.push(glyph_button("next", GLYPH_NEXT, ButtonRole::Plain));
    }
    if let Some(repeat) = c.repeat {
        buttons.push(glyph_button("repeat", if repeat == Repeat::One { GLYPH_REPEAT_ONE } else { GLYPH_REPEAT_ALL }, lit(repeat != Repeat::Off)));
    }
    buttons
}

/// Volume with mute, and the output menu (Sound settings where outputs can't be switched).
fn volume_row(v: &Volume) -> Row {
    let options = if v.can_switch && !v.outputs.is_empty() {
        v.outputs.iter().map(|o| MenuOption { label: o.name.clone(), action: format!("output:{}", o.id), selected: o.default }).collect()
    } else {
        vec![MenuOption { label: "Sound settings".into(), action: "output:settings".into(), selected: false }]
    };
    Row::Slider {
        leading: glyph_button("mute", if v.muted { GLYPH_MUTED } else { GLYPH_VOLUME }, ButtonRole::Plain),
        value: v.value.clamp(0.0, 1.0),
        action: "volume:".into(),
        trailing: Some(Menu { content: ButtonContent::Glyph(GLYPH_OUTPUT), options }),
    }
}

/// The card as the island gets it: "App · Album" over the title over the artist.
fn build_view(card: &Card) -> (Priority, View) {
    let bars = Element::Bars { live: card.playing && card.visualizer != Visualizer::Off };
    let mut rows = Vec::new();
    if let Some(t) = &card.timeline {
        rows.push(Row::Timeline { position_ms: t.position_ms, length_ms: t.length_ms, playing: card.playing, at_unix_ms: t.at_unix_ms, seekable: t.seekable && t.length_ms > 0, rate: t.rate });
    }
    let buttons = controls_row(&card.controls, card.playing);
    if !buttons.is_empty() {
        rows.push(Row::Buttons(buttons));
    }
    if let Some(volume) = &card.volume {
        rows.push(volume_row(volume));
    }
    if !card.lyrics.is_empty() && card.timeline.is_some_and(|t| t.length_ms > 0) {
        rows.push(Row::Lyrics(card.lyrics.clone()));
    }
    let album = card.album.trim();
    let eyebrow = if album.is_empty() || album.eq_ignore_ascii_case(card.title.trim()) || card.ad { card.app.clone() } else { format!("{} · {album}", card.app) };
    let detail = if card.ad {
        Some("Ad".to_string())
    } else {
        Some(card.artist.trim().to_string()).filter(|a| !a.is_empty())
    };
    let view = View {
        accent: Some(card.accent),
        audio: if card.visualizer == Visualizer::Real { card.audio_pid } else { None },
        compact: CompactView { leading: card.art.clone(), trailing: bars.clone() },
        // Live bars in the bubble, as in the pill (the user asked for motion there too).
        minimal: bars.clone(),
        expanded: Some(ExpandedView {
            leading: card.art.clone(),
            eyebrow: Some(eyebrow),
            headline: Headline::Text(card.title.clone()),
            detail,
            trailing: bars,
            rows,
            wide: false,
            // Black like the rest of the island: colour comes from the art, bars and progress.
            tint: false,
            pager: card.pager.clone(),
            open: Some("open".into()),
        }),
    };
    (if card.playing { Priority::Ongoing } else { Priority::Passive }, view)
}

/// The view with motion stopped: bars at rest and the timeline held where it is now.
fn frozen(view: &View) -> View {
    let mut view = view.clone();
    let rest = |e: &mut Element| {
        if let Element::Bars { live } = e {
            *live = false;
        }
    };
    rest(&mut view.compact.trailing);
    rest(&mut view.minimal);
    if let Some(expanded) = &mut view.expanded {
        rest(&mut expanded.trailing);
        for row in &mut expanded.rows {
            if let Row::Timeline { position_ms, length_ms, playing, at_unix_ms, rate, .. } = row {
                let now = unix_ms();
                *position_ms = Row::timeline_position(*position_ms, *length_ms, *playing, *at_unix_ms, *rate, now);
                *playing = false;
                *at_unix_ms = now;
            }
        }
    }
    view
}

/// An event handler that just says "something changed", and where.
fn notify<S: RuntimeType + 'static, A: RuntimeType + 'static>(tx: &mpsc::UnboundedSender<Event>, session: Option<u64>, kind: Kind) -> TypedEventHandler<S, A> {
    let tx = tx.clone();
    TypedEventHandler::new(move |_, _| {
        _ = tx.send(Event { session, kind });
        Ok(())
    })
}

/// Sessions are the same player when their object or app agree (a new proxy for the same
/// session must not count as another player).
fn same_session(a: &Session, b: &Tracked) -> bool {
    a.as_raw() == b.session.as_raw() || a.SourceAppUserModelId().is_ok_and(|x| x == b.aumid.as_str())
}

/// The player's metadata, giving up on a player that doesn't answer.
async fn read_media(session: &Session) -> Option<Media> {
    let op = session.TryGetMediaPropertiesAsync().ok()?;
    let media = tokio::time::timeout(READ_TIMEOUT, async { op.await }).await.ok()?.ok()?;
    let text = |r: Result<windows::core::HSTRING>| r.map(|s| s.to_string()).unwrap_or_default();
    Some(Media {
        title: text(media.Title()),
        artist: text(media.Artist()),
        album: text(media.AlbumTitle()),
        thumbnail: media.Thumbnail().ok().and_then(|t| AgileReference::new(&t).ok()),
        video: media.PlaybackType().and_then(|t| t.Value()).is_ok_and(|t| t == MediaPlaybackType::Video),
    })
}

/// Where `ms` into the track is, as the player counts (100 ns ticks from its timeline's
/// start), kept inside what it says it can seek to.
fn seek_ticks(session: &Session, ms: u64) -> i64 {
    let timeline = session.GetTimelineProperties().ok();
    let ticks = |get: fn(&windows::Media::Control::GlobalSystemMediaTransportControlsSessionTimelineProperties) -> Result<windows::Foundation::TimeSpan>| {
        timeline.as_ref().and_then(|t| get(t).ok()).map(|t| t.Duration)
    };
    let start = ticks(|t| t.StartTime()).unwrap_or(0);
    let target = start.saturating_add(i64::try_from(ms).unwrap_or(i64::MAX).saturating_mul(10_000));
    match (ticks(|t| t.MinSeekTime()), ticks(|t| t.MaxSeekTime())) {
        (Some(min), Some(max)) if max > min => target.clamp(min, max),
        _ => target.max(0),
    }
}

/// The position, when the player reports a timeline. A missing one (a stamp at the Windows
/// epoch) means the player doesn't publish it: no bar. A live stream publishes a moving
/// position with no end and no seeking: a length of 0 (the island shows "LIVE").
fn timeline(session: &Session, playing: bool, playing_since: Option<u64>, now: u64, seekable: bool, rate: f32) -> Option<Timeline> {
    let timeline = session.GetTimelineProperties().ok()?;
    let ms = |ticks: i64| (ticks.max(0) / 10_000) as u64;
    let start = ms(timeline.StartTime().ok()?.Duration);
    let end = ms(timeline.EndTime().ok()?.Duration);
    let position = ms(timeline.Position().ok()?.Duration).saturating_sub(start);
    let stamp = timeline.LastUpdatedTime().ok()?.UniversalTime;
    if stamp <= 0 {
        return None;
    }
    let length = end.saturating_sub(start);
    let at_unix_ms = position_stamp(((stamp - EPOCH_DIFFERENCE) / 10_000).max(0) as u64, playing_since, now);
    if length == 0 {
        let live = end == 0 && playing && !seekable && position > 0;
        return live.then_some(Timeline { position_ms: 0, length_ms: 0, at_unix_ms, seekable: false, rate });
    }
    Some(Timeline { position_ms: position.min(length), length_ms: length, at_unix_ms, seekable, rate })
}

/// When the reported position was true, for extrapolating while playing: the player's
/// stamp, but never before playback (re)started (players that don't re-stamp on resume
/// would otherwise jump ahead by the pause) and never in the future.
fn position_stamp(measured_at: u64, playing_since: Option<u64>, now: u64) -> u64 {
    match playing_since {
        Some(since) => measured_at.max(since).min(now),
        None if measured_at == 0 || measured_at > now => now,
        None => measured_at,
    }
}

/// Reads, scales, trims and colours a cover on a blocking thread, giving up after
/// `ARTWORK_TIMEOUT`. `None` for no cover, a broken one, or a hung player.
async fn load_cover(reference: AgileReference<IRandomAccessStreamReference>) -> Option<Cover> {
    let task = tokio::task::spawn_blocking(move || {
        let bytes = read_stream(&reference.resolve().ok()?).ok()?;
        // Scaled once here: smaller to send, hash, decode and draw, and a broken file
        // fails now (music glyph) instead of as a grey tile on the island.
        let artwork::Prepared { png, accent } = artwork::prepare(&bytes, ARTWORK_PX)?;
        let mut hasher = DefaultHasher::new();
        png.hash(&mut hasher);
        Some(Cover { hash: hasher.finish(), accent, png })
    });
    match tokio::time::timeout(ARTWORK_TIMEOUT, task).await {
        Ok(cover) => cover.ok().flatten(),
        Err(_) => {
            eprintln!("media: the player took too long to hand over its artwork");
            None
        }
    }
}

/// Reads a thumbnail stream to its end (its size is sometimes reported as 0), refusing
/// anything over `MAX_ARTWORK`.
fn read_stream(reference: &IRandomAccessStreamReference) -> Result<Vec<u8>> {
    let too_big = || windows::core::Error::new(windows::Win32::Foundation::E_OUTOFMEMORY, "artwork too large");
    let stream = reference.OpenReadAsync()?.join()?;
    let size = stream.Size()?;
    if size > MAX_ARTWORK {
        return Err(too_big());
    }
    let reader = DataReader::CreateDataReader(&stream)?;
    let chunk = if size > 0 { size as u32 } else { 256 * 1024 };
    let mut bytes = Vec::new();
    loop {
        let loaded = reader.LoadAsync(chunk)?.cast::<IAsyncOperation<u32>>()?.join()?;
        if loaded == 0 {
            break;
        }
        let at = bytes.len();
        bytes.resize(at + loaded as usize, 0);
        reader.ReadBytes(&mut bytes[at..])?;
        if bytes.len() as u64 > MAX_ARTWORK {
            return Err(too_big());
        }
        if size > 0 && bytes.len() as u64 >= size {
            break;
        }
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card() -> Card {
        Card {
            aumid: "Spotify.exe".into(),
            track: "t".into(),
            app: "Spotify".into(),
            title: "Song".into(),
            artist: "Band".into(),
            album: "Record".into(),
            ad: false,
            video: false,
            art: Element::Art(Art::Glyph(GLYPH_MUSIC)),
            accent: DEFAULT_ACCENT,
            playing: true,
            timeline: Some(Timeline { position_ms: 10_000, length_ms: 200_000, at_unix_ms: 1_000, seekable: true, rate: 1.0 }),
            controls: Controls { prev: true, play_pause: true, next: true, shuffle: Some(true), repeat: Some(Repeat::One) },
            volume: None,
            lyrics: Vec::new(),
            pager: None,
            visualizer: Visualizer::Real,
            audio_pid: Some(42),
        }
    }

    fn expanded(card: &Card) -> ExpandedView {
        build_view(card).1.expanded.unwrap()
    }

    #[test]
    fn positions_never_count_time_before_playback_started() {
        // Fresh stamp while playing: used as is.
        assert_eq!(position_stamp(9_000, Some(5_000), 10_000), 9_000);
        // Resumed at 8 000 but the player kept the stamp from before the pause.
        assert_eq!(position_stamp(1_000, Some(8_000), 10_000), 8_000);
        // A stamp from the future is clamped to now.
        assert_eq!(position_stamp(12_000, Some(5_000), 10_000), 10_000);
        // Paused: the stamp doesn't matter for extrapolation, but stays sane.
        assert_eq!(position_stamp(0, None, 10_000), 10_000);
        assert_eq!(position_stamp(4_000, None, 10_000), 4_000);
    }

    #[test]
    fn the_card_reads_app_album_title_artist() {
        let e = expanded(&card());
        assert_eq!((e.eyebrow.as_deref(), &e.headline, e.detail.as_deref()), (Some("Spotify · Record"), &Headline::Text("Song".into()), Some("Band")));
        assert!(!e.tint, "the music card stays black");
        assert_eq!(e.open.as_deref(), Some("open"));
        // An album named like the track (singles) isn't repeated.
        let single = Card { album: "song".into(), ..card() };
        assert_eq!(expanded(&single).eyebrow.as_deref(), Some("Spotify"));
        let ad = Card { title: "Advertisement".into(), artist: String::new(), ad: true, ..card() };
        assert_eq!((expanded(&ad).eyebrow.as_deref(), expanded(&ad).detail.as_deref()), (Some("Spotify"), Some("Ad")));
    }

    #[test]
    fn controls_show_only_what_the_player_offers_with_their_state() {
        let row = controls_row(&card().controls, true);
        let summary: Vec<_> = row.iter().map(|b| (b.action.as_str(), b.content.clone(), b.role)).collect();
        assert_eq!(
            summary,
            [
                ("shuffle", ButtonContent::Glyph(GLYPH_SHUFFLE), ButtonRole::Accent),
                ("prev", ButtonContent::Glyph(GLYPH_PREV), ButtonRole::Plain),
                ("play", ButtonContent::Glyph(GLYPH_PAUSE), ButtonRole::Primary),
                ("next", ButtonContent::Glyph(GLYPH_NEXT), ButtonRole::Plain),
                ("repeat", ButtonContent::Glyph(GLYPH_REPEAT_ONE), ButtonRole::Accent),
            ]
        );
        let plain = Controls { play_pause: true, shuffle: Some(false), repeat: Some(Repeat::Off), ..Controls::default() };
        let row = controls_row(&plain, false);
        assert_eq!(row.iter().map(|b| (b.action.as_str(), b.role)).collect::<Vec<_>>(), [("shuffle", ButtonRole::Plain), ("play", ButtonRole::Primary), ("repeat", ButtonRole::Plain)]);
        assert_eq!(row[1].content, ButtonContent::Glyph(GLYPH_PLAY));
        assert_eq!(row[2].content, ButtonContent::Glyph(GLYPH_REPEAT_ALL));
        assert!(controls_row(&Controls::default(), true).is_empty());
        assert_eq!([Repeat::Off.next(), Repeat::All.next(), Repeat::One.next()], [Repeat::All, Repeat::One, Repeat::Off]);
    }

    #[test]
    fn optimistic_play_pause_holds_the_position() {
        let mut c = card();
        c.set_playing(false, 4_000);
        let t = c.timeline.unwrap();
        assert_eq!((c.playing, t.position_ms, t.at_unix_ms), (false, 13_000, 4_000));
        assert_eq!(controls_row(&c.controls, c.playing)[2].content, ButtonContent::Glyph(GLYPH_PLAY));
        assert_eq!(build_view(&c).0, Priority::Passive);
        c.set_playing(true, 9_000);
        assert_eq!(c.timeline.unwrap().position_ms, 13_000);
    }

    #[test]
    fn volume_row_offers_outputs_or_sound_settings() {
        let output = |id: &str, default| audio::Output { id: id.into(), name: format!("Speaker {id}"), default };
        let v = Volume { value: 0.4, muted: false, outputs: vec![output("a", true), output("b", false)], can_switch: true };
        let Row::Slider { leading, value, action, trailing } = volume_row(&v) else { panic!() };
        assert_eq!((leading.action.as_str(), leading.content, value, action.as_str()), ("mute", ButtonContent::Glyph(GLYPH_VOLUME), 0.4, "volume:"));
        let menu = trailing.unwrap();
        assert_eq!(menu.options.iter().map(|o| (o.action.as_str(), o.selected)).collect::<Vec<_>>(), [("output:a", true), ("output:b", false)]);

        let v = Volume { muted: true, can_switch: false, ..v };
        let Row::Slider { leading, trailing, .. } = volume_row(&v) else { panic!() };
        assert_eq!(leading.content, ButtonContent::Glyph(GLYPH_MUTED));
        assert_eq!(trailing.unwrap().options, [MenuOption { label: "Sound settings".into(), action: "output:settings".into(), selected: false }]);
    }

    #[test]
    fn rows_come_in_order_and_lyrics_need_a_timeline() {
        let lyric = vec![LyricLine { at_ms: 0, text: "la".into() }];
        let volume = Volume { value: 1.0, muted: false, outputs: vec![], can_switch: false };
        let c = Card { lyrics: lyric.clone(), volume: Some(volume), ..card() };
        let rows = expanded(&c).rows;
        assert!(matches!(&rows[..], [Row::Timeline { .. }, Row::Buttons(_), Row::Slider { .. }, Row::Lyrics(_)]), "{rows:?}");
        // A live stream: a length-0 timeline, never seekable, and no lyrics.
        let live = Card { timeline: Some(Timeline { position_ms: 0, length_ms: 0, at_unix_ms: 0, seekable: true, rate: 1.0 }), lyrics: lyric, ..card() };
        let rows = expanded(&live).rows;
        assert!(matches!(rows[0], Row::Timeline { length_ms: 0, seekable: false, .. }));
        assert!(!rows.iter().any(|r| matches!(r, Row::Lyrics(_))));
    }

    #[test]
    fn the_visualizer_setting_drives_bars_and_capture() {
        let (_, view) = build_view(&card());
        assert_eq!((view.compact.trailing, view.audio), (Element::Bars { live: true }, Some(42)));
        let (_, view) = build_view(&Card { visualizer: Visualizer::Simple, ..card() });
        assert_eq!((view.compact.trailing, view.audio), (Element::Bars { live: true }, None));
        let (_, view) = build_view(&Card { visualizer: Visualizer::Off, ..card() });
        assert_eq!((view.compact.trailing, view.audio), (Element::Bars { live: false }, None));
        assert_eq!(view.minimal, Element::Bars { live: false }, "the bubble shows the same bars as the pill");
    }

    #[test]
    fn decoration_follows_settings_and_the_shown_player() {
        let mut c = card();
        let lines = vec![LyricLine { at_ms: 0, text: "la".into() }];
        let lyrics = ("t".to_string(), Some(lines.clone()));
        let info = AudioInfo { aumid: "Spotify.exe".into(), pid: Some(7), exe: None, app: None, system: Some((0.3, false)), outputs: vec![], can_switch: false };
        let on = Settings { lyrics: true, ..Settings::default() };
        decorate(&mut c, &on, Some(&info), Some(&lyrics), None);
        assert_eq!((c.lyrics.clone(), c.audio_pid, c.volume.as_ref().map(|v| v.value)), (lines.clone(), Some(7), Some(0.3)));
        // Lyrics off, or for another track, or an ad: none.
        decorate(&mut c, &Settings::default(), Some(&info), Some(&lyrics), None);
        assert!(c.lyrics.is_empty());
        let mut ad = Card { ad: true, ..card() };
        decorate(&mut ad, &on, None, Some(&lyrics), None);
        assert!(ad.lyrics.is_empty() && ad.volume.is_none());
        // Audio read for another player doesn't apply.
        let other = AudioInfo { aumid: "Chrome".into(), ..info };
        decorate(&mut c, &on, Some(&other), None, None);
        assert_eq!((c.audio_pid, c.volume.is_none()), (None, true));
    }

    #[test]
    fn frozen_views_stop_moving() {
        let (_, view) = build_view(&card());
        let still = frozen(&view);
        assert_eq!(still.compact.trailing, Element::Bars { live: false });
        assert!(matches!(still.expanded.unwrap().rows[0], Row::Timeline { playing: false, .. }));
    }
}
