//! The pill: its state, and how hub snapshots map onto presentations (idle core, compact,
//! peek, expanded, plus the minimal bubble). Building it, input, summoning, visibility,
//! timers and self-animated visuals live in the child modules.
//!
//! Animated values live in one property set: the pill's width `w` and height `h`, the
//! minimal bubble's presence `m`, and the current expanded height `eh`. Everything else
//! (clip, centring, cross-fades) is an expression over them, so morphs run inside DWM.
//! Layer offsets are rounded to whole pixels: a fractional offset makes the compositor
//! resample the content bitmaps, which visibly softens thin strokes and text.

use std::collections::HashSet;
use std::sync::mpsc::Receiver;
use std::time::Instant;

use reui_design::tokens::{color, motion};
use reui_protocol::{
    Activity, ActivityId, ClientMsg, ExpandedView, ImageId, Item, ItemKey, Priority, Row, Snapshot,
};
use tokio::sync::mpsc::UnboundedSender;
use windows::core::{h, Result};
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::features::choices;
use crate::features::clipboard::{self, Key, Panel};
use crate::features::clipboard_keys;
use crate::features::level::Levels;
use crate::features::music::{self, SeekHold, Swipe, Throttle};
use crate::features::visualizer::Visualizer;
use crate::ipc::Event;
use crate::render::{self, Hit, Interaction, Renderer, Thumb};
use crate::drag_guard::DragGuard;
use crate::place::MonitorChoice;
use crate::settings::Visibility;
use crate::{images, order};

mod input;
mod setup;
mod summon;
mod timers;
mod visibility;
mod visuals;

use self::timers::*;

/// A press on a bar in the open card, followed with the mouse captured until release.
enum Drag {
    /// The scrubber: previews `ms`, seeks once on release.
    Seek { left: f32, right: f32, length: u64, ms: u64 },
    /// A slider: sends its value as it moves (throttled) and once more on release.
    Slider { left: f32, right: f32, action: String, value: f32, throttle: Throttle },
}

/// A context menu entry: label, action, whether picking it closes a summoned panel, and
/// whether it shows checked.
pub type MenuEntry = (String, String, bool, bool);

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum State {
    Idle,
    Compact,
    Peek,
    Expanded,
}

