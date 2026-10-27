//! The pill: composition tree, spring morphs, hover/click handling, and how hub snapshots
//! map onto presentations (idle core, compact, peek, expanded, plus the minimal bubble).
//!
//! Animated values live in one property set: the pill's width `w` and height `h`, the
//! minimal bubble's presence `m`, and the current expanded height `eh`. Everything else
//! (clip, centring, cross-fades) is an expression over them, so morphs run inside DWM.
//! Layer offsets are rounded to whole pixels: a fractional offset makes the compositor
//! resample the content bitmaps, which visibly softens thin strokes and text.

use std::collections::HashSet;
use std::sync::mpsc::Receiver;
use std::time::Instant;

use reui_design::tokens::{color, component, island, motion};
use reui_design::{Color, Spring};
use reui_protocol::{
    Activity, ActivityId, ClientMsg, ExpandedView, ImageId, Item, ItemKey, Priority, Row, Snapshot, ACTION_COLLAPSED, ACTION_EXPANDED, ACTION_SEARCH, ACTION_SWIPE_NEXT, ACTION_SWIPE_PREV,
    ACTION_UNDO,
};
use tokio::sync::mpsc::UnboundedSender;
use windows::core::{h, Interface, Result, HSTRING};
use windows::Foundation::{IReference, PropertyValue, TimeSpan};
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::WinRT::Composition::ICompositorDesktopInterop;
use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows_numerics::{Vector2, Vector3};

use crate::audio::Visualizer;
use crate::ipc::Event;
use crate::music::{self, SeekHold, Swipe, Throttle};
use crate::panel::{self, Key, Panel};
use crate::settings::{Settings, Visibility};
use crate::clipwin::Inbound;
use crate::{images, keys, order};
use crate::render::{self, Hit, Interaction, Renderer, Thumb, BARS, BAR_LEVELS_LIVE, BAR_LEVEL_IDLE, COMPACT_SURFACE, EXPANDED_SURFACE, MINIMAL_SURFACE};

pub const TIMER_COLLAPSE: usize = 1;
pub const TIMER_HOVER: usize = 2;
pub const TIMER_REVEAL: usize = 3;
pub const TIMER_SPIN: usize = 4;
pub const TIMER_TICK: usize = 5;
pub const TIMER_BARS: usize = 6;
pub const TIMER_REORDER: usize = 7;
pub const TIMER_SCROLL: usize = 8;
pub const TIMER_SEARCH: usize = 9;
pub const TIMER_AWAIT: usize = 10;
pub const TIMER_HOLD: usize = 11;
/// How often an open card with a playing timeline redraws its position.
const TICK_MS: u32 = 1000;
/// Typing is sent to the module once it pauses this long; the field shows it at once.
const SEARCH_DEBOUNCE_MS: u32 = 60;
/// Enter right after typing waits for the results of what was typed, at most this long.
const AWAIT_RESULTS_MS: u32 = 400;

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
    visibility: Visibility,
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
    /// A slider value set here, shown until the next snapshot.
    slider_hold: Option<f32>,
    /// The bar under the pointer shows its thumb.
    thumb: Option<Thumb>,
    swipe: Swipe,
    /// Wheel travel over the source switcher, toward a whole notch.
    pager_travel: i32,
    to_hub: UnboundedSender<ClientMsg>,
    events: Receiver<Event>,
}

fn color_brush(c: &Compositor, color: Color) -> Result<CompositionColorBrush> {
    let [r, g, b, a] = color.to_rgba8();
    c.CreateColorBrushWithColor(windows::UI::Color { A: a, R: r, G: g, B: b })
}

