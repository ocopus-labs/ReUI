//! Windows' media sessions (GSMTC): following a session's events, reading its metadata
//! and timeline, and where a seek lands.

use std::time::Duration;

use tokio::sync::mpsc;
use windows::core::{AgileReference, Interface, Result, RuntimeType};
use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{GlobalSystemMediaTransportControlsSession as Session, GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status};
use windows::Media::MediaPlaybackType;
use windows::Storage::Streams::IRandomAccessStreamReference;

use super::card::Timeline;
use super::sessions::Candidate;

const READ_TIMEOUT: Duration = Duration::from_secs(2);
/// 100 ns ticks between 1601-01-01 (Windows) and 1970-01-01 (Unix).
const EPOCH_DIFFERENCE: i64 = 116_444_736_000_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Media,
    Playback,
    Timeline,
}

/// "Something changed": in one session (`Some(id)`), or in the list of sessions (`None`).
#[derive(Debug, Clone, Copy)]
pub(super) struct Event {
    pub(super) session: Option<u64>,
    pub(super) kind: Kind,
}

/// A session we're subscribed to, and what we've seen of it.
pub(super) struct Tracked {
    pub(super) id: u64,
    pub(super) session: Session,
    pub(super) aumid: String,
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
    pub(super) fn subscribe(id: u64, session: Session, aumid: String, tx: &mpsc::UnboundedSender<Event>, now: u64) -> Result<Self> {
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

    pub(super) fn unsubscribe(&self) {
        let [media, playback, timeline] = self.tokens;
        _ = self.session.RemoveMediaPropertiesChanged(media);
        _ = self.session.RemovePlaybackInfoChanged(playback);
        _ = self.session.RemoveTimelinePropertiesChanged(timeline);
    }

    /// Notes the playback status: when it started playing, when it last played.
    pub(super) fn observe(&mut self, now: u64) {
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

    pub(super) fn noted(&mut self, media: &Media) {
        self.titled |= !media.title.trim().is_empty();
        self.video = media.video;
    }

    pub(super) fn eligible(&self, show_video: bool) -> bool {
        self.titled && !self.closed && (show_video || !self.video)
    }

    pub(super) fn candidate(&self, current: bool, show_video: bool) -> Candidate {
        Candidate { id: self.id, playing: self.playing, started_at: self.started_at, last_played: self.last_played, current, eligible: self.eligible(show_video) }
    }
}

/// What a player says about its track.
pub(super) struct Media {
    pub(super) title: String,
    pub(super) artist: String,
    pub(super) album: String,
    pub(super) thumbnail: Option<AgileReference<IRandomAccessStreamReference>>,
    pub(super) video: bool,
}

/// An event handler that just says "something changed", and where.
pub(super) fn notify<S: RuntimeType + 'static, A: RuntimeType + 'static>(tx: &mpsc::UnboundedSender<Event>, session: Option<u64>, kind: Kind) -> TypedEventHandler<S, A> {
    let tx = tx.clone();
    TypedEventHandler::new(move |_, _| {
        _ = tx.send(Event { session, kind });
        Ok(())
    })
}

/// Sessions are the same player when their object or app agree (a new proxy for the same
/// session must not count as another player).
pub(super) fn same_session(a: &Session, b: &Tracked) -> bool {
    a.as_raw() == b.session.as_raw() || a.SourceAppUserModelId().is_ok_and(|x| x == b.aumid.as_str())
}

/// The player's metadata, giving up on a player that doesn't answer.
pub(super) async fn read_media(session: &Session) -> Option<Media> {
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
pub(super) fn seek_ticks(session: &Session, ms: u64) -> i64 {
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
pub(super) fn timeline(session: &Session, playing: bool, playing_since: Option<u64>, now: u64, seekable: bool, rate: f32) -> Option<Timeline> {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