pub struct Pill {
    hwnd: HWND,
    scale: f32,
    window: RECT,
    monitor: HMONITOR,
    reduced_motion: bool,
    /// The lyric line last drawn, and the one before it with when it changed (while the
    /// column slides up).
    lyric_line: Option<usize>,
    lyric_change: Option<(usize, Instant)>,
    /// Levels (volume, brightness) as drawn, easing to new values on `TIMER_LEVEL`, which
    /// runs only while one is moving.
    levels: Levels,
    level_ticking: bool,
    compositor: Compositor,
    _target: DesktopWindowTarget,
    root: ContainerVisual,
    props: CompositionPropertySet,
    renderer: Renderer,
    compact_surface: CompositionDrawingSurface,
    expanded_surface: CompositionDrawingSurface,
    minimal_surface: CompositionDrawingSurface,
    /// Level bars in the compact pill and in the open card.
    bars: Vec<SpriteVisual>,
    card_bars: Vec<SpriteVisual>,
    /// Bars inside the minimal bubble, in the second activity's accent.
    minimal_bars: Vec<SpriteVisual>,
    bar_brush: CompositionColorBrush,
    minimal_brush: CompositionColorBrush,
    bars_clock: std::time::Instant,
    /// Whether the front / minimal activity shows bars, and whether they're live.
    front_bars: Option<bool>,
    minimal_bars_live: Option<bool>,
    bars_moving: bool,
    /// The processes the front and minimal activities' bars are about (`Activity::audio`).
    front_audio: Option<u32>,
    minimal_audio: Option<u32>,
    /// Real band levels for moving bars, captured only while they move.
    visualizer: Visualizer,
    logo_surface: CompositionDrawingSurface,
    logo: SpriteVisual,
    /// Animated compact logo: (image, currently ticking).
    logo_state: Option<(ImageId, bool)>,
    /// Current logo angle, so each tick turns on from where the last one stopped.
    logo_angle: f32,
    compact_width: f32,
    expanded_height: f32,
    expanded_width: f32,
    hits: Vec<Hit>,
    /// The hub's snapshot as received.
    raw: Snapshot,
    /// What's shown: `raw` with the user's chosen activity moved to the front.
    snapshot: Snapshot,
    /// The activity the user brought to the front (bubble click or wheel). Holds until
    /// something more urgent arrives, and comes back after it's gone.
    pinned: Option<ActivityId>,
    /// The pinned card was opened on request (a hotkey): it shows over urgent activities
    /// and stays open until dismissed (click outside, another app, the hotkey again, or
    /// `delay.panel-idle` untouched). Urgent ones come back after.
    summoned: bool,
    /// The pointer is over the minimal bubble rather than the pill.
    on_bubble: bool,
    /// The activity whose card is open, as last told to its module.
    open_id: Option<ActivityId>,
    state: State,
    /// Size the springs are heading to, in DIPs.
    target: (f32, f32),
    minimal: bool,
    /// Pointer is over the pill's area.
    inside: bool,
    /// Pointer has rested on the pill long enough to mean it: the pill takes clicks only then,
    /// so quick moves and clicks reach whatever is underneath (browser tabs, title bars).
    hovered: bool,
    /// A fullscreen app is in front.
    hidden: bool,
    /// The desktop is in front rather than an app (see `desktop.rs`).
    on_desktop: bool,
    /// The `show_on_desktop` setting: on the desktop, show as if visibility were Always.
    show_on_desktop: bool,
    /// The `hide_in_fullscreen` setting.
    hide_in_fullscreen: bool,
    /// The `monitor` setting.
    monitor_choice: MonitorChoice,
    /// A drag from elsewhere is passing over: hover intent doesn't count.
    drag_guard: DragGuard,
    visibility: Visibility,
    /// "Pause ReUI" is on (settings.json): hidden whatever happens until it ends.
    paused: bool,
    /// Pushed to the top edge in a hidden mode: shown until the pointer has been away a while.
    revealed: bool,
    /// What the island currently shows (faded in), after visibility mode, reveal and fullscreen.
    shown: bool,
    clickthrough: bool,
    /// The open panel's scroll, selection and search text.
    panel: Panel,
    /// Time of the last scroll frame, while the row is easing.
    scroll_clock: Option<Instant>,
    /// Typed text the module hasn't been told yet.
    search_dirty: bool,
    /// A key that acts on the results (Enter, Alt+n...) pressed before they arrived.
    awaiting: Option<Key>,
    /// A card's context menu is up: clicks on it aren't "outside".
    menu_open: bool,
    /// The app handed the foreground back after a menu; its activation isn't the user
    /// moving on.
    ignore_foreground: Option<HWND>,
    drag: Option<Drag>,
    /// When a drag last redrew the card: high-rate mice move far more often than it's
    /// worth drawing.
    drag_drawn: Option<Instant>,
    /// A seek shown until the player confirms it (E-T6).
    seek_hold: Option<SeekHold>,
    /// A slider value set here (its action, the value), shown until the next snapshot.
    slider_hold: Option<(String, f32)>,
    /// The bar under the pointer shows its thumb.
    thumb: Option<Thumb>,
    swipe: Swipe,
    /// Wheel travel over the source switcher, toward a whole notch.
    pager_travel: i32,
    /// The open card was clicked, so it may have the keyboard (a card that asks something;
    /// see `features::choices`). Never set by a card arriving on its own.
    engaged: bool,
    /// The choice under the keyboard cursor in the open card's list.
    choice_cursor: Option<usize>,
    /// How far the open card's text block is scrolled, and how far it can go.
    block_scroll: f32,
    block_max: f32,
    to_hub: UnboundedSender<ClientMsg>,
    events: Receiver<Event>,
}

impl Pill {
    fn front(&self) -> Option<&Activity> {
        self.snapshot.activities.first()
    }

    /// Nothing is happening (no activities from the hub).
    pub fn idle(&self) -> bool {
        self.raw.activities.is_empty()
    }

