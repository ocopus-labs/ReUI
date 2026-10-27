//! Now Playing: whatever Windows' media sessions (GSMTC) report, from any player that
//! integrates with the system media controls (Spotify, browsers, VLC, Media Player...).
//!
//! Event-driven: the session manager and the followed session raise "changed" events; each
//! one triggers a re-read (bursts are coalesced). Artwork is fetched only when the track
//! changes, and its most vivid colour becomes the accent. The position is sent with a
//! timestamp and advanced by the island, so playback doesn't cause per-second updates.

use std::future::Future;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::pin::Pin;
use std::time::Duration;

use reui_protocol::{unix_ms, Art, Button, ButtonContent, ButtonRole, CompactView, Element, ExpandedView, Headline, ImageId, Priority, Rgb, Row};
use tokio::sync::mpsc;
use windows::core::{Interface, Result, RuntimeType};
use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session, GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};
use windows::Storage::Streams::DataReader;
use windows_future::IAsyncOperation;

use super::artwork;
use super::Module;
use crate::hub::{ModuleContext, ModuleInput, View};

const KEY: &str = "now-playing";
/// Players fire several events per change (metadata, then playback, then timeline).
const SETTLE: Duration = Duration::from_millis(60);
const MAX_ARTWORK: u32 = 4 * 1024 * 1024;
/// Players briefly report nothing between tracks; wait this long before taking Now Playing
/// away, so skipping doesn't close the card or flicker the pill.
const RETRACT_GRACE: Duration = Duration::from_millis(2000);
/// Apple Music-like pink when the artwork gives no colour.
const DEFAULT_ACCENT: Rgb = Rgb(255, 55, 95);
/// 100 ns ticks between 1601-01-01 (Windows) and 1970-01-01 (Unix).
const EPOCH_DIFFERENCE: i64 = 116_444_736_000_000_000;

const GLYPH_PREV: char = '\u{E892}';
const GLYPH_NEXT: char = '\u{E893}';
const GLYPH_PLAY: char = '\u{E768}';
const GLYPH_PAUSE: char = '\u{E769}';
const GLYPH_MUSIC: char = '\u{E8D6}';

pub struct NowPlaying;

impl Module for NowPlaying {
    fn name(&self) -> &'static str {
        "media"
    }

    fn run(self: Box<Self>, ctx: ModuleContext) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async move {
            if let Err(e) = run(ctx).await {
                eprintln!("media: stopped: {e}");
            }
        })
    }
}

/// The session we're following and its event registrations.
struct Followed {
    session: Session,
    tokens: [i64; 3],
}

