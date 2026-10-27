//! Wire protocol between the hub and its surfaces (the island now; dock, taskbar later).
//!
//! Surfaces never know which module produced an activity. Modules describe *what* to show
//! with the semantic view model below, shaped after Apple's Live Activity presentations
//! (compact, minimal, expanded); surfaces decide *how* it looks from the design tokens.
//! Nothing here is a pixel, font or layout value, so a visual redesign never touches it.
//! Frames are a little-endian `u32` length followed by a postcard-encoded message.

use std::io;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const PROTOCOL_VERSION: u32 = 7;
const MAX_FRAME: usize = 16 * 1024 * 1024;

/// Per-user pipe name; the hub also restricts the pipe's ACL to the current user.
pub fn pipe_name() -> String {
    let user = std::env::var("USERNAME").unwrap_or_else(|_| "default".into());
    format!(r"\\.\pipe\reui-hub-{user}")
}

/// What decides which activity is in front. Higher wins; ties go to the newest.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    /// Background info (clipboard toast, stats).
    Passive,
    /// Something in progress (music, timer, agent working).
    Ongoing,
    /// Needs attention but not an answer (agent finished, download done).
    Alert,
    /// Needs the user's decision (permission request).
    Decision,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ActivityId {
    pub module: String,
    pub key: String,
}

/// Content colour (album art, brand colour), not a theme colour.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

/// Semantic colour role; the surface maps it to a token.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tone {
    #[default]
    Primary,
    Secondary,
    /// The activity's accent.
    Accent,
    Positive,
    Warning,
    Negative,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum Art {
    /// Placeholder artwork until real images are sent.
    Gradient { from: Rgb, to: Rgb },
    /// A Segoe Fluent Icons glyph on a tile tinted with the activity's accent.
    Glyph(char),
    /// An image sent earlier as [`HubMsg::Image`], shown cover-fitted.
    Image(ImageId),
    /// A brand mark (single-colour SVG sent as [`HubMsg::Image`]) on an accent-tinted tile,
    /// drawn in the accent colour. `animated` asks for the mark's working animation; the
    /// surface decides when it may actually run.
    Logo { image: ImageId, animated: bool },
}

/// Identifies image bytes the hub sends once per surface, ahead of the first snapshot that
/// uses them, so snapshots (resent on every change) stay small.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImageId(pub u64);

/// The building blocks a region can hold.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub enum Element {
    #[default]
    Empty,
    Text { text: String, tone: Tone },
    /// A Segoe Fluent Icons glyph.
    Glyph { glyph: char, tone: Tone },
    Art(Art),
    /// Circular progress in the accent colour, 0..=1.
    Ring(f32),
    /// Audio level bars in the accent colour; `live` means something is playing.
    Bars { live: bool },
    /// Small status dot in the accent colour.
    Dot,
}