impl Pill {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        hwnd: HWND,
        scale: f32,
        window: RECT,
        monitor: HMONITOR,
        reduced_motion: bool,
        visibility: Visibility,
        to_hub: UnboundedSender<ClientMsg>,
        events: Receiver<Event>,
    ) -> Result<Self> {
        let s = scale;
        let compositor = Compositor::new()?;
        let target = unsafe { compositor.cast::<ICompositorDesktopInterop>()?.CreateDesktopWindowTarget(hwnd, true)? };
        let root = compositor.CreateContainerVisual()?;
        target.SetRoot(&root)?;
        // Showing and hiding scale around the top centre, where the island meets the screen edge.
        root.SetCenterPoint(Vector3::new((window.right - window.left) as f32 / 2.0, 0.0, 0.0))?;

        let core = (island::idle::WIDTH, island::HEIGHT);
        let peek_h = core.1 + island::compact::PEEK_GROW;
        let props = compositor.CreatePropertySet()?;
        props.InsertScalar(h!("w"), core.0 * s)?;
        props.InsertScalar(h!("h"), core.1 * s)?;
        props.InsertScalar(h!("m"), 0.0)?;
        props.InsertScalar(h!("eh"), island::expanded::MIN_HEIGHT * s)?;
        // Expanded corner radius: the pill's, or the wide panel's calmer one.
        props.InsertScalar(h!("r"), island::expanded::RADIUS * s)?;
        let win_w = (window.right - window.left) as f32;
        let expr = |text: &str, scalars: &[(&str, f32)]| -> Result<ExpressionAnimation> {
            let e = compositor.CreateExpressionAnimationWithExpression(&HSTRING::from(text))?;
            e.SetReferenceParameter(h!("p"), &props)?;
            for (k, v) in scalars {
                e.SetScalarParameter(&HSTRING::from(*k), *v)?;
            }
            Ok(e)
        };
        // Expansion progress: 0 at peek size or smaller, 1 at the current expanded height.
        let t = format!("Clamp((p.h - {hp}) / Max(p.eh - {hp}, 1), 0, 1)", hp = peek_h * s);
        let compact_fade = format!("Clamp(1 - {t} * {k}, 0, 1)", k = motion::crossfade::COMPACT_OUT);

        // The pill itself.
        let pill = compositor.CreateSpriteVisual()?;
        pill.SetBrush(&color_brush(&compositor, color::island::BACKGROUND)?)?;
        pill.StartAnimation(h!("Size"), &expr("Vector2(p.w, p.h)", &[])?)?;
        pill.StartAnimation(h!("Offset"), &expr("Vector3(Round((W - p.w) / 2), Round(T), 0)", &[("W", win_w), ("T", island::TOP * s)])?)?;
        let shape = compositor.CreateRoundedRectangleGeometry()?;
        shape.StartAnimation(h!("Size"), &expr("Vector2(p.w, p.h)", &[])?)?;
        shape.StartAnimation(h!("CornerRadius"), &expr("Vector2(Min(p.h / 2, p.r), Min(p.h / 2, p.r))", &[])?)?;
        pill.SetClip(&compositor.CreateGeometricClipWithGeometry(&shape)?)?;
        root.Children()?.InsertAtTop(&pill)?;

        let renderer = Renderer::new(&compositor, s)?;
        let compact_surface = renderer.surface(COMPACT_SURFACE)?;
        let expanded_surface = renderer.surface(EXPANDED_SURFACE)?;
        let minimal_surface = renderer.surface(MINIMAL_SURFACE)?;

        // Compact content: centred, nudged down while peeking, gone early in an expansion.
        let compact = surface_sprite(&compositor, &compact_surface, COMPACT_SURFACE, s)?;
        compact.StartAnimation(
            h!("Offset"),
            &expr(
                "Vector3(Round((p.w - Ws) / 2), Round(Min((p.h - Hc) / 2, D)), 0)",
                &[("Ws", COMPACT_SURFACE.0 * s), ("Hc", core.1 * s), ("D", island::compact::PEEK_GROW / 2.0 * s)],
            )?,
        )?;
        compact.StartAnimation(h!("Opacity"), &expr(&compact_fade, &[])?)?;
        pill.Children()?.InsertAtTop(&compact)?;

        // Level bars are visuals, so they move without redrawing anything.
        let bar_brush = compositor.CreateColorBrush()?;
        let bars = bar_visuals(&compositor, &compact.Children()?, &bar_brush, s)?;

        // Animated logos are their own visual, so spinning never redraws anything.
        let logo_surface = renderer.surface((component::art::COMPACT, component::art::COMPACT))?;
        let logo = surface_sprite(&compositor, &logo_surface, (component::art::COMPACT, component::art::COMPACT), s)?;
        let half = component::art::COMPACT / 2.0 * s;
        logo.SetCenterPoint(Vector3::new(half, half, 0.0))?;
        logo.SetIsVisible(false)?;
        compact.Children()?.InsertAtTop(&logo)?;

        // Expanded content: fades and scales in during the later part of the morph.
        let start = motion::crossfade::EXPANDED_IN_START;
        let from = motion::crossfade::EXPANDED_SCALE_FROM;
        let expanded = surface_sprite(&compositor, &expanded_surface, EXPANDED_SURFACE, s)?;
        expanded.SetCenterPoint(Vector3::new(EXPANDED_SURFACE.0 / 2.0 * s, 0.0, 0.0))?;
        expanded.StartAnimation(h!("Offset"), &expr("Vector3(Round((p.w - We) / 2), 0, 0)", &[("We", EXPANDED_SURFACE.0 * s)])?)?;
        expanded.StartAnimation(h!("Opacity"), &expr(&format!("Clamp(({t} - {start}) / (1 - {start}), 0, 1)"), &[])?)?;
        let grow = format!("{from} + (1 - {from}) * {t}");
        expanded.StartAnimation(h!("Scale"), &expr(&format!("Vector3({grow}, {grow}, 1)"), &[])?)?;
        pill.Children()?.InsertAtTop(&expanded)?;
        let card_bars = bar_visuals(&compositor, &expanded.Children()?, &bar_brush, s)?;

        // Minimal bubble: the second activity, detached to the right of the pill.
        let size = island::minimal::SIZE * s;
        let bubble = compositor.CreateSpriteVisual()?;
        bubble.SetSize(Vector2::new(size, size))?;
        bubble.SetBrush(&color_brush(&compositor, color::island::BACKGROUND)?)?;
        let circle = compositor.CreateRoundedRectangleGeometry()?;
        circle.SetSize(Vector2::new(size, size))?;
        circle.SetCornerRadius(Vector2::new(size / 2.0, size / 2.0))?;
        bubble.SetClip(&compositor.CreateGeometricClipWithGeometry(&circle)?)?;
        bubble.SetCenterPoint(Vector3::new(size / 2.0, size / 2.0, 0.0))?;
        bubble.StartAnimation(
            h!("Offset"),
            &expr("Vector3(Round((W + p.w) / 2 + G), Round(T), 0)", &[("W", win_w), ("G", island::minimal::GAP * s), ("T", island::TOP * s)])?,
        )?;
        bubble.StartAnimation(h!("Opacity"), &expr(&format!("p.m * {compact_fade}"), &[])?)?;
        bubble.StartAnimation(h!("Scale"), &expr("Vector3(0.6 + 0.4 * p.m, 0.6 + 0.4 * p.m, 1)", &[])?)?;
        bubble.Children()?.InsertAtTop(&surface_sprite(&compositor, &minimal_surface, MINIMAL_SURFACE, s)?)?;
        let minimal_brush = compositor.CreateColorBrush()?;
        let minimal_bars = bar_visuals(&compositor, &bubble.Children()?, &minimal_brush, s)?;
        root.Children()?.InsertAtTop(&bubble)?;

        Ok(Self {
            hwnd,
            scale,
            window,
            monitor,
            reduced_motion,
            compositor,
            _target: target,
            root,
            props,
            renderer,
            compact_surface,
            expanded_surface,
            minimal_surface,
            bars,
            card_bars,
            minimal_bars,
            bar_brush,
            minimal_brush,
            bars_clock: std::time::Instant::now(),
            front_bars: None,
            minimal_bars_live: None,
            bars_moving: false,
            front_audio: None,
            minimal_audio: None,
            visualizer: Visualizer::default(),
            logo_surface,
            logo,
            logo_state: None,
            logo_angle: 0.0,
            compact_width: core.0,
            expanded_height: island::expanded::MIN_HEIGHT,
            expanded_width: island::expanded::WIDTH,
            hits: Vec::new(),
            raw: Snapshot::default(),
            snapshot: Snapshot::default(),
            pinned: None,
            summoned: false,
            on_bubble: false,
            open_id: None,
            state: State::Idle,
            target: core,
            minimal: false,
            inside: false,
            hovered: false,
            hidden: false,
            visibility,
            revealed: false,
            shown: true,
            clickthrough: true,
            panel: Panel::default(),
            scroll_clock: None,
            search_dirty: false,
            awaiting: None,
            menu_open: false,
            ignore_foreground: None,
            drag: None,
            drag_drawn: None,
            seek_hold: None,
            slider_hold: None,
            thumb: None,
            swipe: Swipe::default(),
            pager_travel: 0,
            to_hub,
            events,
        })
    }

    fn front(&self) -> Option<&Activity> {
        self.snapshot.activities.first()
    }

    /// Apply everything the hub link queued; only the newest snapshot matters. Messages for
    /// module windows are returned, in order, for the caller to deliver outside this borrow.
    pub fn drain_hub(&mut self) -> Vec<Inbound> {
        let mut latest = None;
        let mut focus = None;
        let mut inbound = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Connected => eprintln!("island: connected to hub"),
                Event::Focus(id) => focus = Some(id),
                // Kept encoded; the pill decodes what a snapshot shows (below), the
                // clipboard window serves the bytes as they are.
                Event::Image(id, bytes) => images::put(id, bytes),
                Event::Window { surface, message, images: ids } => {
                    let message = Inbound::message(surface, &message);
                    images::window_message(message.kind(), &ids);
                    inbound.push(message);
                }
                // Evict in stream order, exactly where the hub does, so images that arrived
                // for a later snapshot survive.
                Event::Snapshot(snapshot) => {
                    let shown = panel::referenced(&snapshot);
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
                    inbound.push(Inbound::Disconnected);
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
        inbound
    }

    /// The hub asked to open this activity (the user pressed its hotkey): bring it forward
    /// over anything (showing the island even in a hidden mode) and expand it until
    /// dismissed. Asking again while it's open closes it.
    fn focus(&mut self, id: ActivityId) {
        if self.hidden || !self.raw.activities.iter().any(|a| a.id == id) {
            return;
        }
        if self.summoned && self.state == State::Expanded && self.pinned.as_ref() == Some(&id) {
            self.set_state(State::Compact);
            return;
        }
        self.summoned = true;
        self.bring_forward(id);
        if self.visibility != Visibility::Always {
            self.revealed = true;
            self.refresh_visibility();
            unsafe { SetTimer(Some(self.hwnd), TIMER_REVEAL, motion::delay::REVEAL_HOLD, None) };
        }
        self.set_state(State::Expanded);
        if !self.hovered {
            unsafe { SetTimer(Some(self.hwnd), TIMER_COLLAPSE, motion::delay::PANEL_IDLE, None) };
        }
    }

    /// Closes a summoned card when the user clearly moved on.
    fn dismiss_summoned(&mut self) {
        if self.summoned && self.state == State::Expanded {
            self.set_state(State::Compact);
        }
    }

    /// A button went down anywhere: outside a summoned card, that dismisses it.
    pub fn on_mouse_down(&mut self, pt: POINT) {
        if self.menu_open {
            return;
        }
        let (x, y) = (pt.x as f32, pt.y as f32);
        let (l, t, r, b) = self.shape_rect();
        if !(x >= l && x < r && y >= t && y < b) {
            self.dismiss_summoned();
        }
    }

    /// Tells modules when their card opens and closes (`expanded` / `collapsed`), so e.g.
    /// the clipboard keeps its history card while it's being used.
    fn sync_open(&mut self) {
        let now_open = if self.state == State::Expanded { self.front().map(|a| a.id.clone()) } else { None };
        if now_open == self.open_id {
            return;
        }
        if let Some(old) = self.open_id.take() {
            _ = self.to_hub.send(ClientMsg::Action { id: old, action: ACTION_COLLAPSED.into() });
        }
        if let Some(new) = &now_open {
            _ = self.to_hub.send(ClientMsg::Action { id: new.clone(), action: ACTION_EXPANDED.into() });
        }
        self.open_id = now_open;
    }

    fn apply(&mut self, snapshot: Snapshot) -> Result<()> {
        self.raw = snapshot;
        if self.drag.is_none() {
            self.slider_hold = None;
        }
        let ordered = self.ordered();
        self.render(ordered)
    }

    /// What to show where (see `order::arrange`).
    fn ordered(&mut self) -> Snapshot {
        let open = if self.state == State::Expanded { self.open_id.clone() } else { None };
        let (activities, pin_alive) = order::arrange(self.raw.activities.clone(), self.pinned.as_ref(), self.summoned, open.as_ref());
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

    /// Mouse wheel at screen point `pt`. In an open card: over the source switcher it
    /// switches source, over a slider it moves it, and a row of cards scrolls (also
    /// sideways, for trackpads). A sideways swipe on the compact pill skips track. Otherwise
    /// it cycles through activities in the hub's order.
    pub fn on_wheel(&mut self, delta: i16, horizontal: bool, pt: POINT) {
        if self.state == State::Expanded && !horizontal {
            let (cx, cy) = self.card_point((pt.x - self.window.left) as f32, (pt.y - self.window.top) as f32);
            let hit = render::hit_at(&self.hits, cx, cy).map(|h| (h.pager, h.slider.is_some(), h.action.clone()));
            match hit {
                Some((true, ..)) => return self.wheel_pager(delta as i32),
                Some((_, true, action)) => return self.wheel_slider(delta as i32, action),
                _ => {}
            }
        }
        if horizontal && matches!(self.state, State::Compact | State::Peek) && !self.on_bubble {
            match self.swipe.feed(delta as i32, Instant::now()) {
                Some(true) => self.send_action(ACTION_SWIPE_NEXT.into()),
                Some(false) => self.send_action(ACTION_SWIPE_PREV.into()),
                None => {}
            }
            return;
        }
        if self.state == State::Expanded && self.panel.row().is_some() {
            // Tilting right (positive) moves on, like turning the wheel down.
            self.panel.wheel(if horizontal { -(delta as i32) } else { delta as i32 });
            self.update_scroll();
            return;
        }
        if horizontal {
            return;
        }
        let ids: Vec<ActivityId> = self.raw.activities.iter().map(|a| a.id.clone()).collect();
        let Some(current) = self.front().map(|a| a.id.clone()) else { return };
        if ids.len() < 2 {
            return;
        }
        let at = ids.iter().position(|id| *id == current).unwrap_or(0);
        let next = if delta < 0 { (at + 1) % ids.len() } else { (at + ids.len() - 1) % ids.len() };
        self.bring_forward(ids[next].clone());
    }

    /// Wheel over "‹ 1/2 ›": a notch down is the next source.
    fn wheel_pager(&mut self, delta: i32) {
        let Some(pager) = self.front_view().and_then(|v| v.pager.clone()) else { return };
        self.pager_travel += delta;
        while self.pager_travel.abs() >= music::WHEEL_NOTCH {
            let down = self.pager_travel < 0;
            self.pager_travel -= music::WHEEL_NOTCH * self.pager_travel.signum();
            self.send_action(if down { pager.next.clone() } else { pager.prev.clone() });
        }
    }

    /// Wheel over a slider: 5 % a notch, shown at once.
    fn wheel_slider(&mut self, delta: i32, action: String) {
        let Some(current) = self.slider_hold.or_else(|| self.slider_value()) else { return };
        let value = music::wheel_value(current, delta);
        self.slider_hold = Some(value);
        self.send_action(music::slider_action(&action, value));
        self.redraw_expanded();
    }

    /// The module's slider value in the open card.
    fn slider_value(&self) -> Option<f32> {
        self.front_view()?.rows.iter().find_map(|r| if let Row::Slider { value, .. } = r { Some(*value) } else { None })
    }

    /// The open card's timeline as (position now, length, playing, rate), a held seek
    /// counting as the position.
    fn timeline_now(&self) -> Option<(u64, u64, bool, f32)> {
        let now = reui_protocol::unix_ms();
        self.front_view()?.rows.iter().find_map(|r| match r {
            Row::Timeline { position_ms, length_ms, playing, at_unix_ms, rate, .. } if *length_ms > 0 => {
                let reported = Row::timeline_position(*position_ms, *length_ms, *playing, *at_unix_ms, *rate, now);
                let shown = self.seek_hold.map_or(reported, |h| h.position(*length_ms, *playing, *rate, now));
                Some((shown, *length_ms, *playing, *rate))
            }
            _ => None,
        })
    }

    /// Lets go of a held seek once the player agrees with it, or it has held long enough.
    fn prune_hold(&mut self) {
        let Some(hold) = self.seek_hold else { return };
        let now = reui_protocol::unix_ms();
        let confirmed = self.front_view().and_then(|v| {
            v.rows.iter().find_map(|r| match r {
                Row::Timeline { position_ms, length_ms, playing, at_unix_ms, rate, .. } if *length_ms > 0 => {
                    let reported = Row::timeline_position(*position_ms, *length_ms, *playing, *at_unix_ms, *rate, now);
                    Some(!hold.holds(reported, *length_ms, *playing, *rate, now))
                }
                _ => None,
            })
        });
        if confirmed.unwrap_or(true) {
            self.seek_hold = None;
        }
    }

    /// A press in the open card at client point (`x`, `y`): on a seek bar or a slider it
    /// starts a drag (the caller captures the mouse). A plain click is a drag that ends
    /// where it began.
    pub fn on_press(&mut self, x: f32, y: f32) -> bool {
        if self.state != State::Expanded {
            return false;
        }
        let (cx, cy) = self.card_point(x, y);
        let Some(hit) = render::hit_at(&self.hits, cx, cy) else { return false };
        self.drag = match (hit.seek, hit.slider) {
            (Some((left, right, length)), _) => Some(Drag::Seek { left, right, length, ms: music::scrub_ms(cx, left, right, length) }),
            (_, Some((left, right))) => Some(Drag::Slider { left, right, action: hit.action.clone(), value: music::fraction_at(cx, left, right), throttle: Throttle::default() }),
            _ => None,
        };
        if self.drag.is_none() {
            return false;
        }
        self.send_drag();
        self.redraw_expanded();
        true
    }

    /// The pointer moved (screen point) during a drag.
    fn on_drag(&mut self, pt: POINT) {
        let (cx, _) = self.card_point((pt.x - self.window.left) as f32, (pt.y - self.window.top) as f32);
        match &mut self.drag {
            Some(Drag::Seek { left, right, length, ms }) => *ms = music::scrub_ms(cx, *left, *right, *length),
            Some(Drag::Slider { left, right, value, .. }) => *value = music::fraction_at(cx, *left, *right),
            None => return,
        }
        self.send_drag();
        let frame = std::time::Duration::from_secs_f32(1.0 / motion::scroll::FPS);
        if self.drag_drawn.is_none_or(|t| t.elapsed() >= frame) {
            self.drag_drawn = Some(Instant::now());
            self.redraw_expanded();
        }
    }

    /// A slider being dragged tells its module, at most 20 times a second.
    fn send_drag(&mut self) {
        let Some(Drag::Slider { action, value, throttle, .. }) = &mut self.drag else { return };
        if throttle.ready(Instant::now()) {
            let action = music::slider_action(action, *value);
            self.send_action(action);
        }
    }

    /// The button came up (client point): ends a drag, sending where it ended. Returns
    /// whether there was one (the caller then releases the mouse).
    pub fn on_release(&mut self, x: f32, y: f32) -> bool {
        if self.drag.is_none() {
            return false;
        }
        let (sx, sy) = (x as i32 + self.window.left, y as i32 + self.window.top);
        self.on_drag(POINT { x: sx, y: sy });
        self.finish_drag();
        true
    }

    /// The mouse capture went elsewhere mid-drag: keep what was set.
    pub fn on_capture_lost(&mut self) {
        self.finish_drag();
    }

    fn finish_drag(&mut self) {
        match self.drag.take() {
            Some(Drag::Seek { ms, .. }) => {
                self.send_action(format!("seek:{ms}"));
                self.seek_hold = Some(SeekHold { position_ms: ms, at_unix_ms: reui_protocol::unix_ms() });
                // Redraw when the hold runs out, in case nothing else does (paused).
                unsafe { SetTimer(Some(self.hwnd), TIMER_HOLD, music::SEEK_HOLD_MS as u32 + 20, None) };
            }
            Some(Drag::Slider { action, value, .. }) => {
                self.send_action(music::slider_action(&action, value));
                self.slider_hold = Some(value);
            }
            None => return,
        }
        self.redraw_expanded();
        self.update_tick();
    }

    /// The thumb of the bar under the pointer (screen point), redrawn only when it changes.
    fn update_thumb(&mut self, pt: POINT) {
        let want = if self.state == State::Expanded && self.shown {
            let (cx, cy) = self.card_point((pt.x - self.window.left) as f32, (pt.y - self.window.top) as f32);
            render::hit_at(&self.hits, cx, cy).and_then(|h| match (h.seek, h.slider) {
                (Some(_), _) => Some(Thumb::Seek),
                (_, Some(_)) => Some(Thumb::Slider),
                _ => None,
            })
        } else {
            None
        };
        if want != self.thumb {
            self.thumb = want;
            self.redraw_expanded();
        }
    }

    fn render(&mut self, snapshot: Snapshot) -> Result<()> {
        let previous = self.front().map(|a| (a.id.clone(), a.priority));
        self.snapshot = snapshot;
        if self.front().map(|a| &a.id) != previous.as_ref().map(|(id, _)| id) {
            // Holds belong to the activity that was in front.
            self.seek_hold = None;
            self.slider_hold = None;
        }
        self.prune_hold();
        self.front_audio = self.front().and_then(|a| a.audio);
        self.minimal_audio = self.snapshot.activities.get(1).and_then(|a| a.audio);
        let front = self.snapshot.activities.first();
        self.panel.sync(front.map(|a| &a.id), front.and_then(|a| a.expanded.as_ref()), keys::captured());
        let accent = |a: Option<&Activity>| a.and_then(|a| a.accent).map(render::rgb).unwrap_or(color::accent::DEFAULT);
        let front = self.snapshot.activities.first();
        let second = self.snapshot.activities.get(1);

        let front_accent = accent(front);
        let compact = self.renderer.draw_compact(&self.compact_surface, front.map(|a| &a.compact), front_accent)?;
        let card = self.renderer.draw_expanded(&self.expanded_surface, front.and_then(|a| a.expanded.as_ref()), front_accent, &self.interaction())?;
        let second_accent = accent(second);
        let minimal_bars = self.renderer.draw_minimal(&self.minimal_surface, second.map(|a| &a.minimal), second_accent)?;
        let has_second = second.is_some();
        let front = front.map(|a| (a.id.clone(), a.priority, a.expanded.is_some()));

        self.compact_width = compact.width;
        self.place_bars(compact.bars, card.bars, minimal_bars, front_accent, second_accent)?;
        self.place_logo(compact.logo, front_accent)?;
        self.expanded_height = card.height;
        self.expanded_width = card.width;
        self.hits = card.hits;
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
        let (scrub, slider, thumb) = match &self.drag {
            Some(Drag::Seek { ms, .. }) => (Some(*ms), self.slider_hold, Some(Thumb::Seek)),
            Some(Drag::Slider { value, .. }) => (None, Some(*value), Some(Thumb::Slider)),
            None => (None, self.slider_hold, self.thumb),
        };
        Interaction {
            query: self.panel.query.as_deref(),
            keyboard: keys::captured(),
            selected: self.panel.selected,
            scroll: self.panel.scroll,
            scrub,
            hold: self.seek_hold,
            slider,
            thumb,
        }
    }

    fn front_view(&self) -> Option<&ExpandedView> {
        self.front().and_then(|a| a.expanded.as_ref())
    }

    fn items(&self) -> &[Item] {
        self.front_view().and_then(|v| v.rows.iter().find_map(|r| if let Row::Items(items) = r { Some(items.as_slice()) } else { None })).unwrap_or_default()
    }

    /// The panel reads the keyboard only while a card opened by its hotkey, with a search
    /// field, is open and showing; the hook is gone the moment that stops being true.
    fn update_capture(&mut self) {
        let want = self.state == State::Expanded
            && self.summoned
            && self.shown
            && self.front_view().is_some_and(|v| v.rows.iter().any(|r| matches!(r, Row::Search { .. })));
        if want == keys::captured() {
            return;
        }
        if want {
            keys::capture(self.hwnd);
            if !keys::captured() {
                return;
            }
            self.panel.query = Some(self.panel.module_query().unwrap_or_default().to_string());
            self.panel.select(self.panel.selected);
        } else {
            keys::release();
            self.panel.query = None;
            self.search_dirty = false;
            self.awaiting = None;
            unsafe {
                _ = KillTimer(Some(self.hwnd), TIMER_SEARCH);
                _ = KillTimer(Some(self.hwnd), TIMER_AWAIT);
            }
        }
        // Caret and ring come and go; a closing card keeps its last frame.
        if self.state == State::Expanded {
            self.redraw_expanded();
            self.update_scroll();
        }
    }

    /// Eases the card row toward its target on a short frame timer that stops when it
    /// settles; with reduced motion it jumps.
    fn update_scroll(&mut self) {
        if !self.panel.moving() {
            return;
        }
        if self.reduced_motion || self.state != State::Expanded {
            self.panel.scroll = self.panel.target;
            self.redraw_expanded();
            return;
        }
        if self.scroll_clock.is_none() {
            self.scroll_clock = Some(Instant::now());
            let frame = 1000.0 / motion::scroll::FPS;
            let interval = ((frame / 15.625).floor().max(1.0) * 15.625) as u32;
            unsafe { SetTimer(Some(self.hwnd), TIMER_SCROLL, interval, None) };
        }
    }

    fn tick_scroll(&mut self) {
        let now = Instant::now();
        let dt = self.scroll_clock.replace(now).map_or(16.0, |t| (now - t).as_secs_f32() * 1000.0);
        let moving = self.panel.tick(dt);
        self.redraw_expanded();
        if !moving {
            self.scroll_clock = None;
            unsafe { _ = KillTimer(Some(self.hwnd), TIMER_SCROLL) };
        }
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

    /// Typed text is still on its way to the module, or its results haven't come back.
    fn search_pending(&self) -> bool {
        self.search_dirty || self.panel.query.as_deref().is_some_and(|q| Some(q) != self.panel.module_query())
    }

    fn flush_search(&mut self) {
        unsafe { _ = KillTimer(Some(self.hwnd), TIMER_SEARCH) };
        if std::mem::take(&mut self.search_dirty) {
            let query = self.panel.query.clone().unwrap_or_default();
            self.send_action(format!("{ACTION_SEARCH}{query}"));
        }
    }

    fn run_awaiting(&mut self) {
        unsafe { _ = KillTimer(Some(self.hwnd), TIMER_AWAIT) };
        if let Some(key) = self.awaiting.take() {
            self.on_key(key);
        }
    }

    /// Keys the hook queued (see `keys`).
    pub fn on_keys(&mut self) {
        let keys = keys::drain();
        if keys.is_empty() || !keys::captured() {
            return;
        }
        for key in keys {
            self.on_key(key);
        }
        // Typing counts as using the panel.
        if self.summoned && self.state == State::Expanded && !self.hovered {
            unsafe { SetTimer(Some(self.hwnd), TIMER_COLLAPSE, motion::delay::PANEL_IDLE, None) };
        }
        if self.state == State::Expanded {
            self.redraw_expanded();
            self.update_scroll();
        }
    }

    fn on_key(&mut self, key: Key) {
        let Some(query) = self.panel.query.as_mut() else { return };
        let edited = match key {
            Key::Type(c) => {
                query.push(c);
                true
            }
            Key::Backspace => query.pop().is_some(),
            Key::BackspaceWord => {
                let before = query.len();
                panel::delete_word(query);
                query.len() != before
            }
            Key::Escape if !query.is_empty() => {
                query.clear();
                self.search_dirty = true;
                self.flush_search();
                return;
            }
            Key::Escape => {
                self.set_state(State::Compact);
                return;
            }
            _ => false,
        };
        if edited {
            self.search_dirty = true;
            unsafe { SetTimer(Some(self.hwnd), TIMER_SEARCH, SEARCH_DEBOUNCE_MS, None) };
            return;
        }
        if matches!(key, Key::Activate | Key::Item(_) | Key::Nth(_)) && self.search_pending() {
            // Act on the results of what was typed, not the list still showing.
            self.flush_search();
            self.awaiting = Some(key);
            unsafe { SetTimer(Some(self.hwnd), TIMER_AWAIT, AWAIT_RESULTS_MS, None) };
            return;
        }
        let selected = self.panel.selected;
        let count = self.items().len();
        let action = match key {
            Key::Move(d) => {
                self.panel.select(selected.saturating_add_signed(d as isize));
                None
            }
            Key::First => {
                self.panel.select(0);
                None
            }
            Key::Last => {
                self.panel.select(usize::MAX);
                None
            }
            Key::Activate => self.items().get(selected).map(|i| i.action.clone()),
            Key::Item(k) => self.item_action(selected, k),
            Key::Nth(n) => {
                let i = self.panel.row().map_or(0, |r| r.first_shown(self.panel.scroll)) + n;
                (i < count).then(|| self.items()[i].action.clone())
            }
            Key::Tab(d) => self.front_view().and_then(|v| {
                let tabs = v.rows.iter().find_map(|r| if let Row::Tabs(t) = r { Some(t) } else { None })?;
                let at = tabs.iter().position(|t| t.selected).unwrap_or(0) as i32;
                let next = (at + d).rem_euclid(tabs.len().max(1) as i32) as usize;
                tabs.get(next).filter(|t| !t.selected).map(|t| t.action.clone())
            }),
            Key::Undo => Some(ACTION_UNDO.to_string()),
            Key::Type(_) | Key::Backspace | Key::BackspaceWord | Key::Escape => None,
        };
        if let Some(action) = action {
            let pastes = matches!(key, Key::Activate | Key::Nth(_) | Key::Item(ItemKey::ShiftEnter | ItemKey::CtrlEnter));
            self.act(action, pastes);
        }
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

    /// The context menu entries (label, action) of the card under client point (`x`, `y`),
    /// or `None` when it isn't over a card.
    /// Each entry says whether picking it closes the panel (see [`Self::act`]).
    pub fn card_menu(&self, x: f32, y: f32) -> Option<Vec<MenuEntry>> {
        if self.state != State::Expanded {
            return None;
        }
        let (cx, cy) = self.card_point(x, y);
        let i = render::hit_at(&self.hits, cx, cy)?.card?;
        let closes = |a: &reui_protocol::ItemAction| matches!(a.key, Some(ItemKey::ShiftEnter | ItemKey::CtrlEnter));
        Some(self.items().get(i)?.actions.iter().map(|a| (a.label.clone(), a.action.clone(), closes(a), false)).collect())
    }

    /// The choices of the menu button under client point (`x`, `y`) in the open card (a
    /// slider's output device), the current one checked; `None` when it isn't over one.
    pub fn click_menu(&self, x: f32, y: f32) -> Option<Vec<MenuEntry>> {
        if self.state != State::Expanded {
            return None;
        }
        let (cx, cy) = self.card_point(x, y);
        if !render::hit_at(&self.hits, cx, cy)?.menu {
            return None;
        }
        let menu = self.front_view()?.rows.iter().find_map(|r| if let Row::Slider { trailing: Some(menu), .. } = r { Some(menu) } else { None })?;
        Some(menu.options.iter().map(|o| (o.label.clone(), o.action.clone(), false, o.selected)).collect())
    }

    /// A card's context menu is up (keys go to it, clicks on it don't dismiss the panel)...
    pub fn menu_opened(&mut self) {
        self.menu_open = true;
        keys::pause(true);
    }

    /// ...and gone, the foreground handed back to `previous`.
    pub fn menu_closed(&mut self, previous: HWND) {
        self.menu_open = false;
        keys::pause(false);
        self.ignore_foreground = Some(previous);
    }

    /// Client pixels to expanded-surface DIPs.
    fn card_point(&self, x: f32, y: f32) -> (f32, f32) {
        let s = self.scale;
        let win_w = (self.window.right - self.window.left) as f32;
        ((x - (win_w - EXPANDED_SURFACE.0 * s) / 2.0) / s, (y - island::TOP * s) / s)
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

    /// An open card with a playing timeline is the one thing that redraws on a clock, once a
    /// second, and only while it's open; compact never ticks.
    /// Redraws the playing timeline just after each whole second of its position, so the
    /// times step 0:09 → 0:10 → 0:11 instead of skipping when the ticks drift.
    /// Lyrics move on at their own lines too: the tick comes early when a line starts
    /// before the next whole second.
    fn update_tick(&self) {
        let playing = self.timeline_now().filter(|(_, _, playing, _)| *playing);
        unsafe {
            match playing {
                Some((now, _, _, rate)) if self.state == State::Expanded && self.shown => {
                    let lyrics = self.front_view().and_then(|v| v.rows.iter().find_map(|r| if let Row::Lyrics(l) = r { Some(l.as_slice()) } else { None }));
                    let line = lyrics.and_then(|l| music::next_line_in(l, now)).map(|ms| (ms as f32 / rate.max(0.01)) as u32 + 5);
                    let delay = line.map_or(next_tick(now), |line| line.clamp(16, next_tick(now)));
                    SetTimer(Some(self.hwnd), TIMER_TICK, delay, None);
                }
                _ => _ = KillTimer(Some(self.hwnd), TIMER_TICK),
            }
        }
    }

    fn redraw_expanded(&mut self) {
        self.prune_hold();
        let front = self.snapshot.activities.first();
        let accent = front.and_then(|a| a.accent).map(render::rgb).unwrap_or(color::accent::DEFAULT);
        match self.renderer.draw_expanded(&self.expanded_surface, front.and_then(|a| a.expanded.as_ref()), accent, &self.interaction()) {
            Ok(card) => self.hits = card.hits,
            Err(e) => eprintln!("island: redraw failed: {e}"),
        }
    }

    /// Draws and positions the animated compact logo (or hides it).
    fn place_logo(&mut self, spot: Option<(ImageId, f32, f32)>, accent: Color) -> Result<()> {
        let s = self.scale;
        match spot {
            Some((image, left, top)) => {
                self.renderer.draw_logo(&self.logo_surface, image, component::art::COMPACT, accent)?;
                self.logo.SetOffset(Vector3::new((left * s).round(), (top * s).round(), 0.0))?;
                self.logo.SetIsVisible(true)?;
                let spinning = self.logo_state.is_some_and(|(_, spinning)| spinning);
                self.logo_state = Some((image, spinning));
            }
            None => {
                self.stop_logo();
                self.logo.SetIsVisible(false)?;
                self.logo_state = None;
            }
        }
        self.update_logo();
        Ok(())
    }

    /// Spins the logo while it's showing in compact; never when hidden or with reduced motion.
    fn update_logo(&mut self) {
        let Some((image, spinning)) = self.logo_state else { return };
        let want = matches!(self.state, State::Compact | State::Peek) && self.shown && !self.reduced_motion;
        if want == spinning {
            return;
        }
        if want {
            // Ticks, not a continuous spin: DWM only works during each short turn.
            unsafe { SetTimer(Some(self.hwnd), TIMER_SPIN, motion::logo::INTERVAL, None) };
            self.tick_logo();
        } else {
            self.stop_logo();
        }
        self.logo_state = Some((image, want));
    }

    /// One eased quarter turn from wherever the last tick stopped.
    fn tick_logo(&mut self) {
        let from = self.logo_angle;
        let to = from + 90.0;
        self.logo_angle = to % 360.0;
        let result = self.compositor.CreateScalarKeyFrameAnimation().and_then(|a| {
            let ease = self.compositor.CreateCubicBezierEasingFunction(Vector2::new(0.4, 0.0), Vector2::new(0.2, 1.0))?;
            a.InsertKeyFrame(0.0, from)?;
            a.InsertKeyFrameWithEasingFunction(1.0, to, &ease)?;
            a.SetDuration(TimeSpan { Duration: motion::logo::TURN as i64 * 10_000 })?;
            self.logo.StartAnimation(h!("RotationAngleInDegrees"), &a)
        });
        if let Err(e) = result {
            eprintln!("island: logo animation failed: {e}");
        }
    }

    fn stop_logo(&mut self) {
        unsafe { _ = KillTimer(Some(self.hwnd), TIMER_SPIN) };
        _ = self.logo.StopAnimation(h!("RotationAngleInDegrees"));
        _ = self.logo.SetRotationAngleInDegrees(0.0);
        self.logo_angle = 0.0;
    }

    /// Positions every set of bars where the renderer laid them out (or hides them).
    fn place_bars(
        &mut self,
        compact: Option<(f32, f32, bool)>,
        card: Option<(f32, f32, bool)>,
        minimal: Option<(f32, f32, bool)>,
        front_accent: Color,
        second_accent: Color,
    ) -> Result<()> {
        let s = self.scale;
        for (set, spot) in [(&self.bars, compact), (&self.card_bars, card), (&self.minimal_bars, minimal)] {
            for (i, bar) in set.iter().enumerate() {
                bar.SetIsVisible(spot.is_some())?;
                if let Some((left, mid, _)) = spot {
                    let x = left + i as f32 * (component::bars::WIDTH + component::bars::GAP);
                    bar.SetOffset(Vector3::new((x * s).round(), ((mid - component::bars::HEIGHT / 2.0) * s).round(), 0.0))?;
                }
            }
        }
        for (brush, accent) in [(&self.bar_brush, front_accent), (&self.minimal_brush, second_accent)] {
            let [r, g, b, a] = accent.to_rgba8();
            brush.SetColor(windows::UI::Color { A: a, R: r, G: g, B: b })?;
        }
        self.front_bars = compact.or(card).map(|(_, _, live)| live);
        self.minimal_bars_live = minimal.map(|(_, _, live)| live);
        self.rest_bars();
        self.update_bars();
        Ok(())
    }

    /// Which bar sets should be moving right now: the front's (pill or open card) and the
    /// minimal bubble's (hidden while the card is open).
    fn moving_sets(&self) -> (bool, bool) {
        if !self.shown || self.reduced_motion {
            return (false, false);
        }
        let front = self.front_bars == Some(true) && self.state != State::Idle;
        let minimal = self.minimal_bars_live == Some(true) && self.minimal && self.state != State::Expanded;
        (front, minimal)
    }

    /// Runs the frame clock while any set is moving; rests them otherwise.
    fn update_bars(&mut self) {
        let (front, minimal) = self.moving_sets();
        // Capture follows the moving bars: one process at a time, the front's first. It
        // stops (within its poll) when they rest, hide or switch to another process.
        let pid = if front { self.front_audio } else { None }.or(if minimal { self.minimal_audio } else { None });
        self.visualizer.follow(pid);
        let want = front || minimal;
        if want == self.bars_moving {
            return;
        }
        self.bars_moving = want;
        unsafe {
            if want {
                // A fixed frame rate, rounded to the system timer's 15.625 ms grain so it
                // doesn't land on the next tick and drop to ~21 fps.
                let frame = 1000.0 / motion::bars::FPS;
                let interval = ((frame / 15.625).floor().max(1.0) * 15.625) as u32;
                SetTimer(Some(self.hwnd), TIMER_BARS, interval, None);
            } else {
                _ = KillTimer(Some(self.hwnd), TIMER_BARS);
                self.rest_bars();
            }
        }
    }

    /// One frame. Bars whose process is being captured show its bands (bass on the left);
    /// the rest (or all, until the capture hears something) follow their own smooth
    /// two-sine waves. Only visible sets move.
    fn tick_bars(&self) {
        const WAVES: [(f32, f32, f32, f32); BARS] = [(1.7, 0.0, 3.1, 1.2), (2.3, 2.1, 1.3, 0.4), (1.1, 4.0, 2.9, 2.6), (2.9, 1.0, 1.9, 5.1)];
        let t = self.bars_clock.elapsed().as_secs_f32() * std::f32::consts::TAU;
        let (front, minimal) = self.moving_sets();
        let front_set = if self.state == State::Expanded { &self.card_bars } else { &self.bars };
        let real = self.visualizer.levels();
        let captured = self.visualizer.pid();
        let sets = [(front_set, front, 0.0, self.front_audio), (&self.minimal_bars, minimal, 0.9, self.minimal_audio)];
        for (set, moving, phase, pid) in sets {
            if !moving {
                continue;
            }
            let bands = real.filter(|_| pid.is_some() && pid == captured);
            for (i, (bar, (f1, p1, f2, p2))) in set.iter().zip(WAVES).enumerate() {
                let level = match bands {
                    Some(bands) => BAR_LEVEL_IDLE + (1.0 - BAR_LEVEL_IDLE) * bands[i.min(bands.len() - 1)],
                    None => (0.6 + 0.25 * (t * f1 + p1 + phase).sin() + 0.15 * (t * f2 + p2).sin()).clamp(BAR_LEVEL_IDLE, 1.0),
                };
                _ = bar.SetScale(Vector3::new(1.0, level, 1.0));
            }
        }
    }

    fn rest_bars(&self) {
        for (set, live) in [(&self.bars, self.front_bars), (&self.card_bars, self.front_bars), (&self.minimal_bars, self.minimal_bars_live)] {
            for (bar, level) in set.iter().zip(BAR_LEVELS_LIVE) {
                _ = bar.SetScale(Vector3::new(1.0, if live == Some(true) { level } else { BAR_LEVEL_IDLE }, 1.0));
            }
        }
    }

    fn state_size(&self, state: State) -> (f32, f32) {
        let core_h = island::HEIGHT;
        let grow = island::compact::PEEK_GROW;
        match state {
            State::Idle => (island::idle::WIDTH, core_h),
            State::Compact => (self.compact_width, core_h),
            State::Peek => (self.compact_width + grow, core_h + grow),
            State::Expanded => (self.expanded_width, self.expanded_height),
        }
    }

    /// Spring the pill to the current state's size, if it isn't already heading there.
    fn retarget(&mut self) {
        let size = self.state_size(self.state);
        if size == self.target {
            return;
        }
        self.target = size;
        let preset = match self.state {
            _ if self.reduced_motion => motion::spring::REDUCED,
            State::Idle => motion::spring::SETTLE,
            State::Compact => motion::spring::COLLAPSE,
            State::Peek => motion::spring::PEEK,
            State::Expanded => motion::spring::EXPAND,
        };
        let height = Spring {
            damping_ratio: (preset.damping_ratio + motion::height::DAMPING_OFFSET).min(1.0),
            period_ms: preset.period_ms * motion::height::PERIOD_FACTOR,
        };
        let result = self.spring(h!("w"), size.0 * self.scale, preset).and_then(|_| self.spring(h!("h"), size.1 * self.scale, height));
        if let Err(e) = result {
            eprintln!("island: spring failed: {e}");
        }
    }

    fn set_minimal(&mut self, on: bool) {
        if self.minimal == on {
            return;
        }
        self.minimal = on;
        self.update_bars();
        let preset = if self.reduced_motion { motion::spring::REDUCED } else { motion::spring::SETTLE };
        if let Err(e) = self.spring(h!("m"), if on { 1.0 } else { 0.0 }, preset) {
            eprintln!("island: spring failed: {e}");
        }
    }

    fn spring(&self, key: &HSTRING, to: f32, preset: Spring) -> Result<()> {
        let spring = self.compositor.CreateSpringScalarAnimation()?;
        spring.SetFinalValue(&PropertyValue::CreateSingle(to)?.cast::<IReference<f32>>()?)?;
        spring.SetDampingRatio(preset.damping_ratio)?;
        spring.SetPeriod(TimeSpan { Duration: (preset.period_ms * 10_000.0) as i64 })?;
        self.props.StartAnimation(key, &spring)
    }

    fn set_clickthrough(&mut self, on: bool) {
        if self.clickthrough == on {
            return;
        }
        self.clickthrough = on;
        unsafe {
            let ex = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE);
            let flag = WS_EX_TRANSPARENT.0 as isize;
            SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, if on { ex | flag } else { ex & !flag });
        }
    }

    /// Screen rectangle that counts as "on the pill". Compact uses its peek size so the
    /// edge doesn't flicker as the pill grows, and it reaches the top of the screen.
    fn hit_rect(&self) -> (f32, f32, f32, f32) {
        let state = if self.state == State::Compact { State::Peek } else { self.state };
        let (w, h) = self.state_size(state);
        let s = self.scale;
        let win_w = (self.window.right - self.window.left) as f32;
        let left = self.window.left as f32 + (win_w - w * s) / 2.0;
        let top = self.window.top as f32;
        (left, top, left + w * s, top + (island::TOP + h) * s)
    }

    /// Screen rectangle of the pill as drawn (without the band above it).
    fn shape_rect(&self) -> (f32, f32, f32, f32) {
        let (l, _, r, b) = self.hit_rect();
        (l, self.window.top as f32 + island::TOP * self.scale, r, b)
    }

    /// Screen rectangle of the minimal bubble, when it's there to be clicked.
    fn bubble_rect(&self) -> Option<(f32, f32, f32, f32)> {
        if !self.minimal || self.state == State::Expanded {
            return None;
        }
        let s = self.scale;
        let win_w = (self.window.right - self.window.left) as f32;
        let pill_w = self.state_size(self.state).0;
        let left = self.window.left as f32 + (win_w + pill_w * s) / 2.0 + island::minimal::GAP * s;
        let top = self.window.top as f32 + island::TOP * s;
        let size = island::minimal::SIZE * s;
        Some((left, top, left + size, top + size))
    }

    pub fn visibility(&self) -> Visibility {
        self.visibility
    }

    pub fn set_visibility(&mut self, visibility: Visibility) {
        self.visibility = visibility;
        Settings { visibility }.save();
        self.refresh_visibility();
    }

    fn wants_shown(&self) -> bool {
        if self.hidden {
            return false;
        }
        let activities = &self.snapshot.activities;
        self.revealed
            || match self.visibility {
                Visibility::Always => true,
                Visibility::WhenActive => !activities.is_empty(),
                Visibility::WhenNeeded => activities.iter().any(|a| a.priority >= Priority::Alert),
            }
    }

    /// Fades the island in or out to match the visibility mode, reveal and fullscreen.
    pub fn refresh_visibility(&mut self) {
        let show = self.wants_shown();
        if show == self.shown {
            return;
        }
        self.shown = show;
        if show {
            self.raise();
        }
        self.animate_presence(show);
        if !show {
            self.inside = false;
            self.hovered = false;
            self.set_clickthrough(true);
            unsafe { _ = KillTimer(Some(self.hwnd), TIMER_HOVER) };
            if self.state == State::Expanded {
                self.set_state(State::Compact);
            }
        }
        self.update_bars();
        self.update_logo();
        self.update_capture();
    }

    /// Emerges from the top edge (slide down, scale up with a little overshoot, fade in) or
    /// retracts into it (quicker, accelerating). Starts from wherever the island is now, so
    /// a quick show/hide/show reverses smoothly.
    fn animate_presence(&self, show: bool) {
        let from_scale = motion::show::FROM_SCALE;
        let (opacity, scale, rise) = if show { (1.0, 1.0, 0.0) } else { (0.0, from_scale, -motion::show::RISE * self.scale) };
        if self.reduced_motion {
            for property in [h!("Opacity"), h!("Scale"), h!("Offset")] {
                _ = self.root.StopAnimation(property);
            }
            _ = self.root.SetOpacity(opacity);
            _ = self.root.SetScale(Vector3::new(1.0, 1.0, 1.0));
            _ = self.root.SetOffset(Vector3::new(0.0, 0.0, 0.0));
            return;
        }
        let c = &self.compositor;
        let result = (|| -> Result<()> {
            let (duration, ease) = if show {
                // Decelerate with a slight overshoot: arrives, then settles.
                (motion::show::IN, c.CreateCubicBezierEasingFunction(Vector2::new(0.34, 1.4), Vector2::new(0.64, 1.0))?)
            } else {
                (motion::show::OUT, c.CreateCubicBezierEasingFunction(Vector2::new(0.4, 0.0), Vector2::new(1.0, 1.0))?)
            };
            let span = |ms: u32| TimeSpan { Duration: ms as i64 * 10_000 };

            let shape = c.CreateVector3KeyFrameAnimation()?;
            shape.InsertKeyFrameWithEasingFunction(1.0, Vector3::new(scale, scale, 1.0), &ease)?;
            shape.SetDuration(span(duration))?;
            self.root.StartAnimation(h!("Scale"), &shape)?;

            let slide = c.CreateVector3KeyFrameAnimation()?;
            slide.InsertKeyFrameWithEasingFunction(1.0, Vector3::new(0.0, rise, 0.0), &ease)?;
            slide.SetDuration(span(duration))?;
            self.root.StartAnimation(h!("Offset"), &slide)?;

            let fade = c.CreateScalarKeyFrameAnimation()?;
            fade.InsertKeyFrame(1.0, opacity)?;
            fade.SetDuration(span(motion::delay::FADE))?;
            // Fading out waits until the shape has mostly retracted.
            if !show {
                fade.SetDelayTime(span(duration.saturating_sub(motion::delay::FADE)))?;
            }
            self.root.StartAnimation(h!("Opacity"), &fade)
        })();
        if let Err(e) = result {
            eprintln!("island: presence animation failed: {e}");
            _ = self.root.SetOpacity(opacity);
        }
    }

    pub fn on_mouse_move(&mut self, pt: POINT) {
        if self.drag.is_some() {
            // The card stays as it is (no leave, no collapse) until the button comes up.
            return self.on_drag(pt);
        }
        // In a hidden mode, pushing the pointer against the top edge above the island reveals it.
        let at_edge = pt.y <= self.window.top + 1 && pt.x >= self.window.left && pt.x < self.window.right;
        if at_edge && !self.hidden && self.visibility != Visibility::Always && !self.shown {
            self.revealed = true;
            self.refresh_visibility();
            unsafe { SetTimer(Some(self.hwnd), TIMER_REVEAL, motion::delay::REVEAL_HOLD, None) };
        }
        let interactive = self.shown && self.state != State::Idle;
        let (x, y) = (pt.x as f32, pt.y as f32);
        let within = |(l, t, r, b): (f32, f32, f32, f32)| x >= l && x < r && y >= t && y < b;
        let on_bubble = interactive && self.bubble_rect().is_some_and(within);
        let inside = interactive && (within(self.hit_rect()) || on_bubble);
        self.on_bubble = on_bubble;
        // On the visible pill (or bubble) itself, clicks are the island's at once: a quick
        // click there is clearly aimed at it. The rest delay only gates the peek, and the
        // thin band above the pill, where browser tabs actually are.
        if interactive && (within(self.shape_rect()) || on_bubble) {
            self.set_clickthrough(false);
        }
        self.update_thumb(pt);
        if inside == self.inside {
            return;
        }
        self.inside = inside;
        if inside {
            // Only take over once the pointer rests here (see `hovered`).
            unsafe { SetTimer(Some(self.hwnd), TIMER_HOVER, motion::delay::HOVER_INTENT, None) };
            return;
        }
        unsafe { _ = KillTimer(Some(self.hwnd), TIMER_HOVER) };
        if self.revealed {
            // Hide again a while after the pointer leaves.
            unsafe { SetTimer(Some(self.hwnd), TIMER_REVEAL, motion::delay::REVEAL_HOLD, None) };
        }
        let was_hovered = std::mem::replace(&mut self.hovered, false);
        self.set_clickthrough(true);
        match self.state {
            State::Expanded => unsafe {
                let delay = if self.summoned { motion::delay::PANEL_IDLE } else { motion::delay::COLLAPSE };
                _ = SetTimer(Some(self.hwnd), TIMER_COLLAPSE, delay, None);
            },
            State::Peek if was_hovered => self.set_state(State::Compact),
            _ => {}
        }
    }

    fn on_hover_intent(&mut self) {
        if !self.inside || !self.shown || self.state == State::Idle {
            return;
        }
        self.hovered = true;
        unsafe { _ = KillTimer(Some(self.hwnd), TIMER_COLLAPSE) };
        self.set_clickthrough(false);
        // Resting on the bubble makes it clickable without growing the pill.
        if self.state == State::Compact && !self.on_bubble {
            self.set_state(State::Peek);
        }
    }

    /// `x`, `y` are client pixels. Shift+click on a card is its Shift+Enter action.
    pub fn on_click(&mut self, x: f32, y: f32, shift: bool) {
        // A click on the minimal bubble swaps that activity into the pill.
        if let Some((l, t, r, b)) = self.bubble_rect() {
            let (sx, sy) = (x + self.window.left as f32, y + self.window.top as f32);
            if sx >= l && sx < r && sy >= t && sy < b {
                if let Some(second) = self.snapshot.activities.get(1) {
                    self.bring_forward(second.id.clone());
                }
                return;
            }
        }
        let Some(front) = self.front() else { return };
        if self.state != State::Expanded {
            if front.expanded.is_some() {
                self.set_state(State::Expanded);
            } else {
                // No card to open here: the module may have its own window (the clipboard).
                _ = self.to_hub.send(ClientMsg::Action { id: front.id.clone(), action: "open".into() });
            }
            return;
        }
        let (cx, cy) = self.card_point(x, y);
        if let Some(hit) = render::hit_at(&self.hits, cx, cy) {
            let card = hit.card.is_some();
            let action = match hit.card {
                Some(i) if shift => self.item_action(i, ItemKey::ShiftEnter),
                _ => None,
            };
            let action = action.unwrap_or_else(|| hit.action_at(cx));
            // The switcher's middle ("1/2") is only a wheel target.
            if !action.is_empty() {
                self.act(action, card);
            }
        }
    }

    pub fn on_timer(&mut self, id: usize) {
        // The bar and scroll frame timers are periodic; every other timer here is one-shot.
        if id == TIMER_BARS {
            self.tick_bars();
            return;
        }
        if id == TIMER_SCROLL {
            self.tick_scroll();
            return;
        }
        unsafe { _ = KillTimer(Some(self.hwnd), id) };
        match id {
            TIMER_HOVER => self.on_hover_intent(),
            TIMER_HOLD if self.state == State::Expanded => {
                self.redraw_expanded();
                self.update_tick();
            }
            TIMER_TICK => {
                self.redraw_expanded();
                self.update_tick();
            }
            TIMER_SPIN if self.logo_state.is_some_and(|(_, ticking)| ticking) => {
                unsafe { SetTimer(Some(self.hwnd), TIMER_SPIN, motion::logo::INTERVAL, None) };
                self.tick_logo();
            }
            // Revealed stays revealed while the pointer is on it or a summoned card is open.
            TIMER_REVEAL if self.inside || (self.summoned && self.state == State::Expanded) => unsafe {
                SetTimer(Some(self.hwnd), TIMER_REVEAL, motion::delay::REVEAL_HOLD, None);
            },
            TIMER_REVEAL => {
                self.revealed = false;
                self.refresh_visibility();
            }
            TIMER_COLLAPSE if !self.hovered && self.state == State::Expanded => self.set_state(State::Compact),
            TIMER_SEARCH => self.flush_search(),
            TIMER_AWAIT => {
                // The results never came back to match; act on what's showing.
                self.run_awaiting();
                if self.state == State::Expanded {
                    self.redraw_expanded();
                }
            }
            TIMER_REORDER => {
                unsafe { _ = KillTimer(Some(self.hwnd), TIMER_REORDER) };
                let ordered = self.ordered();
                if let Err(e) = self.render(ordered) {
                    eprintln!("island: render failed: {e}");
                }
            }
            _ => {}
        }
    }

    pub fn on_foreground(&mut self, fg: HWND) {
        if self.ignore_foreground.take() == Some(fg) {
            return;
        }
        // Switching apps means the summoned card is done with.
        self.dismiss_summoned();
        let hide = is_fullscreen(fg, self.monitor);
        if !hide {
            self.raise();
        }
        if hide == self.hidden {
            return;
        }
        self.hidden = hide;
        self.refresh_visibility();
    }

    /// Puts the island back at the top of the always-on-top band. Being topmost once isn't
    /// enough: the taskbar, Start, pop-ups and apps that make themselves topmost later all
    /// land above it, so this runs whenever the foreground changes and when it shows.
    fn raise(&self) {
        unsafe {
            _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER);
        }
    }
}

impl Drop for Pill {
    fn drop(&mut self) {
        keys::release();
    }
}

/// Four rounded level bars, hidden until placed, scaling around their vertical centre.
fn bar_visuals(c: &Compositor, parent: &VisualCollection, brush: &CompositionColorBrush, s: f32) -> Result<Vec<SpriteVisual>> {
    let (bw, bh) = (component::bars::WIDTH * s, component::bars::HEIGHT * s);
    let mut bars = Vec::with_capacity(BARS);
    for _ in 0..BARS {
        let bar = c.CreateSpriteVisual()?;
        bar.SetBrush(brush)?;
        bar.SetSize(Vector2::new(bw, bh))?;
        bar.SetCenterPoint(Vector3::new(bw / 2.0, bh / 2.0, 0.0))?;
        let rounded = c.CreateRoundedRectangleGeometry()?;
        rounded.SetSize(Vector2::new(bw, bh))?;
        rounded.SetCornerRadius(Vector2::new(bw / 2.0, bw / 2.0))?;
        bar.SetClip(&c.CreateGeometricClipWithGeometry(&rounded)?)?;
        bar.SetIsVisible(false)?;
        parent.InsertAtTop(&bar)?;
        bars.push(bar);
    }
    Ok(bars)
}

fn surface_sprite(c: &Compositor, surface: &CompositionDrawingSurface, dips: (f32, f32), s: f32) -> Result<SpriteVisual> {
    let sprite = c.CreateSpriteVisual()?;
    sprite.SetSize(Vector2::new((dips.0 * s).ceil(), (dips.1 * s).ceil()))?;
    sprite.SetBrush(&c.CreateSurfaceBrushWithSurface(surface)?)?;
    Ok(sprite)
}

/// A foreground window covering its whole monitor (and not the desktop) counts as fullscreen.
fn is_fullscreen(fg: HWND, monitor: HMONITOR) -> bool {
    unsafe {
        if let Ok(state) = SHQueryUserNotificationState() {
            if state == QUNS_RUNNING_D3D_FULL_SCREEN || state == QUNS_PRESENTATION_MODE {
                return true;
            }
        }
        if fg.is_invalid() || MonitorFromWindow(fg, MONITOR_DEFAULTTONULL) != monitor {
            return false;
        }
        let mut class = [0u16; 64];
        let len = GetClassNameW(fg, &mut class) as usize;
        let class = String::from_utf16_lossy(&class[..len]);
        if matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd") {
            return false;
        }
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let mut rect = RECT::default();
        GetMonitorInfoW(monitor, &mut info).as_bool()
            && GetWindowRect(fg, &mut rect).is_ok()
            && rect.left <= info.rcMonitor.left
            && rect.top <= info.rcMonitor.top
            && rect.right >= info.rcMonitor.right
            && rect.bottom >= info.rcMonitor.bottom
    }
}

/// Milliseconds until just after the next whole second of a position.
fn next_tick(position_ms: u64) -> u32 {
    (TICK_MS - (position_ms % u64::from(TICK_MS)) as u32 + 15).max(16)
}

#[cfg(test)]
mod tick_tests {
    #[test]
    fn ticks_land_just_after_whole_seconds() {
        assert_eq!(super::next_tick(9_400), 615);
        assert_eq!(super::next_tick(10_000), 1_015);
        assert_eq!(super::next_tick(10_999), 16);
    }
}