    /// Hands over the hub link and the last snapshot, for a pill rebuilt at another scale
    /// (see `main::rebuild`). This pill is left without a link.
    pub fn take_link(&mut self) -> (UnboundedSender<ClientMsg>, Receiver<Event>, Snapshot) {
        let to_hub = std::mem::replace(&mut self.to_hub, tokio::sync::mpsc::unbounded_channel().0);
        let events = std::mem::replace(&mut self.events, std::sync::mpsc::channel().1);
        (to_hub, events, std::mem::take(&mut self.raw))
    }

    /// Shows `snapshot` on a freshly built pill: its images are decoded again from the
    /// encoded copies the link keeps.
    pub fn restore(&mut self, snapshot: Snapshot) {
        for id in clipboard::referenced(&snapshot) {
            if let Some(bytes) = images::get(id) {
                if let Err(e) = self.renderer.add_image(id, &bytes) {
                    eprintln!("island: can't decode image: {e}");
                }
            }
        }
        if let Err(e) = self.apply(snapshot) {
            eprintln!("island: render failed: {e}");
        }
    }

    /// Apply everything the hub link queued; only the newest snapshot matters. Messages for
    /// module windows are returned, in order, for the caller to deliver outside this borrow.
    pub fn drain_hub(&mut self) {
        let mut latest = None;
        let mut focus = None;
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Connected => eprintln!("island: connected to hub"),
                Event::Focus(id) => focus = Some(id),
                // Kept encoded; the pill decodes what a snapshot shows (below).
                Event::Image(id, bytes) => images::put(id, bytes),
                // Evict in stream order, exactly where the hub does, so images that arrived
                // for a later snapshot survive.
                Event::Snapshot(snapshot) => {
                    let shown = clipboard::referenced(&snapshot);
                    images::evict(&shown);
                    for &id in &shown {
                        if self.renderer.has_image(id) {
                            continue;
                        }
                        if let Some(bytes) = images::get(id) {
                            if let Err(e) = self.renderer.add_image(id, &bytes) {
                                eprintln!("island: can't decode image: {e}");
                            }
                        }
                    }
                    self.renderer.retain_images(&shown);
                    latest = Some(snapshot);
                }
                Event::Disconnected => {
                    images::clear();
                    self.renderer.retain_images(&HashSet::new());
                    latest = Some(Snapshot::default());
                }
            }
        }
        if let Some(snapshot) = latest {
            if let Err(e) = self.apply(snapshot) {
                eprintln!("island: render failed: {e}");
            }
        }
        if let Some(id) = focus {
            self.focus(id);
        }
    }

    fn apply(&mut self, snapshot: Snapshot) -> Result<()> {
        self.raw = snapshot;
        if self.drag.is_none() {
            self.slider_hold = None;
        }
        self.levels.sync(&self.raw.activities, Instant::now(), !self.reduced_motion && self.shown);
        self.update_levels();
        let ordered = self.ordered();
        self.render(ordered)
    }

    /// What to show where (see `order::arrange`).
    fn ordered(&mut self) -> Snapshot {
        let open = if self.state == State::Expanded { self.open_id.clone() } else { None };
        // "When needed" hides ongoing things, so a confirmation there leads instead of
        // waiting in the bubble beside something the user can't see.
        let briefs_lead = self.visibility == Visibility::WhenNeeded && !self.desktop_shows() && !self.raw.activities.iter().any(|a| a.priority >= reui_protocol::Priority::Alert);
        let mut raw = self.raw.activities.clone();
        if self.paused {
            // Paused: only decisions show (see `wants_shown`), alone in the pill.
            raw.retain(|a| a.priority == reui_protocol::Priority::Decision);
        }
        let (activities, pin_alive) = order::arrange(raw, self.pinned.as_ref(), self.summoned, open.as_ref(), briefs_lead);
        if !pin_alive {
            self.pinned = None;
            self.summoned = false;
        }
        Snapshot { activities }
    }

    /// Brings another activity to the front and re-renders.
    fn bring_forward(&mut self, id: ActivityId) {
        self.pinned = Some(id);
        let ordered = self.ordered();
        if let Err(e) = self.render(ordered) {
            eprintln!("island: render failed: {e}");
        }
    }

    fn render(&mut self, snapshot: Snapshot) -> Result<()> {
        let previous = self.front().map(|a| (a.id.clone(), a.priority));
        let previous_view = self.front_view().cloned();
        self.snapshot = snapshot;
        if self.front().map(|a| &a.id) != previous.as_ref().map(|(id, _)| id) {
            // Holds belong to the activity that was in front.
            self.seek_hold = None;
            self.slider_hold = None;
            // So does the keyboard: another card never inherits it from a click on this one.
            self.engaged = false;
            self.block_scroll = 0.0;
        }
        if !choices::same_list(previous_view.as_ref(), self.front_view()) {
            // The next step of a question starts at its first choice (shown only while the
            // card has the keyboard).
            let asks = self.front_view().is_some_and(|v| v.rows.iter().any(|r| matches!(r, Row::Choices { .. })));
            self.choice_cursor = asks.then_some(0);
            self.block_scroll = 0.0;
        }
        self.prune_hold();
        self.front_audio = self.front().and_then(|a| a.audio);
        self.minimal_audio = self.snapshot.activities.get(1).and_then(|a| a.audio);
        let front = self.snapshot.activities.first();
        self.panel.sync(front.map(|a| &a.id), front.and_then(|a| a.expanded.as_ref()), clipboard_keys::captured());
        let accent = |a: Option<&Activity>| a.and_then(|a| a.accent).map(render::rgb).unwrap_or(color::accent::DEFAULT);
        let front = self.snapshot.activities.first();
        let second = self.snapshot.activities.get(1);

        let now = Instant::now();
        let level = |a: Option<&Activity>| a.and_then(|a| self.levels.value(&a.id, now));
        let front_accent = accent(front);
        let compact = self.renderer.draw_compact(&self.compact_surface, front.map(|a| &a.compact), front_accent, level(front))?;
        let card = self.renderer.draw_expanded(&self.expanded_surface, front.and_then(|a| a.expanded.as_ref()), front_accent, &self.interaction())?;
        let second_accent = accent(second);
        let minimal_bars = self.renderer.draw_minimal(&self.minimal_surface, second.map(|a| &a.minimal), second_accent, level(second))?;
        let has_second = second.is_some();
        let front = front.map(|a| (a.id.clone(), a.priority, a.expanded.is_some()));

        self.compact_width = compact.width;
        self.place_bars(compact.bars, card.bars, minimal_bars, front_accent, second_accent)?;
        self.place_logo(compact.logo, front_accent)?;
        self.expanded_height = card.height;
        self.expanded_width = card.width;
        self.hits = card.hits;
        self.block_max = card.block_max;
        self.props.InsertScalar(h!("eh"), card.height * self.scale)?;
        self.props.InsertScalar(h!("r"), card.radius * self.scale)?;
        self.set_minimal(has_second);

        match front {
            None => self.set_state(State::Idle),
            Some((id, priority, expandable)) => {
                let is_new = previous.as_ref().map(|(old, _)| old) != Some(&id);
                let was_decision = previous.as_ref().is_some_and(|(_, p)| *p == Priority::Decision);
                if is_new && priority == Priority::Decision && expandable && !self.hidden {
                    // A decision opens itself so it's noticed, then folds back to compact (still
                    // pending) unless the user engages, so it never blocks the top of the screen.
                    self.set_state(State::Expanded);
                    if !self.hovered {
                        unsafe { SetTimer(Some(self.hwnd), TIMER_COLLAPSE, motion::delay::DECISION_HOLD, None) };
                    }
                } else if (is_new && was_decision && !self.hovered) || self.state == State::Idle {
                    // An answered decision folds away; the first activity wakes an idle pill.
                    self.set_state(State::Compact);
                } else if self.state == State::Expanded && !expandable {
                    // The card was taken away (a module folding its own card): back to compact.
                    self.set_state(State::Compact);
                }
            }
        }
        // Content may have changed size within the same state.
        self.retarget();
        self.update_tick();
        self.sync_open();
        self.refresh_visibility();
        self.update_capture();
        self.update_scroll();
        if self.awaiting.is_some() && !self.search_pending() {
            self.run_awaiting();
        }
        Ok(())
    }

    /// What the user is doing with the open panel, for the renderer.
    fn interaction(&self) -> Interaction<'_> {
        let held = self.slider_hold.as_ref().map(|(action, value)| (action.as_str(), *value, false));
        let (scrub, slider, thumb) = match &self.drag {
            Some(Drag::Seek { ms, .. }) => (Some(*ms), held, Some(Thumb::Seek)),
            Some(Drag::Slider { action, value, .. }) => (None, Some((action.as_str(), *value, true)), Some(Thumb::Slider)),
            None => (None, held, self.thumb),
        };
        Interaction {
            query: self.panel.query.as_deref(),
            keyboard: clipboard_keys::captured(),
            selected: self.panel.selected,
            scroll: self.panel.scroll,
            scrub,
            hold: self.seek_hold,
            slider,
            thumb,
            lyric: self.lyric_change.map(|(prev, at)| (prev, music::lyric_ease(at.elapsed().as_secs_f32() * 1000.0 / motion::lyrics::DURATION as f32))),
            level: self.front().and_then(|a| self.levels.value(&a.id, Instant::now())),
            block_scroll: self.block_scroll.min(self.block_max),
            choice: self.choice_cursor.filter(|_| clipboard_keys::captured()),
        }
    }

    fn front_view(&self) -> Option<&ExpandedView> {
        self.front().and_then(|a| a.expanded.as_ref())
    }

    fn items(&self) -> &[Item] {
        self.front_view().and_then(|v| v.rows.iter().find_map(|r| if let Row::Items(items) = r { Some(items.as_slice()) } else { None })).unwrap_or_default()
    }

    /// Sends an action for the front activity.
    pub fn send_action(&self, action: String) {
        if let Some(front) = self.front() {
            _ = self.to_hub.send(ClientMsg::Action { id: front.id.clone(), action });
        }
    }

    /// A module command from the menu.
    pub fn command(&self, module: &str, action: &str) {
        _ = self.to_hub.send(ClientMsg::Command { module: module.into(), action: action.into() });
    }

    /// The action card `i` claims for `key`, if any.
    fn item_action(&self, i: usize, key: ItemKey) -> Option<String> {
        self.items().get(i)?.actions.iter().find(|a| a.key == Some(key)).map(|a| a.action.clone())
    }

    /// Sends an action from the open panel. One that uses the item (paste, copy) also
    /// closes a summoned panel, so the keyboard goes straight back to the app; pin, delete,
    /// tabs and undo keep it open.
    pub fn act(&mut self, action: String, closes: bool) {
        self.send_action(action);
        if closes && self.summoned && self.state == State::Expanded {
            self.set_state(State::Compact);
        }
    }

    fn set_state(&mut self, state: State) {
        if self.state == state {
            return;
        }
        let closing = self.state == State::Expanded;
        self.state = state;
        if state != State::Expanded {
            unsafe { _ = KillTimer(Some(self.hwnd), TIMER_COLLAPSE) };
            self.summoned = false;
        }
        if closing {
            // A closed panel starts over next time; the closing frame isn't redrawn.
            self.panel.reset();
            self.engaged = false;
            self.choice_cursor = None;
            self.block_scroll = 0.0;
            self.thumb = None;
            if self.drag.take().is_some() {
                unsafe { _ = ReleaseCapture() };
            }
            self.scroll_clock = None;
            unsafe { _ = KillTimer(Some(self.hwnd), TIMER_SCROLL) };
            // Let what an open (or summoned) card was holding back come forward again,
            // once this change settles.
            unsafe { SetTimer(Some(self.hwnd), TIMER_REORDER, 1, None) };
        }
        self.retarget();
        self.update_bars();
        self.update_logo();
        self.update_tick();
        self.sync_open();
        self.update_capture();
    }

    fn redraw_expanded(&mut self) {
        self.note_lyric();
        self.prune_hold();
        let front = self.snapshot.activities.first();
        let accent = front.and_then(|a| a.accent).map(render::rgb).unwrap_or(color::accent::DEFAULT);
        match self.renderer.draw_expanded(&self.expanded_surface, front.and_then(|a| a.expanded.as_ref()), accent, &self.interaction()) {
            Ok(card) => {
                self.hits = card.hits;
                self.block_max = card.block_max;
            }
            Err(e) => eprintln!("island: redraw failed: {e}"),
        }
    }

}

impl Drop for Pill {
    fn drop(&mut self) {
        clipboard_keys::release();
    }
}
