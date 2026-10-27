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

pub const PROTOCOL_VERSION: u32 = 14;
const MAX_FRAME: usize = 16 * 1024 * 1024;

/// Per-user pipe name; the hub also restricts the pipe's ACL to the current user.
pub fn pipe_name() -> String {
    let user = std::env::var("USERNAME").unwrap_or_else(|_| "default".into());
    format!(r"\\.\pipe\reui-hub-{user}")
}

/// What decides which activity is in front. Higher wins; ties go to the newest.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    /// Background info (stats, a history card nobody asked for).
    Passive,
    /// Something in progress (music, timer, agent working).
    Ongoing,
    /// A moment's confirmation (copied). Takes the pill only when nothing else is there;
    /// otherwise it shows in the minimal bubble beside what's playing, then goes.
    Brief,
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
    /// Audio level bars in the accent colour; `live` means something is playing. When the
    /// activity names an `audio` process, a surface may drive them from that audio.
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

/// What a preview card shows.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum Preview {
    Text(String),
    /// Monospace text.
    Code(String),
    Image(ImageId),
    Color(Rgb),
    /// A web page; `title`, `site` and `image` once its metadata has been fetched.
    Link { host: String, path: String, title: Option<String>, site: Option<String>, image: Option<ImageId> },
    Files { name: String, count: u32 },
}

/// A clickable preview card (clipboard history and the like).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Item {
    /// Sent back to the owning module when the card is clicked.
    pub action: String,
    pub preview: Preview,
    /// Icon of the app it came from.
    pub icon: Option<ImageId>,
    /// How long ago ("2 min").
    pub age: String,
    /// One fact about the content ("1920 × 1080", "12 lines").
    pub meta: Option<String>,
    /// Kept by the user (shown with a small star).
    pub pinned: bool,
    /// More things to do with it: the card's context menu, and their keyboard shortcuts
    /// while the card is selected. `action` above is the click / Enter.
    pub actions: Vec<ItemAction>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ItemAction {
    /// Menu label ("Paste as plain text").
    pub label: String,
    /// Sent back to the owning module.
    pub action: String,
    pub key: Option<ItemKey>,
}

/// Keyboard shortcuts a selected card's actions can claim.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKey {
    ShiftEnter,
    CtrlEnter,
    Delete,
    CtrlP,
}

/// A filter tab over a row of cards ("Links 12").
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Tab {
    pub label: String,
    pub count: u32,
    pub selected: bool,
    /// Sent back when the tab is chosen.
    pub action: String,
}

/// A full-width row under the expanded header.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum Row {
    Progress { fraction: f32, leading: Option<String>, trailing: Option<String> },
    /// Media position the surface advances by itself while `playing`, so the hub sends it
    /// only when something changes. `position_ms` was true at `at_unix_ms`. When `seekable`,
    /// clicking the bar sends the action `seek:<ms>`.
    /// `rate` is the playback speed (1.0 normal, 1.5 for a sped-up podcast). A `length_ms` of
    /// 0 is a live stream: no bar, a "LIVE" label. While dragging, the surface previews the
    /// position and sends one `seek:<ms>` on release.
    Timeline { position_ms: u64, length_ms: u64, playing: bool, at_unix_ms: u64, seekable: bool, rate: f32 },
    Buttons(Vec<Button>),
    /// A row of preview cards, scrolling sideways when there are more than fit. Keyboard:
    /// ←/→ select, Enter sends the selected card's `action`, its `actions` claim their keys.
    Items(Vec<Item>),
    /// A search field. The surface owns the text while typing and sends
    /// `search:<query>` (see [`ACTION_SEARCH`]) as it changes; `query` is the module's
    /// current one. `buttons` sit at its right end (icon buttons).
    Search { query: String, buttons: Vec<Button> },
    /// Filter tabs; Tab / Shift+Tab move between them.
    Tabs(Vec<Tab>),
    /// One quiet line, for when there's nothing else to show ("No matches").
    Note(String),
    /// A level (volume): `leading` toggles (mute), dragging or the wheel sends
    /// `<action><0..1 value>` (e.g. `volume:0.42`), `trailing` opens a menu (output device).
    Slider { leading: Button, value: f32, action: String, trailing: Option<Menu> },
    /// Synced lyrics, timed against the view's `Timeline`; the surface shows the current
    /// line and moves on by itself.
    Lyrics(Vec<LyricLine>),
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct LyricLine {
    /// Position in the track where the line starts.
    pub at_ms: u64,
    pub text: String,
}

