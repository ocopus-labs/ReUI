//! Building the pill: the window's composition tree (layers, clips, surfaces, the bars and
//! the minimal bubble) and the property set its morphs animate.

use std::sync::mpsc::Receiver;

use reui_design::look::{Appearance, Look};
use reui_design::tokens::{component, island, look as range, motion, notch};
use reui_design::Color;
use reui_protocol::{
    ClientMsg, Snapshot,
};
use tokio::sync::mpsc::UnboundedSender;
use windows::core::{h, Interface, Result, HSTRING};
use windows::UI::Composition::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::WinRT::Composition::ICompositorDesktopInterop;
use windows_numerics::{Vector2, Vector3};

use crate::features::clipboard::Panel;
use crate::features::level::Levels;
use crate::features::music::Swipe;
use crate::features::visualizer::Visualizer;
use crate::ipc::Event;
use crate::drag_guard::DragGuard;
use crate::place::MonitorChoice;
use crate::render::{expanded_surface, Renderer, COMPACT_SURFACE, EAR_SURFACE, MINIMAL_SURFACE, SWITCHER_SURFACE};
use crate::settings::Visibility;

use super::switcher::Tabs;
use super::{Pill, State};
use super::visuals::bar_visuals;

fn color_brush(c: &Compositor, color: Color) -> Result<CompositionColorBrush> {
    let [r, g, b, a] = color.to_rgba8();
    c.CreateColorBrushWithColor(windows::UI::Color { A: a, R: r, G: g, B: b })
}