impl Followed {
    fn unsubscribe(&self) {
        let [media, playback, timeline] = self.tokens;
        _ = self.session.RemoveMediaPropertiesChanged(media);
        _ = self.session.RemovePlaybackInfoChanged(playback);
        _ = self.session.RemoveTimelinePropertiesChanged(timeline);
    }
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

async fn run(mut ctx: ModuleContext) -> Result<()> {
    let manager = Manager::RequestAsync()?.await?;
    // `true` = metadata (title, artist, cover) may have changed; `false` = playback or position.
    let (changed, mut changes) = mpsc::unbounded_channel::<bool>();
    manager.CurrentSessionChanged(&notify(&changed, true))?;
    manager.SessionsChanged(&notify(&changed, true))?;

    let mut followed: Option<Followed> = None;
    let mut artwork: Option<Artwork> = None;
    let mut published = false;
    let mut retract_at: Option<tokio::time::Instant> = None;
    _ = changed.send(true);

    loop {
        tokio::select! {
            change = changes.recv() => {
                let Some(mut metadata) = change else { break };
                tokio::time::sleep(SETTLE).await;
                while let Ok(more) = changes.try_recv() {
                    metadata |= more;
                }

                let session = pick_session(&manager);
                let same = matches!((&followed, &session), (Some(f), Some(s)) if f.session.as_raw() == s.as_raw());
                if !same {
                    metadata = true;
                    if let Some(old) = followed.take() {
                        old.unsubscribe();
                    }
                    if let Some(session) = session {
                        let tokens = [
                            session.MediaPropertiesChanged(&notify(&changed, true))?,
                            session.PlaybackInfoChanged(&notify(&changed, false))?,
                            session.TimelinePropertiesChanged(&notify(&changed, false))?,
                        ];
                        followed = Some(Followed { session, tokens });
                    }
                }
                let shown = match &followed {
                    Some(f) => publish(&ctx, &f.session, &mut artwork, metadata).await.unwrap_or_else(|e| {
                        eprintln!("media: can't read the session: {e}");
                        false
                    }),
                    None => false,
                };
                if shown {
                    published = true;
                    retract_at = None;
                } else if published && retract_at.is_none() {
                    retract_at = Some(tokio::time::Instant::now() + RETRACT_GRACE);
                }
            }
            _ = async { tokio::time::sleep_until(retract_at.unwrap()).await }, if retract_at.is_some() => {
                retract_at = None;
                published = false;
                ctx.retract(KEY);
            }
            input = ctx.next() => match input {
                Some(ModuleInput::Action(action)) => {
                    let Some(f) = &followed else { continue };
                    let op = match action.action.as_str() {
                        "prev" => f.session.TrySkipPreviousAsync(),
                        "play" => f.session.TryTogglePlayPauseAsync(),
                        "next" => f.session.TrySkipNextAsync(),
                        // Positions are relative to the timeline's start, in 100 ns ticks.
                        seek if seek.starts_with("seek:") => {
                            let Ok(ms) = seek["seek:".len()..].parse::<i64>() else { continue };
                            let start = f.session.GetTimelineProperties().and_then(|t| t.StartTime()).map_or(0, |t| t.Duration);
                            f.session.TryChangePlaybackPositionAsync(start + ms * 10_000)
                        }
                        _ => continue,
                    };
                    // The player answers with change events, which re-publish.
                    if let Err(e) = async { op?.await }.await {
                        eprintln!("media: {} failed: {e}", action.action);
                    }
                }
                Some(_) => {}
                None => break,
            },
        }
    }
    if let Some(f) = followed {
        f.unsubscribe();
    }
    Ok(())
}

/// An event handler that just says "something changed" (and whether it was metadata).
fn notify<S: RuntimeType + 'static, A: RuntimeType + 'static>(tx: &mpsc::UnboundedSender<bool>, metadata: bool) -> TypedEventHandler<S, A> {
    let tx = tx.clone();
    TypedEventHandler::new(move |_, _| {
        _ = tx.send(metadata);
        Ok(())
    })
}

/// The session worth showing: the current one if it's playing, otherwise any playing one,
/// otherwise the current one (paused).
fn pick_session(manager: &Manager) -> Option<Session> {
    let playing = |s: &Session| s.GetPlaybackInfo().and_then(|p| p.PlaybackStatus()).is_ok_and(|st| st == Status::Playing);
    let current = manager.GetCurrentSession().ok();
    if current.as_ref().is_some_and(playing) {
        return current;
    }
    let sessions = manager.GetSessions().ok()?;
    sessions.into_iter().find(playing).or(current)
}

/// Reads the session and publishes it. Returns whether anything is shown.
async fn publish(ctx: &ModuleContext, session: &Session, artwork: &mut Option<Artwork>, metadata_changed: bool) -> Result<bool> {
    let media = session.TryGetMediaPropertiesAsync()?.await?;
    let title = media.Title()?.to_string();
    if title.trim().is_empty() {
        return Ok(false);
    }
    let artist = media.Artist()?.to_string();
    let album = media.AlbumTitle()?.to_string();
    let app = app_name(&session.SourceAppUserModelId()?.to_string());
    let playback = session.GetPlaybackInfo()?;
    let status = playback.PlaybackStatus()?;
    if status == Status::Closed {
        return Ok(false);
    }
    let playing = status == Status::Playing;
    let controls = playback.Controls()?;

    let track = format!("{app}\u{1}{title}\u{1}{artist}");
    let new_track = artwork.as_ref().is_none_or(|a| a.track != track);
    if new_track || metadata_changed {
        let bytes = media.Thumbnail().and_then(|t| read_stream(&t)).ok();
        let hash = bytes.as_deref().map(|b| {
            let mut h = DefaultHasher::new();
            b.hash(&mut h);
            h.finish()
        });
        if new_track || artwork.as_ref().map(|a| a.hash) != Some(hash) {
            let accent = bytes.as_deref().and_then(artwork::accent).unwrap_or(DEFAULT_ACCENT);
            let image = bytes.map(|b| ctx.image(b));
            *artwork = Some(Artwork { track, hash, image, accent });
        }
    }
    let Artwork { image, accent, .. } = artwork.as_ref().unwrap();
    let art = Element::Art(match image {
        Some(id) => Art::Image(*id),
        None => Art::Glyph(GLYPH_MUSIC),
    });

    let mut rows = Vec::new();
    let timeline = session.GetTimelineProperties()?;
    let ms = |ticks: i64| (ticks.max(0) / 10_000) as u64;
    let start = ms(timeline.StartTime()?.Duration);
    let length = ms(timeline.EndTime()?.Duration).saturating_sub(start);
    if length > 0 {
        let measured_at = ((timeline.LastUpdatedTime()?.UniversalTime - EPOCH_DIFFERENCE) / 10_000).max(0) as u64;
        // Some players never refresh the stamp; don't extrapolate from the distant past.
        let at = if measured_at == 0 || measured_at > unix_ms() { unix_ms() } else { measured_at };
        rows.push(Row::Timeline {
            position_ms: ms(timeline.Position()?.Duration).saturating_sub(start).min(length),
            length_ms: length,
            playing,
            at_unix_ms: at,
            seekable: controls.IsPlaybackPositionEnabled().unwrap_or(false),
        });
    }
    let glyph = |action: &str, glyph: char, role| Button { action: action.into(), content: ButtonContent::Glyph(glyph), role };
    let mut buttons = Vec::new();
    if controls.IsPreviousEnabled()? {
        buttons.push(glyph("prev", GLYPH_PREV, ButtonRole::Plain));
    }
    if controls.IsPlayPauseToggleEnabled()? {
        buttons.push(glyph("play", if playing { GLYPH_PAUSE } else { GLYPH_PLAY }, ButtonRole::Primary));
    }
    if controls.IsNextEnabled()? {
        buttons.push(glyph("next", GLYPH_NEXT, ButtonRole::Plain));
    }
    if !buttons.is_empty() {
        rows.push(Row::Buttons(buttons));
    }

    let detail = match (artist.trim().is_empty(), album.trim().is_empty()) {
        (false, _) => Some(artist),
        (true, false) => Some(album),
        (true, true) => None,
    };
    ctx.publish(
        KEY,
        if playing { Priority::Ongoing } else { Priority::Passive },
        View {
            accent: Some(*accent),
            compact: CompactView { leading: art.clone(), trailing: Element::Bars { live: playing } },
            minimal: Element::Bars { live: playing },
            expanded: Some(ExpandedView {
                leading: art,
                eyebrow: Some(app),
                headline: Headline::Text(title),
                detail,
                trailing: Element::Bars { live: playing },
                rows,
            }),
        },
    );
    Ok(true)
}

/// Reads a thumbnail. Blocking on purpose: it's a short local read, and the stream
/// interfaces aren't `Send`, so they can't be held across an `.await` in a module.
fn read_stream(reference: &windows::Storage::Streams::IRandomAccessStreamReference) -> Result<Vec<u8>> {
    let stream = reference.OpenReadAsync()?.join()?;
    let size = stream.Size()?.min(MAX_ARTWORK as u64) as u32;
    let reader = DataReader::CreateDataReader(&stream)?;
    let loaded = reader.LoadAsync(size)?.cast::<IAsyncOperation<u32>>()?.join()?;
    let mut bytes = vec![0u8; loaded as usize];
    reader.ReadBytes(&mut bytes)?;
    Ok(bytes)
}

/// A readable player name from its AppUserModelID ("Spotify.exe", "MSEdge",
/// "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify", "Microsoft.ZuneMusic_8wekyb3d8bbwe!...").
fn app_name(aumid: &str) -> String {
    const KNOWN: [(&str, &str); 9] = [
        ("spotify", "Spotify"),
        ("chrome", "Chrome"),
        ("msedge", "Edge"),
        ("firefox", "Firefox"),
        ("zunemusic", "Media Player"),
        ("vlc", "VLC"),
        ("applemusic", "Apple Music"),
        ("brave", "Brave"),
        ("opera", "Opera"),
    ];
    let lower = aumid.to_ascii_lowercase();
    if let Some((_, name)) = KNOWN.iter().find(|(needle, _)| lower.contains(needle)) {
        return name.to_string();
    }
    // Packaged: "Publisher.App_hash!Entry" → "App"; executable: "C:\...\app.exe" → "app".
    let base = aumid.split('!').next().unwrap_or(aumid);
    let base = base.split('_').next().unwrap_or(base);
    let base = base.rsplit(['\\', '/']).next().unwrap_or(base);
    let base = base.strip_suffix(".exe").unwrap_or(base);
    let name = base.rsplit('.').next().unwrap_or(base);
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => "Now Playing".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::app_name;

    #[test]
    fn player_names_are_readable() {
        assert_eq!(app_name("Spotify.exe"), "Spotify");
        assert_eq!(app_name("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"), "Spotify");
        assert_eq!(app_name("MSEdge"), "Edge");
        assert_eq!(app_name("Chrome"), "Chrome");
        assert_eq!(app_name("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"), "Media Player");
        assert_eq!(app_name("C:\\Program Files\\Foobar2000\\foobar2000.exe"), "Foobar2000");
        assert_eq!(app_name("Publisher.CoolPlayer_abc123!App"), "CoolPlayer");
        assert_eq!(app_name(""), "Now Playing");
    }
}