/// A button that opens a list of choices.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Menu {
    pub content: ButtonContent,
    pub options: Vec<MenuOption>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct MenuOption {
    pub label: String,
    pub action: String,
    pub selected: bool,
}

/// "1 of 2" between alternatives of one activity (players), with ‹ › to move.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Pager {
    pub index: u32,
    pub count: u32,
    pub prev: String,
    pub next: String,
}

/// Actions a surface sends on its own, about the front activity: its card opened or closed.
/// Modules that don't care ignore them.
pub const ACTION_EXPANDED: &str = "expanded";
pub const ACTION_COLLAPSED: &str = "collapsed";
/// Prefix of the action a [`Row::Search`] field sends: `search:<query>`.
pub const ACTION_SEARCH: &str = "search:";
/// Ctrl+Z in a card that has a search field: undo the module's last removal.
pub const ACTION_UNDO: &str = "undo";
/// A horizontal swipe (trackpad) on the compact pill, to the front activity: next / previous.
pub const ACTION_SWIPE_NEXT: &str = "swipe:next";
pub const ACTION_SWIPE_PREV: &str = "swipe:prev";

/// Milliseconds since the Unix epoch, the clock [`Row::Timeline`] is stamped with.
pub fn unix_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

impl Row {
    /// Where a timeline is at `now_unix_ms`, clamped to its length.
    pub fn timeline_position(position_ms: u64, length_ms: u64, playing: bool, at_unix_ms: u64, rate: f32, now_unix_ms: u64) -> u64 {
        let elapsed = (now_unix_ms.saturating_sub(at_unix_ms) as f64 * f64::from(rate.max(0.0))) as u64;
        let advanced = if playing { position_ms + elapsed } else { position_ms };
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
    /// Ask for the wide expanded island (rich content like preview cards).
    pub wide: bool,
    /// Wash the card with the accent from the top-left (artwork colour), no blur.
    pub tint: bool,
    /// Switch between alternatives (players), shown at the top right.
    pub pager: Option<Pager>,
    /// Clicking the leading art sends this (open the player).
    pub open: Option<String>,
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
    /// The process whose sound this activity is about (a music player), for audio-reactive
    /// visuals. The surface captures it only while the bars are live and shown.
    pub audio: Option<u32>,
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
    /// A command for a module whether or not it has an activity showing (tray menu:
    /// "Pause clipboard history"). The module gets it as an action with an empty key.
    Command { module: String, action: String },
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
    /// Bring this activity to the front and open it (e.g. a hotkey asked for it). Sent after
    /// the snapshot that contains it.
    Focus { id: ActivityId },
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
        let elements = [&self.compact.leading, &self.compact.trailing, &self.minimal].into_iter().chain(expanded).filter_map(Element::image);
        let cards = self.expanded.iter().flat_map(|e| &e.rows).flat_map(|row| match row {
            Row::Items(items) => items
                .iter()
                .flat_map(|i| {
                    let preview = match &i.preview {
                        Preview::Image(id) => Some(*id),
                        Preview::Link { image, .. } => *image,
                        _ => None,
                    };
                    preview.into_iter().chain(i.icon)
                })
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        });
        elements.chain(cards)
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
                audio: None,
                expanded: Some(ExpandedView {
                    leading: Element::Ring(0.4),
                    eyebrow: Some("Timer".into()),
                    headline: Headline::Value("01:45".into()),
                    detail: None,
                    trailing: Element::Glyph { glyph: '\u{E916}', tone: Tone::Accent },
                    wide: false,
                    tint: false,
                    pager: None,
                    open: None,
                    rows: vec![
                        Row::Progress { fraction: 0.4, leading: Some("1:24".into()), trailing: None },
                        Row::Timeline { position_ms: 1_000, length_ms: 60_000, playing: true, at_unix_ms: 5, seekable: true, rate: 1.0 },
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
        assert_eq!(Row::timeline_position(10_000, 60_000, true, 1_000, 1.0, 4_000), 13_000);
        assert_eq!(Row::timeline_position(10_000, 60_000, true, 1_000, 1.5, 3_000), 13_000);
        assert_eq!(Row::timeline_position(10_000, 60_000, false, 1_000, 1.0, 4_000), 10_000);
        assert_eq!(Row::timeline_position(59_000, 60_000, true, 1_000, 1.0, 9_000), 60_000);
    }

    #[test]
    fn priority_orders_decisions_first() {
        assert!(Priority::Decision > Priority::Alert && Priority::Alert > Priority::Ongoing && Priority::Ongoing > Priority::Passive);
    }
}
