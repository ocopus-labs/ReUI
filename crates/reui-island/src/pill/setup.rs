//! Building the pill: the window's composition tree (layers, clips, surfaces, the bars and
//! the minimal bubble) and the property set its morphs animate.

use std::sync::mpsc::Receiver;

use reui_design::look::{Appearance, Look};
use reui_design::tokens::{component, island, look as range, motion};
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
use crate::render::{expanded_surface, Renderer, COMPACT_SURFACE, MINIMAL_SURFACE};
use crate::settings::Visibility;

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
    ) -> Result<Self> {
        // Size scales the whole pill and its type: it's drawn at that much more than the
        // display's scale, so every DIP below grows with it.
        let scale = dpi * look.size;
        let s = scale;
        let top = look.top();
        let expanded_dims = expanded_surface(&look);
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
        let shape = compositor.CreateRoundedRectangleGeometry()?;
        shape.StartAnimation(h!("Size"), &expr("Vector2(p.w, p.h)", &[])?)?;
        shape.StartAnimation(h!("CornerRadius"), &expr(&format!("Vector2({corner}, {corner})"), &corner_scalars)?)?;
        pill.SetClip(&compositor.CreateGeometricClipWithGeometry(&shape)?)?;
        root.Children()?.InsertAtTop(&pill)?;

        let renderer = Renderer::new(&compositor, s, look)?;
        let compact_surface = renderer.surface(COMPACT_SURFACE)?;
        let expanded_surface = renderer.surface(expanded_dims)?;
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
        let expanded = surface_sprite(&compositor, &expanded_surface, expanded_dims, s)?;
        expanded.SetCenterPoint(Vector3::new(expanded_dims.0 / 2.0 * s, 0.0, 0.0))?;
        expanded.StartAnimation(h!("Offset"), &expr("Vector3(Round((p.w - We) / 2), 0, 0)", &[("We", expanded_dims.0 * s)])?)?;
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
        circle.SetSize(Vector2::new(size, size))?;
        circle.SetCornerRadius(Vector2::new(round, round))?;
        bubble.SetClip(&compositor.CreateGeometricClipWithGeometry(&circle)?)?;
        bubble.SetCenterPoint(Vector3::new(size / 2.0, size / 2.0, 0.0))?;
        bubble.StartAnimation(
            h!("Offset"),
            &expr("Vector3(Round((W + p.w) / 2 + G), Round(T), 0)", &[("W", win_w), ("G", island::minimal::GAP * s), ("T", top * s)])?,
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
            on_desktop: crate::desktop::on_desktop(crate::desktop::front(unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() }), false),
            show_on_desktop: true,
            hide_in_fullscreen: true,
            monitor_choice: MonitorChoice::default(),
            drag_guard: DragGuard::default(),
            visibility,
            paused: false,
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
            engaged: false,
            choice_cursor: None,
            block_scroll: 0.0,
            block_max: 0.0,
            choice_spans: None,
            to_hub,
            events,
        })
    }
}

fn surface_sprite(c: &Compositor, surface: &CompositionDrawingSurface, dips: (f32, f32), s: f32) -> Result<SpriteVisual> {
    let sprite = c.CreateSpriteVisual()?;
    sprite.SetSize(Vector2::new((dips.0 * s).ceil(), (dips.1 * s).ceil()))?;
    sprite.SetBrush(&c.CreateSurfaceBrushWithSurface(surface)?)?;
    Ok(sprite)
}
