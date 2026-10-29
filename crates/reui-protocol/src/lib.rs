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

pub const PROTOCOL_VERSION: u32 = 18;
const MAX_FRAME: usize = 16 * 1024 * 1024;

/// Per-user pipe name; the hub also restricts the pipe's ACL to the current user. A test
/// hub (`REUI_CONFIG_DIR` set: its own files) gets its own pipe, so it runs beside the
/// real ReUI without either seeing the other's surfaces.
pub fn pipe_name() -> String {
    let user = std::env::var("USERNAME").unwrap_or_else(|_| "default".into());
    let sandbox = std::env::var_os("REUI_CONFIG_DIR").is_some_and(|v| !v.is_empty());
    format!(r"\\.\pipe\reui-hub-{user}{}", if sandbox { "-sandbox" } else { "" })
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
    /// A horizontal level (volume, brightness), 0..=1: an accent fill on a track. `dim`
    /// draws it muted (the level is kept but not in effect). The surface eases between
    /// values, so a level that keeps changing reads as one movement.
    Level { value: f32, dim: bool },
    /// Time that runs on by itself (a timer, a stopwatch): tabular numerals the surface
    /// advances, so the hub sends it only when something changes. Dimmed while stopped.
    Clock(Clock),
    /// A ring in the accent colour showing how much of `total_ms` the clock has left
    /// (counting down) or has run (counting up), advancing by itself.
    ClockRing { clock: Clock, total_ms: u64 },
}

/// A time value that changes by itself: `value_ms` was true at `at_unix_ms`, and while
/// `running` it moves down (`down`, a countdown, stopping at zero) or up from there.
/// Surfaces show it rounded the way people read clocks: a countdown rounds up (the last
/// second reads 0:01 and 0:00 only at the end), a count-up rounds down.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clock {
    pub value_ms: u64,
    pub at_unix_ms: u64,
    pub running: bool,
    pub down: bool,
}

impl Clock {
    /// The value at `now_unix_ms`; a countdown never goes below zero.
    pub fn value_at(&self, now_unix_ms: u64) -> u64 {
        if !self.running {
            return self.value_ms;
        }
        let elapsed = now_unix_ms.saturating_sub(self.at_unix_ms);
        if self.down {
            self.value_ms.saturating_sub(elapsed)
        } else {
            self.value_ms + elapsed
        }
    }

    /// What it reads at `now_unix_ms` in whole steps of `step_ms` (1000 for seconds, 100
    /// for tenths): a countdown rounds up, a count-up rounds down.
    pub fn shown(&self, now_unix_ms: u64, step_ms: u64) -> u64 {
        let v = self.value_at(now_unix_ms);
        if self.down { v.div_ceil(step_ms) } else { v / step_ms }
    }

    /// Milliseconds until what it reads (in steps of `step_ms`) next changes, or `None`
    /// when it won't (stopped, or a countdown at zero).
    pub fn next_change(&self, now_unix_ms: u64, step_ms: u64) -> Option<u64> {
        if !self.running {
            return None;
        }
        let v = self.value_at(now_unix_ms);
        match (self.down, v % step_ms) {
            (true, 0) if v == 0 => None,
            (true, 0) => Some(step_ms),
            (true, r) => Some(r),
            (false, r) => Some(step_ms - r),
        }
    }
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
    /// A [`Clock`] as the hero numerals; `tenths` adds a tenths digit (a stopwatch).
    Clock { clock: Clock, tenths: bool },
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
    /// current one. `buttons` sit at its right end (icon buttons). `placeholder` shows
    /// while it's empty ("Search" when `None`), `glyph` replaces the magnifier. Enter with
    /// no card to choose sends [`ACTION_SUBMIT`] (after the module has seen the text).
    Search { query: String, buttons: Vec<Button>, placeholder: Option<String>, glyph: Option<char> },
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
    /// A list to choose from (an agent's question). A click sends the choice's `action`;
    /// with `multi` the module answers by publishing again with `selected` flipped, so the
    /// surface never keeps a selection of its own. Keyboard (once the card has it): ↑/↓
    /// move, 1–9 pick, Space picks the one under the cursor, Enter picks it (single) or
    /// presses the primary button (multi).
    Choices { options: Vec<Choice>, multi: bool },
    /// A body of text under the header (a full command, an edit's changed lines, a plan)
    /// that scrolls inside the card with the wheel instead of growing it.
    Block { text: String, kind: BlockKind },
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Choice {
    pub label: String,
    /// A quieter line under the label (the option's description).
    pub detail: Option<String>,
    /// Checked, in a multi-select list.
    pub selected: bool,
    /// Sent back to the owning module when the choice is clicked or picked.
    pub action: String,
}

/// How a [`Row::Block`] reads.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    /// Wrapped text (a plan).
    Prose,
    /// Monospace lines, cut rather than wrapped (a script).
    Code,
    /// Monospace lines where `+` and `-` (or `−`) lines are additions and removals.
    Diff,
}