/// Compact presentation: two sides around the island's centre.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct CompactView {
    pub leading: Element,
    pub trailing: Element,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum Headline {
    /// Numerals (timers, prices, scores): large, tabular.
    Value(String),
    Text(String),
    /// Commands, paths and code: monospace.
    Code(String),
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonRole {
    /// The one main action.
    Primary,
    /// Grey fill.
    Secondary,
    /// Tinted with the activity's accent.
    Accent,
    Destructive,
    /// No fill at all: media transport (previous / next).
    Plain,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum ButtonContent {
    Glyph(char),
    Text(String),
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Button {
    /// Sent back to the owning module when pressed.
    pub action: String,
    pub content: ButtonContent,
    pub role: ButtonRole,
}

/// A full-width row under the expanded header.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum Row {
    Progress { fraction: f32, leading: Option<String>, trailing: Option<String> },
    /// Media position the surface advances by itself while `playing`, so the hub sends it
    /// only when something changes. `position_ms` was true at `at_unix_ms`. When `seekable`,
    /// clicking the bar sends the action `seek:<ms>`.
    Timeline { position_ms: u64, length_ms: u64, playing: bool, at_unix_ms: u64, seekable: bool },
    Buttons(Vec<Button>),
}

/// Milliseconds since the Unix epoch, the clock [`Row::Timeline`] is stamped with.
pub fn unix_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

impl Row {
    /// Where a timeline is at `now_unix_ms`, clamped to its length.
    pub fn timeline_position(position_ms: u64, length_ms: u64, playing: bool, at_unix_ms: u64, now_unix_ms: u64) -> u64 {
        let advanced = if playing { position_ms + now_unix_ms.saturating_sub(at_unix_ms) } else { position_ms };
        advanced.min(length_ms)
    }
}

/// Expanded presentation. Header: `leading` | eyebrow, headline, detail | `trailing`;
/// then full-width rows. The text column always reads in three levels: a small accent
/// eyebrow (source or category), the hero headline, and a quieter detail line.
/// A single row of glyph-only buttons is shown inline beside the hero instead of as a row.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ExpandedView {
    pub leading: Element,
    pub eyebrow: Option<String>,
    pub headline: Headline,
    pub detail: Option<String>,
    pub trailing: Element,
    pub rows: Vec<Row>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Activity {
    pub id: ActivityId,
    pub priority: Priority,
    /// Tints rings, bars, dots, glyph tiles and accent buttons; the surface's default if `None`.
    pub accent: Option<Rgb>,
    pub compact: CompactView,
    /// Shown in the detached bubble while another activity is in front.
    pub minimal: Element,
    pub expanded: Option<ExpandedView>,
}

/// Everything a surface needs, front activity first.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    pub activities: Vec<Activity>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceKind {
    Island,
    /// A short-lived agent hook bridge (`reui-bridge`): sends one [`ClientMsg::Hook`],
    /// gets at most one [`HubMsg::HookReply`], and never receives snapshots.
    Bridge,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum ClientMsg {
    Hello { version: u32, surface: SurfaceKind },
    Action { id: ActivityId, action: String },
    /// A coding agent's hook event, forwarded verbatim. `agent` names the module that
    /// handles it ("claude"); `payload` is the hook's stdin JSON. When `wants_reply` is set
    /// the bridge waits for a [`HubMsg::HookReply`].
    Hook { agent: String, payload: String, wants_reply: bool },
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum HubMsg {
    Welcome { version: u32 },
    Rejected { reason: String },
    /// Encoded image bytes (PNG, JPEG, ...) for [`Art::Image`].
    Image { id: ImageId, bytes: Vec<u8> },
    Snapshot(Snapshot),
    /// What the bridge prints to the agent on stdout; `None` means "no decision" and the
    /// agent continues with its own flow.
    HookReply { output: Option<String> },
}

impl Element {
    fn image(&self) -> Option<ImageId> {
        match self {
            Element::Art(Art::Image(id) | Art::Logo { image: id, .. }) => Some(*id),
            _ => None,
        }
    }
}

impl Activity {
    /// Every image this activity shows, in any presentation.
    pub fn images(&self) -> impl Iterator<Item = ImageId> + '_ {
        let expanded = self.expanded.iter().flat_map(|e| [&e.leading, &e.trailing]);
        [&self.compact.leading, &self.compact.trailing, &self.minimal].into_iter().chain(expanded).filter_map(Element::image)
    }
}

pub async fn write_frame<W: AsyncWrite + Unpin, T: Serialize>(w: &mut W, msg: &T) -> io::Result<()> {
    let body = postcard::to_stdvec(msg).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if body.len() > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut frame = Vec::with_capacity(4 + body.len());
    frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
    frame.extend_from_slice(&body);
    w.write_all(&frame).await?;
    w.flush().await
}

/// Not cancel-safe: run it in its own task or loop, never as one arm of a `select!`.
pub async fn read_frame<R: AsyncRead + Unpin, T: DeserializeOwned>(r: &mut R) -> io::Result<T> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len).await?;
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).await?;
    postcard::from_bytes(&body).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn frames_round_trip() {
        let (mut a, mut b) = tokio::io::duplex(1024);
        let snapshot = HubMsg::Snapshot(Snapshot {
            activities: vec![Activity {
                id: ActivityId { module: "demo".into(), key: "timer".into() },
                priority: Priority::Ongoing,
                accent: Some(Rgb(10, 132, 255)),
                compact: CompactView { leading: Element::Ring(0.4), trailing: Element::Text { text: "01:45".into(), tone: Tone::Primary } },
                minimal: Element::Ring(0.4),
                expanded: Some(ExpandedView {
                    leading: Element::Ring(0.4),
                    eyebrow: Some("Timer".into()),
                    headline: Headline::Value("01:45".into()),
                    detail: None,
                    trailing: Element::Glyph { glyph: '\u{E916}', tone: Tone::Accent },
                    rows: vec![
                        Row::Progress { fraction: 0.4, leading: Some("1:24".into()), trailing: None },
                        Row::Timeline { position_ms: 1_000, length_ms: 60_000, playing: true, at_unix_ms: 5, seekable: true },
                        Row::Buttons(vec![Button { action: "pause".into(), content: ButtonContent::Glyph('\u{E769}'), role: ButtonRole::Accent }]),
                    ],
                }),
            }],
        });
        write_frame(&mut a, &snapshot).await.unwrap();
        write_frame(&mut a, &HubMsg::Welcome { version: PROTOCOL_VERSION }).await.unwrap();
        assert_eq!(read_frame::<_, HubMsg>(&mut b).await.unwrap(), snapshot);
        assert_eq!(read_frame::<_, HubMsg>(&mut b).await.unwrap(), HubMsg::Welcome { version: PROTOCOL_VERSION });
    }

    #[test]
    fn timelines_advance_only_while_playing() {
        assert_eq!(Row::timeline_position(10_000, 60_000, true, 1_000, 4_000), 13_000);
        assert_eq!(Row::timeline_position(10_000, 60_000, false, 1_000, 4_000), 10_000);
        assert_eq!(Row::timeline_position(59_000, 60_000, true, 1_000, 9_000), 60_000);
    }

    #[test]
    fn priority_orders_decisions_first() {
        assert!(Priority::Decision > Priority::Alert && Priority::Alert > Priority::Ongoing && Priority::Ongoing > Priority::Passive);
    }
}