impl Pill {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        hwnd: HWND,
        dpi: f32,
        window: RECT,
        monitor: HMONITOR,
        reduced_motion: bool,
        visibility: Visibility,
        appearance: Appearance,
        look: Look,
        to_hub: UnboundedSender<ClientMsg>,
        events: Receiver<Event>,
        gpu: Option<Gpu>,
    ) -> Result<Self> {
        // Size scales the whole pill and its type: it's drawn at that much more than the
        // display's scale, so every DIP below grows with it.
        let scale = dpi * look.size;
        let s = scale;
        let top = look.top();
        let expanded_dims = expanded_surface(&look);
        // A rebuild (new look or scale) keeps the old pill's device and renderer.
        let (compositor, target, renderer) = match gpu {
            Some(Gpu { compositor, target, mut renderer }) => {
                renderer.retarget(s, look)?;
                (compositor, target, renderer)
            }
            None => {
                let compositor = Compositor::new()?;
                let target = unsafe { compositor.cast::<ICompositorDesktopInterop>()?.CreateDesktopWindowTarget(hwnd, true)? };
                let renderer = Renderer::new(&compositor, s, look)?;
                (compositor, target, renderer)
            }
        };
        let root = compositor.CreateContainerVisual()?;
        target.SetRoot(&root)?;
        // Showing and hiding scale around the top centre, where the island meets the screen edge.
        root.SetCenterPoint(Vector3::new((window.right - window.left) as f32 / 2.0, 0.0, 0.0))?;

        let core = (island::idle::WIDTH, island::HEIGHT);
        let peek_h = core.1 + island::compact::PEEK_GROW;
        let rest = idle_size(&look);
        let props = compositor.CreatePropertySet()?;
        props.InsertScalar(h!("w"), rest.0 * s)?;
        props.InsertScalar(h!("h"), rest.1 * s)?;
        props.InsertScalar(h!("m"), 0.0)?;
        props.InsertScalar(h!("eh"), island::expanded::MIN_HEIGHT * s)?;
        // Expanded corner radius: the card's, or the wide panel's calmer one.
        props.InsertScalar(h!("r"), look.radius() * s)?;
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
        pill.SetBrush(&color_brush(&compositor, look.palette.background)?)?;
        pill.StartAnimation(h!("Size"), &expr("Vector2(p.w, p.h)", &[])?)?;
        pill.StartAnimation(h!("Offset"), &expr("Vector3(Round((W - p.w) / 2), Round(T), 0)", &[("W", win_w), ("T", top * s)])?)?;
        // Corners: the compact pill's (a capsule at full roundness, `look.rounded`) easing
        // into the card's `p.r` as the pill opens, never more than half its height. At full
        // roundness this is exactly `Min(p.h / 2, p.r)`.
        let corner = format!(
            "Min(p.h / 2, Min(p.r, (K + (p.h / 2 - K) * R) + (p.r - (K + (p.h / 2 - K) * R)) * {t}))",
            t = t
        );
        let corner_scalars = [("K", range::roundness::MIN_RADIUS * s), ("R", look.roundness)];
        let notched = look.notch();
        // A notch's rectangle reaches one corner radius above the screen's edge, so its top
        // corners are off-screen and the top is flat; size springs and hit testing stay.
        let outline = || -> Result<CompositionRoundedRectangleGeometry> {
            let shape = compositor.CreateRoundedRectangleGeometry()?;
            if notched {
                shape.StartAnimation(h!("Size"), &expr(&format!("Vector2(p.w, p.h + {corner})"), &corner_scalars)?)?;
                shape.StartAnimation(h!("Offset"), &expr(&format!("Vector2(0, -({corner}))"), &corner_scalars)?)?;
            } else {
                shape.StartAnimation(h!("Size"), &expr("Vector2(p.w, p.h)", &[])?)?;
            }
            shape.StartAnimation(h!("CornerRadius"), &expr(&format!("Vector2({corner}, {corner})"), &corner_scalars)?)?;
            Ok(shape)
        };
        pill.SetClip(&compositor.CreateGeometricClipWithGeometry(&outline()?)?)?;
        root.Children()?.InsertAtTop(&pill)?;

        let compact_surface = renderer.surface(COMPACT_SURFACE)?;
        let expanded_surface = renderer.surface(expanded_dims)?;
        let minimal_surface = renderer.surface(MINIMAL_SURFACE)?;

        // Compact content: centred, nudged down while peeking, gone early in an expansion.
        // Its place on screen is rounded once (the window's centre), and its offset inside
        // the pill is that minus the pill's own rounded offset. Rounding each separately
        // made them disagree by a pixel from frame to frame as the width moved: the content
        // shook while it should stand still.
        let compact = surface_sprite(&compositor, &compact_surface, COMPACT_SURFACE, s)?;
        compact.StartAnimation(
            h!("Offset"),
            &expr(
                "Vector3(Round((W - Ws) / 2) - Round((W - p.w) / 2), Round(Min((p.h - Hc) / 2, D)), 0)",
                &[("W", win_w), ("Ws", COMPACT_SURFACE.0 * s), ("Hc", core.1 * s), ("D", island::compact::PEEK_GROW / 2.0 * s)],
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
        let expanded = surface_sprite(&compositor, &expanded_surface, expanded_dims, s)?;
        expanded.SetCenterPoint(Vector3::new(expanded_dims.0 / 2.0 * s, 0.0, 0.0))?;
        // Held still on screen while the pill grows around it, as the compact content is.
        expanded.StartAnimation(h!("Offset"), &expr("Vector3(Round((W - We) / 2) - Round((W - p.w) / 2), 0, 0)", &[("W", win_w), ("We", expanded_dims.0 * s)])?)?;
        expanded.StartAnimation(h!("Opacity"), &expr(&format!("Clamp(({t} - {start}) / (1 - {start}), 0, 1)"), &[])?)?;
        let grow = format!("{from} + (1 - {from}) * {t}");
        expanded.StartAnimation(h!("Scale"), &expr(&format!("Vector3({grow}, {grow}, 1)"), &[])?)?;
        pill.Children()?.InsertAtTop(&expanded)?;
        let card_bars = bar_visuals(&compositor, &expanded.Children()?, &bar_brush, s)?;

        // Minimal bubble: the second activity, detached to the right of the pill.
        let size = island::minimal::SIZE * s;
        let bubble = compositor.CreateSpriteVisual()?;
        bubble.SetSize(Vector2::new(size, size))?;
        bubble.SetBrush(&color_brush(&compositor, look.palette.background)?)?;
        let circle = compositor.CreateRoundedRectangleGeometry()?;
        let round = look.rounded(island::minimal::SIZE) * s;
        circle.SetCornerRadius(Vector2::new(round, round))?;
        if notched {
            // The notch's trailing end: flat-topped like the notch, joined to the edge.
            circle.SetSize(Vector2::new(size, size + round))?;
            circle.SetOffset(Vector2::new(0.0, -round))?;
        } else {
            circle.SetSize(Vector2::new(size, size))?;
        }
        bubble.SetClip(&compositor.CreateGeometricClipWithGeometry(&circle)?)?;
        let bubble_edge = edge(&compositor, &circle, look.palette.hairline)?;
        bubble.SetCenterPoint(Vector3::new(size / 2.0, size / 2.0, 0.0))?;
        let bubble_gap = if notched { notch::JOIN_GAP } else { island::minimal::GAP } * s;
        bubble.StartAnimation(
            h!("Offset"),
            &expr("Vector3(Round((W + p.w) / 2 + G), Round(T), 0)", &[("W", win_w), ("G", bubble_gap), ("T", top * s)])?,
        )?;
        bubble.StartAnimation(h!("Opacity"), &expr(&format!("p.m * {compact_fade}"), &[])?)?;
        // A notch's end stays joined to the edge: it fades in without growing.
        if !notched {
            bubble.StartAnimation(h!("Scale"), &expr("Vector3(0.6 + 0.4 * p.m, 0.6 + 0.4 * p.m, 1)", &[])?)?;
        }
        bubble.Children()?.InsertAtTop(&surface_sprite(&compositor, &minimal_surface, MINIMAL_SURFACE, s)?)?;
        let minimal_brush = compositor.CreateColorBrush()?;
        let minimal_bars = bar_visuals(&compositor, &bubble.Children()?, &minimal_brush, s)?;
        root.Children()?.InsertAtTop(&bubble)?;

        // The view switcher: centred under the card, riding its height spring (`p.h`), in
        // and out with `p.v` (dropping `rise` into place), as wide as its tabs (`p.sw`).
        props.InsertScalar(h!("v"), 0.0)?;
        props.InsertScalar(h!("sw"), 0.0)?;
        let switcher_surface = renderer.surface(SWITCHER_SURFACE)?;
        let switcher = compositor.CreateSpriteVisual()?;
        switcher.SetBrush(&color_brush(&compositor, look.palette.background)?)?;
        let switcher_clip = compositor.CreateRoundedRectangleGeometry()?;
        switcher.SetClip(&compositor.CreateGeometricClipWithGeometry(&switcher_clip)?)?;
        let rise = if reduced_motion { 0.0 } else { island::switcher::RISE * s };
        let place = "Vector3(Round((W - p.sw) / 2), Round(T + p.h + G - (1 - p.v) * R), 0)";
        switcher.StartAnimation(h!("Offset"), &expr(place, &[("W", win_w), ("T", top * s), ("G", island::switcher::GAP * s), ("R", rise)])?)?;
        switcher.StartAnimation(h!("Opacity"), &expr("p.v", &[])?)?;
        switcher.Children()?.InsertAtTop(&surface_sprite(&compositor, &switcher_surface, SWITCHER_SURFACE, s)?)?;
        let switcher_edge = if look.palette.hairline.a > 0.0 {
            let line = edge(&compositor, &switcher_clip, look.palette.hairline)?;
            switcher.Children()?.InsertAtTop(&line)?;
            Some(line)
        } else {
            None
        };
        root.Children()?.InsertAtTop(&switcher)?;

        // A decision waiting behind the open place: a bubble `waiting-gap` after the
        // switcher's end, in with `p.wv` (and the switcher's own `p.v`), scaling up from its
        // centre as the second activity's bubble does.
        props.InsertScalar(h!("wv"), 0.0)?;
        let waiting_geometry = reui_design::layout::switcher::Switcher::new(&look, 0).waiting();
        let waiting_px = waiting_geometry.size * s;
        let waiting_surface = renderer.surface((waiting_geometry.size, waiting_geometry.size))?;
        let waiting = compositor.CreateSpriteVisual()?;
        waiting.SetSize(Vector2::new(waiting_px, waiting_px))?;
        waiting.SetBrush(&color_brush(&compositor, look.palette.background)?)?;
        let waiting_clip = compositor.CreateRoundedRectangleGeometry()?;
        waiting_clip.SetSize(Vector2::new(waiting_px, waiting_px))?;
        let waiting_round = waiting_geometry.radius * s;
        waiting_clip.SetCornerRadius(Vector2::new(waiting_round, waiting_round))?;
        waiting.SetClip(&compositor.CreateGeometricClipWithGeometry(&waiting_clip)?)?;
        waiting.SetCenterPoint(Vector3::new(waiting_px / 2.0, waiting_px / 2.0, 0.0))?;
        let place = "Vector3(Round((W + p.sw) / 2 + Gw), Round(T + p.h + G - (1 - p.v) * R), 0)";
        let place_scalars = [("W", win_w), ("T", top * s), ("G", island::switcher::GAP * s), ("Gw", island::switcher::WAITING_GAP * s), ("R", rise)];
        waiting.StartAnimation(h!("Offset"), &expr(place, &place_scalars)?)?;
        waiting.StartAnimation(h!("Opacity"), &expr("p.v * p.wv", &[])?)?;
        waiting.StartAnimation(h!("Scale"), &expr("Vector3(0.6 + 0.4 * p.wv, 0.6 + 0.4 * p.wv, 1)", &[])?)?;
        waiting.Children()?.InsertAtTop(&surface_sprite(&compositor, &waiting_surface, (waiting_geometry.size, waiting_geometry.size), s)?)?;
        if look.palette.hairline.a > 0.0 {
            let line = edge(&compositor, &waiting_clip, look.palette.hairline)?;
            line.SetSize(Vector2::new(waiting_px, waiting_px))?;
            waiting.Children()?.InsertAtTop(&line)?;
        }
        root.Children()?.InsertAtTop(&waiting)?;

        let tabs = Tabs {
            sprite: switcher,
            clip: switcher_clip,
            edge: switcher_edge,
            surface: switcher_surface,
            waiting_surface,
            waiting: None,
            waiting_hover: false,
            waiting_drawn: None,
            places: Vec::new(),
            enabled: true,
            selected: None,
            hover: None,
            layout: reui_design::layout::switcher::Switcher::new(&look, 0),
            drawn: None,
            travel: 0,
        };

        // The theme's hairline just inside the edge (light: it holds the white island's
        // shape on white windows; dark has none). Over the content, following the morphs.
        if look.palette.hairline.a > 0.0 {
            let pill_edge = edge(&compositor, &outline()?, look.palette.hairline)?;
            pill_edge.StartAnimation(h!("Size"), &expr("Vector2(p.w, p.h)", &[])?)?;
            pill.Children()?.InsertAtTop(&pill_edge)?;
            bubble_edge.SetSize(Vector2::new(size, size))?;
            bubble.Children()?.InsertAtTop(&bubble_edge)?;
        }

        // The notch's ears, over everything so their overlap hides the shape's edge where
        // they join it. The right one moves out to the second activity's end while it shows.
        if notched {
            let ear_px = EAR_SURFACE.0 * s;
            let one = s;
            for right in [false, true] {
                let surface = renderer.surface(EAR_SURFACE)?;
                renderer.draw_ear(&surface, right)?;
                let ear = surface_sprite(&compositor, &surface, EAR_SURFACE, s)?;
                let x = if right {
                    format!("Round((W - p.w) / 2 + p.w + p.m * {compact_fade} * (G + S) - O)")
                } else {
                    "Round((W - p.w) / 2) - E + O".to_string()
                };
                let scalars = [("W", win_w), ("E", ear_px), ("O", one), ("G", bubble_gap), ("S", size)];
                ear.StartAnimation(h!("Offset"), &expr(&format!("Vector3({x}, 0, 0)"), &scalars)?)?;
                root.Children()?.InsertAtTop(&ear)?;
            }
        }

        Ok(Self {
            hwnd,
            scale,
            dpi,
            look,
            appearance,
            window,
            monitor,
            reduced_motion,
            lyric_line: None,
            lyric_change: None,
            levels: Levels::default(),
            level_ticking: false,
            _keys: KeysRelease,
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
            target: rest,
            minimal: false,
            inside: false,
            hovered: false,
            hidden: false,
            on_desktop: crate::desktop::on_desktop(crate::desktop::front(unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() }), false),
            show_on_desktop: true,
            hide_in_fullscreen: true,
            monitor_choice: MonitorChoice::default(),
            drag_guard: DragGuard::default(),
            visibility,
            paused: false,
            hidden_by_user: false,
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
            hover_card: None,
            swipe: Swipe::default(),
            pager_travel: 0,
            engaged: false,
            choice_cursor: None,
            block_scroll: 0.0,
            block_max: 0.0,
            choice_spans: None,
            tabs,
            idle: Default::default(),
            gate: Default::default(),
            intent: Default::default(),
            to_hub,
            events,
        })
    }
}