/// Which line of a [`BlockKind::Diff`] is which: `Positive` for additions, `Negative` for
/// removals, `Secondary` for the rest.
pub fn diff_tone(line: &str) -> Tone {
    match line.chars().next() {
        Some('+') => Tone::Positive,
        Some('-' | '\u{2212}') => Tone::Negative,
        _ => Tone::Secondary,
    }
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
/// Enter in a search field with no cards to choose from: act on what was typed (start the
/// timer it describes).
pub const ACTION_SUBMIT: &str = "submit";
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

/// How wide an expanded card asks to be; the widths are the surface's.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CardSize {
    #[default]
    Regular,
    /// A little wider, for a moment with a session strip and a body (agent moments).
    Roomy,
    /// Rich content (a row of preview cards); a view of only rows has no header.
    Wide,
}

/// One quiet line above the header saying whose card this is: a mark, a title and a few
/// facts ("ReUI", ["fix the build", "4m"]), so two sessions of one agent never get confused.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Context {
    pub mark: Element,
    pub title: String,
    pub facts: Vec<String>,
}

/// Expanded presentation. Header: `leading` | eyebrow, headline, detail | `trailing`;
/// then full-width rows. The text column always reads in three levels: a small accent
/// eyebrow (source or category), the hero headline, and a quieter detail line.
/// A single row of glyph-only buttons is shown inline beside the hero instead of as a row.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ExpandedView {
    /// Whose card this is, drawn above the header (the pager moves onto it).
    pub context: Option<Context>,
    pub leading: Element,
    pub eyebrow: Option<String>,
    pub headline: Headline,
    pub detail: Option<String>,
    pub trailing: Element,
    pub rows: Vec<Row>,
    pub size: CardSize,
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
    /// The Settings window (`reui-settings`): sends [`ClientMsg::Command`]s (a module's
    /// `reload` after it wrote that module's settings file, `clear`, ...) and never
    /// receives snapshots.
    Settings,
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
        let expanded = self.expanded.iter().flat_map(|e| [Some(&e.leading), Some(&e.trailing), e.context.as_ref().map(|c| &c.mark)].into_iter().flatten());
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
    use super::Clock;

    #[test]
    fn countdown_rounds_up_and_stops_at_zero() {
        let c = Clock { value_ms: 10_000, at_unix_ms: 1_000, running: true, down: true };
        assert_eq!(c.shown(1_000, 1000), 10);
        assert_eq!(c.shown(1_001, 1000), 10);
        assert_eq!(c.shown(1_000 + 9_400, 1000), 1);
        assert_eq!(c.shown(1_000 + 10_000, 1000), 0);
        assert_eq!(c.value_at(1_000 + 60_000), 0);
        assert_eq!(c.next_change(1_000 + 9_400, 1000), Some(600));
        assert_eq!(c.next_change(1_000 + 2_000, 1000), Some(1000));
        assert_eq!(c.next_change(1_000 + 10_000, 1000), None);
    }

    #[test]
    fn count_up_rounds_down() {
        let c = Clock { value_ms: 0, at_unix_ms: 0, running: true, down: false };
        assert_eq!(c.shown(999, 1000), 0);
        assert_eq!(c.shown(1_000, 1000), 1);
        assert_eq!(c.shown(1_250, 100), 12);
        assert_eq!(c.next_change(1_250, 1000), Some(750));
        let stopped = Clock { running: false, ..c };
        assert_eq!(stopped.next_change(5_000, 1000), None);
        assert_eq!(stopped.value_at(5_000), 0);
    }

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
                    context: Some(Context { mark: Element::Art(Art::Logo { image: ImageId(7), animated: false }), title: "ReUI".into(), facts: vec!["fix the build".into(), "4m".into()] }),
                    size: CardSize::Roomy,
                    tint: false,
                    pager: None,
                    open: None,
                    rows: vec![
                        Row::Progress { fraction: 0.4, leading: Some("1:24".into()), trailing: None },
                        Row::Timeline { position_ms: 1_000, length_ms: 60_000, playing: true, at_unix_ms: 5, seekable: true, rate: 1.0 },
                        Row::Buttons(vec![Button { action: "pause".into(), content: ButtonContent::Glyph('\u{E769}'), role: ButtonRole::Accent }]),
                        Row::Choices { options: vec![Choice { label: "React".into(), detail: Some("A UI library".into()), selected: true, action: "pick:0".into() }], multi: true },
                        Row::Block { text: "+ added\n- removed".into(), kind: BlockKind::Diff },
                    ],
                }),
            }],
        });
        let HubMsg::Snapshot(shown) = &snapshot else { unreachable!() };
        assert_eq!(shown.activities[0].images().collect::<Vec<_>>(), vec![ImageId(7)], "the strip's mark is an image to send");
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
    fn diff_lines_take_their_tones() {
        assert_eq!(diff_tone("+ let x = 1;"), Tone::Positive);
        assert_eq!(diff_tone("- let x = 0;"), Tone::Negative);
        assert_eq!(diff_tone("\u{2212} gone"), Tone::Negative);
        assert_eq!(diff_tone("  context"), Tone::Secondary);
        assert_eq!(diff_tone(""), Tone::Secondary);
    }

    #[test]
    fn priority_orders_decisions_first() {
        assert!(Priority::Decision > Priority::Alert && Priority::Alert > Priority::Ongoing && Priority::Ongoing > Priority::Passive);
    }
}
