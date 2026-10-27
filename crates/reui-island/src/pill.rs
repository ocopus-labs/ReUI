//! The pill: composition tree, spring morphs, hover/click handling, and how hub snapshots
//! map onto presentations (idle core, compact, peek, expanded, plus the minimal bubble).
//!
//! Animated values live in one property set: the pill's width `w` and height `h`, the
//! minimal bubble's presence `m`, and the current expanded height `eh`. Everything else
//! (clip, centring, cross-fades) is an expression over them, so morphs run inside DWM.
//! Layer offsets are rounded to whole pixels: a fractional offset makes the compositor
//! resample the content bitmaps, which visibly softens thin strokes and text.

use std::sync::mpsc::Receiver;

use reui_design::tokens::{color, component, island, motion};
use reui_design::{Color, Spring};
use reui_protocol::{Activity, ActivityId, ClientMsg, ImageId, Priority, Row, Snapshot};
use tokio::sync::mpsc::UnboundedSender;
use windows::core::{h, Interface, Result, HSTRING};
use windows::Foundation::{IReference, PropertyValue, TimeSpan};
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::WinRT::Composition::ICompositorDesktopInterop;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows_numerics::{Vector2, Vector3};

use crate::ipc::Event;
use crate::order;
use crate::settings::{Settings, Visibility};
use crate::render::{self, Hit, Renderer, BARS, BAR_LEVELS_LIVE, BAR_LEVEL_IDLE, COMPACT_SURFACE, EXPANDED_SURFACE, MINIMAL_SURFACE};