/// Releases the keyboard hook when the pill goes. A field rather than `Drop` on `Pill`, so a
/// rebuild can move the compositor and renderer out of the old pill ([`Pill::into_gpu`]).
pub(super) struct KeysRelease;

impl Drop for KeysRelease {
    fn drop(&mut self) {
        crate::features::clipboard_keys::release();
    }
}

/// What a rebuild keeps: creating these takes ~140 ms (the D3D device, the factories, the
/// fonts), while the visual tree a new look needs takes a few.
pub struct Gpu {
    compositor: Compositor,
    target: windows::UI::Composition::Desktop::DesktopWindowTarget,
    renderer: Renderer,
}

impl Pill {
    /// The old pill's compositor, window target and renderer, for the next one; the rest
    /// (its visuals, timers' state, the keyboard hook) goes.
    pub fn into_gpu(self) -> Gpu {
        Gpu { compositor: self.compositor, target: self._target, renderer: self.renderer }
    }
}

/// The resting pill's size: a notch at rest is narrower and only about a menu bar tall.
pub(super) fn idle_size(look: &Look) -> (f32, f32) {
    if look.notch() {
        (notch::IDLE_WIDTH, notch::IDLE_HEIGHT)
    } else {
        (island::idle::WIDTH, island::HEIGHT)
    }
}

/// A stroke along `shape` in `color`, two pixels wide and centred on the edge, so the
/// clip it sits under leaves one crisp physical pixel inside.
fn edge(c: &Compositor, shape: &CompositionRoundedRectangleGeometry, color: Color) -> Result<ShapeVisual> {
    let visual = c.CreateShapeVisual()?;
    let stroke = c.CreateSpriteShapeWithGeometry(shape)?;
    stroke.SetStrokeBrush(&color_brush(c, color)?)?;
    stroke.SetStrokeThickness(2.0)?;
    visual.Shapes()?.Append(&stroke)?;
    Ok(visual)
}

fn surface_sprite(c: &Compositor, surface: &CompositionDrawingSurface, dips: (f32, f32), s: f32) -> Result<SpriteVisual> {
    let sprite = c.CreateSpriteVisual()?;
    sprite.SetSize(Vector2::new((dips.0 * s).ceil(), (dips.1 * s).ceil()))?;
    sprite.SetBrush(&c.CreateSurfaceBrushWithSurface(surface)?)?;
    Ok(sprite)
}