pub const TIMER_COLLAPSE: usize = 1;
pub const TIMER_HOVER: usize = 2;
pub const TIMER_REVEAL: usize = 3;
pub const TIMER_SPIN: usize = 4;
pub const TIMER_TICK: usize = 5;
pub const TIMER_BARS: usize = 6;
/// How often an open card with a playing timeline redraws its position.
const TICK_MS: u32 = 1000;

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
    logo_surface: CompositionDrawingSurface,
    logo: SpriteVisual,
    /// Animated compact logo: (image, currently ticking).
    logo_state: Option<(ImageId, bool)>,
    /// Current logo angle, so each tick turns on from where the last one stopped.
    logo_angle: f32,
    compact_width: f32,
    expanded_height: f32,
    hits: Vec<Hit>,
    /// The hub's snapshot as received.
    raw: Snapshot,
    /// What's shown: `raw` with the user's chosen activity moved to the front.
    snapshot: Snapshot,
    /// The activity the user brought to the front (bubble click or wheel). Holds until
    /// something more urgent arrives, and comes back after it's gone.
    pinned: Option<ActivityId>,
    /// The pointer is over the minimal bubble rather than the pill.
    on_bubble: bool,
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
        shape.StartAnimation(h!("CornerRadius"), &expr("Vector2(Min(p.h / 2, R), Min(p.h / 2, R))", &[("R", island::expanded::RADIUS * s)])?)?;
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
            logo_surface,
            logo,
            logo_state: None,
            logo_angle: 0.0,
            compact_width: core.0,
            expanded_height: island::expanded::MIN_HEIGHT,
            hits: Vec::new(),
            raw: Snapshot::default(),
            snapshot: Snapshot::default(),
            pinned: None,
            on_bubble: false,
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
            to_hub,
            events,
        })
    }

    fn front(&self) -> Option<&Activity> {
        self.snapshot.activities.first()
    }

    /// Apply everything the hub link queued; only the newest snapshot matters.
    pub fn drain_hub(&mut self) {
        let mut latest = None;
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Connected => eprintln!("island: connected to hub"),
                Event::Image(id, bytes) => {
                    if let Err(e) = self.renderer.add_image(id, &bytes) {
                        eprintln!("island: can't decode image: {e}");
                    }
                }
                Event::Snapshot(snapshot) => latest = Some(snapshot),
                Event::Disconnected => latest = Some(Snapshot::default()),
            }
        }
        if let Some(snapshot) = latest {
            if let Err(e) = self.apply(snapshot) {
                eprintln!("island: render failed: {e}");
            }
        }
    }

    fn apply(&mut self, snapshot: Snapshot) -> Result<()> {
        self.raw = snapshot;
        let ordered = self.ordered();
        self.render(ordered)
    }

    /// What to show where (see `order::arrange`).
    fn ordered(&mut self) -> Snapshot {
        let (activities, pin_alive) = order::arrange(self.raw.activities.clone(), self.pinned.as_ref());
        if !pin_alive {
            self.pinned = None;
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

    /// Mouse wheel over the island: cycle through activities in the hub's order.
    pub fn on_wheel(&mut self, delta: i16) {
        let ids: Vec<ActivityId> = self.raw.activities.iter().map(|a| a.id.clone()).collect();
        let Some(current) = self.front().map(|a| a.id.clone()) else { return };
        if ids.len() < 2 {
            return;
        }
        let at = ids.iter().position(|id| *id == current).unwrap_or(0);
        let next = if delta < 0 { (at + 1) % ids.len() } else { (at + ids.len() - 1) % ids.len() };
        self.bring_forward(ids[next].clone());
    }

    fn render(&mut self, snapshot: Snapshot) -> Result<()> {
        let previous = self.front().map(|a| (a.id.clone(), a.priority));
        self.snapshot = snapshot;
        let accent = |a: Option<&Activity>| a.and_then(|a| a.accent).map(render::rgb).unwrap_or(color::accent::DEFAULT);
        let front = self.snapshot.activities.first();
        let second = self.snapshot.activities.get(1);

        let front_accent = accent(front);
        let compact = self.renderer.draw_compact(&self.compact_surface, front.map(|a| &a.compact), front_accent)?;
        let card = self.renderer.draw_expanded(&self.expanded_surface, front.and_then(|a| a.expanded.as_ref()), front_accent)?;
        let second_accent = accent(second);
        let minimal_bars = self.renderer.draw_minimal(&self.minimal_surface, second.map(|a| &a.minimal), second_accent)?;
        let has_second = second.is_some();
        let front = front.map(|a| (a.id.clone(), a.priority, a.expanded.is_some()));

        self.compact_width = compact.width;
        self.place_bars(compact.bars, card.bars, minimal_bars, front_accent, second_accent)?;
        self.place_logo(compact.logo, front_accent)?;
        self.expanded_height = card.height;
        self.hits = card.hits;
        self.props.InsertScalar(h!("eh"), card.height * self.scale)?;
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
        self.refresh_visibility();
        Ok(())
    }

    fn set_state(&mut self, state: State) {
        if self.state == state {
            return;
        }
        self.state = state;
        if state != State::Expanded {
            unsafe { _ = KillTimer(Some(self.hwnd), TIMER_COLLAPSE) };
        }
        self.retarget();
        self.update_bars();
        self.update_logo();
        self.update_tick();
    }

    /// An open card with a playing timeline is the one thing that redraws on a clock, once a
    /// second, and only while it's open; compact never ticks.
    fn update_tick(&self) {
        let live = self.state == State::Expanded
            && self.shown
            && self.front().and_then(|a| a.expanded.as_ref()).is_some_and(|e| e.rows.iter().any(|r| matches!(r, Row::Timeline { playing: true, .. })));
        unsafe {
            if live {
                SetTimer(Some(self.hwnd), TIMER_TICK, TICK_MS, None);
            } else {
                _ = KillTimer(Some(self.hwnd), TIMER_TICK);
            }
        }
    }

    fn redraw_expanded(&mut self) {
        let front = self.snapshot.activities.first();
        let accent = front.and_then(|a| a.accent).map(render::rgb).unwrap_or(color::accent::DEFAULT);
        match self.renderer.draw_expanded(&self.expanded_surface, front.and_then(|a| a.expanded.as_ref()), accent) {
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

    /// One frame: each bar follows its own smooth two-sine wave. Only visible sets move.
    fn tick_bars(&self) {
        const WAVES: [(f32, f32, f32, f32); BARS] = [(1.7, 0.0, 3.1, 1.2), (2.3, 2.1, 1.3, 0.4), (1.1, 4.0, 2.9, 2.6), (2.9, 1.0, 1.9, 5.1)];
        let t = self.bars_clock.elapsed().as_secs_f32() * std::f32::consts::TAU;
        let (front, minimal) = self.moving_sets();
        let front_set = if self.state == State::Expanded { &self.card_bars } else { &self.bars };
        let sets = [(front_set, front, 0.0), (&self.minimal_bars, minimal, 0.9)];
        for (set, moving, phase) in sets {
            if !moving {
                continue;
            }
            for (bar, (f1, p1, f2, p2)) in set.iter().zip(WAVES) {
                let level = (0.6 + 0.25 * (t * f1 + p1 + phase).sin() + 0.15 * (t * f2 + p2).sin()).clamp(BAR_LEVEL_IDLE, 1.0);
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
            State::Expanded => (island::expanded::WIDTH, self.expanded_height),
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
                _ = SetTimer(Some(self.hwnd), TIMER_COLLAPSE, motion::delay::COLLAPSE, None);
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

    /// `x`, `y` are client pixels.
    pub fn on_click(&mut self, x: f32, y: f32) {
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
            }
            return;
        }
        let s = self.scale;
        let win_w = (self.window.right - self.window.left) as f32;
        let cx = (x - (win_w - EXPANDED_SURFACE.0 * s) / 2.0) / s;
        let cy = (y - island::TOP * s) / s;
        if let Some(hit) = render::hit_at(&self.hits, cx, cy) {
            _ = self.to_hub.send(ClientMsg::Action { id: front.id.clone(), action: hit.action_at(cx) });
        }
    }

    pub fn on_timer(&mut self, id: usize) {
        // The bar frame timer is periodic; every other timer here is one-shot.
        if id == TIMER_BARS {
            self.tick_bars();
            return;
        }
        unsafe { _ = KillTimer(Some(self.hwnd), id) };
        match id {
            TIMER_HOVER => self.on_hover_intent(),
            TIMER_TICK => {
                self.redraw_expanded();
                self.update_tick();
            }
            TIMER_SPIN if self.logo_state.is_some_and(|(_, ticking)| ticking) => {
                unsafe { SetTimer(Some(self.hwnd), TIMER_SPIN, motion::logo::INTERVAL, None) };
                self.tick_logo();
            }
            TIMER_REVEAL if self.inside => unsafe {
                SetTimer(Some(self.hwnd), TIMER_REVEAL, motion::delay::REVEAL_HOLD, None);
            },
            TIMER_REVEAL => {
                self.revealed = false;
                self.refresh_visibility();
            }
            TIMER_COLLAPSE if !self.hovered && self.state == State::Expanded => self.set_state(State::Compact),
            _ => {}
        }
    }

    pub fn on_foreground(&mut self, fg: HWND) {
        let hide = is_fullscreen(fg, self.monitor);
        if hide == self.hidden {
            return;
        }
        self.hidden = hide;
        self.refresh_visibility();
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
