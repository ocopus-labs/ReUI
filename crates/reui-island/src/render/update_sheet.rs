//! Test-only: the moments of a ReUI update, drawn offscreen by the island's own renderer
//! as `modules/updates/view.rs` builds them (kept in step by hand): the pill while an
//! update waits, the card offering it, downloading, installing, failing, and the brief
//! after an upgrade. One sheet per theme, at 150 %.
//!
//! `cargo test -p reui-island update_sheet -- --ignored`, PNGs in `target/update-sheet/`.

use std::path::PathBuf;

use reui_design::look::{Appearance, Look, Theme};
use reui_design::layout::expanded_surface;
use reui_design::tokens::island;
use reui_protocol::{Button, ButtonContent, ButtonRole, CardSize, CompactView, Element, ExpandedView, Headline, Rgb, Row, Tone};
use windows::core::Result;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::WinRT::{CreateDispatcherQueueController, DispatcherQueueOptions, DQTAT_COM_NONE, DQTYPE_THREAD_CURRENT};
use windows::UI::Composition::Compositor;
use windows_numerics::Matrix3x2;

use super::panel_sheet::{canvas, save};
use super::{d2d, rgb, Device, Interaction, Renderer, COMPACT_SURFACE};

const ACCENT: Rgb = Rgb(10, 132, 255);
const DOWNLOAD: char = '\u{E896}';
const VERSION: &str = "Version 0.4.0";
const NOTES: &str = "The clipboard's new look: type filters and landscape cards";

fn button(action: &str, label: &str, role: ButtonRole) -> Button {
    Button { action: action.into(), content: ButtonContent::Text(label.into()), role }
}

fn card(leading_hero: &str, detail: Option<&str>, rows: Vec<Row>, eyebrow: &str, size: CardSize) -> ExpandedView {
    ExpandedView {
        leading: Element::Empty,
        eyebrow: Some(eyebrow.into()),
        headline: Headline::Text(leading_hero.into()),
        detail: detail.map(Into::into),
        trailing: Element::Empty,
        rows,
        context: None,
        size,
        tint: false,
        pager: None,
        open: None,
    }
}

/// What the island shows, in order: (caption, pill, card).
fn moments() -> Vec<(&'static str, Option<CompactView>, Option<ExpandedView>)> {
    let answers = |main: &str| vec![Row::Buttons(vec![button("later", "Later", ButtonRole::Secondary), button("install", main, ButtonRole::Accent)])];
    let mark = Element::Glyph { glyph: DOWNLOAD, tone: Tone::Accent };
    let text = |t: &str, tone| Element::Text { text: t.into(), tone };
    vec![
        ("pill: an update is waiting", Some(CompactView { leading: mark.clone(), trailing: text("Update", Tone::Accent) }), None),
        ("card: offered", None, Some(card(VERSION, Some(NOTES), answers("Install"), "ReUI", CardSize::Roomy))),
        ("pill: downloading", Some(CompactView { leading: mark.clone(), trailing: Element::Ring(0.42) }), None),
        ("card: downloading", None, Some(card("Downloading", Some(VERSION), vec![Row::Progress { fraction: 0.42, leading: None, trailing: Some("42 %".into()) }], "ReUI", CardSize::Roomy))),
        ("card: installing", None, Some(card("Installing", Some(VERSION), vec![], "ReUI", CardSize::Roomy))),
        (
            "card: failed",
            None,
            Some(card("Couldn't update", Some("The download stopped. Check your connection."), answers("Try again"), "ReUI", CardSize::Roomy)),
        ),
        (
            "card: after the upgrade",
            None,
            Some(ExpandedView {
                leading: Element::Glyph { glyph: '\u{E735}', tone: Tone::Secondary },
                ..card("Updated to ReUI 0.4.0", None, vec![Row::Buttons(vec![button("open", "What's new", ButtonRole::Primary)])], "ReUI", CardSize::Regular)
            }),
        ),
    ]
}

#[test]
#[ignore = "draws PNGs for review: cargo test -p reui-island update_sheet -- --ignored"]
fn update_sheet() {
    unsafe { _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let _queue = unsafe { CreateDispatcherQueueController(DispatcherQueueOptions { dwSize: size_of::<DispatcherQueueOptions>() as u32, threadType: DQTYPE_THREAD_CURRENT, apartmentType: DQTAT_COM_NONE }) }.expect("a dispatcher queue");
    let compositor = Compositor::new().expect("a compositor");
    let device = Device::new(&compositor).expect("a device");
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/update-sheet");
    std::fs::create_dir_all(&dir).unwrap();
    for (theme, name) in [(Theme::Dark, "dark"), (Theme::Light, "light")] {
        let look = Look::new(&Appearance { theme, ..Appearance::default() }, false, None);
        let renderer = Renderer::new(&device, 1.5, look).unwrap();
        let path = dir.join(format!("{name}.png"));
        unsafe { draw(&device, &renderer, &path) }.unwrap();
        println!("{}", path.display());
    }
}

/// Every moment down one sheet on a grey backdrop, each shape on its own row.
unsafe fn draw(device: &Device, renderer: &Renderer, path: &std::path::Path) -> Result<()> {
    let s = renderer.scale;
    let accent = rgb(ACCENT);
    let (sw, _) = expanded_surface(renderer.look());
    let (margin, gap) = (32.0, 28.0);
    let moments = moments();
    // Measure first: each card's height.
    let probe = canvas(device, 1, 1)?.1;
    let mut heights = Vec::new();
    for (_, pill, open) in &moments {
        heights.push(match (pill, open) {
            (_, Some(view)) => renderer.expanded_into(&probe, Some(view), accent, &Interaction::default())?.height,
            _ => island::HEIGHT,
        });
    }
    let total: f32 = heights.iter().sum::<f32>() + gap * (moments.len() - 1) as f32 + 2.0 * margin;
    let (bitmap, rt) = canvas(device, ((sw + 2.0 * margin) * s).ceil() as u32, (total * s).ceil() as u32)?;
    rt.BeginDraw();
    rt.Clear(Some(&d2d(rgb(Rgb(96, 104, 120)))));
    let material = rt.CreateSolidColorBrush(&d2d(renderer.look().palette.background), None)?;
    let mut y = margin;
    for ((_, pill, open), h) in moments.iter().zip(&heights) {
        match (pill, open) {
            (_, Some(view)) => {
                let layout = renderer.expanded_into(&probe, Some(view), accent, &Interaction::default())?;
                let dx = (sw - layout.width) / 2.0;
                let r = layout.radius * s;
                let shape = D2D1_ROUNDED_RECT { rect: D2D_RECT_F { left: (margin + dx) * s, top: y * s, right: (margin + dx + layout.width) * s, bottom: (y + layout.height) * s }, radiusX: r, radiusY: r };
                rt.FillRoundedRectangle(&shape, &material);
                rt.SetTransform(&Matrix3x2::translation(margin * s, y * s));
                renderer.expanded_into(&rt, Some(view), accent, &Interaction::default())?;
                rt.SetTransform(&Matrix3x2::identity());
            }
            (Some(view), None) => {
                let layout = renderer.compact_into(&probe, Some(view), accent, None)?;
                let left = margin + (sw - COMPACT_SURFACE.0) / 2.0 + reui_design::layout::compact::left(layout.width);
                let r = renderer.look().rounded(island::HEIGHT) * s;
                let shape = D2D1_ROUNDED_RECT { rect: D2D_RECT_F { left: left * s, top: y * s, right: (left + layout.width) * s, bottom: (y + island::HEIGHT) * s }, radiusX: r, radiusY: r };
                rt.FillRoundedRectangle(&shape, &material);
                rt.SetTransform(&Matrix3x2::translation((margin + (sw - COMPACT_SURFACE.0) / 2.0) * s, y * s));
                renderer.compact_into(&rt, Some(view), accent, None)?;
                rt.SetTransform(&Matrix3x2::identity());
            }
            (None, None) => {}
        }
        y += h + gap;
    }
    rt.EndDraw(None, None)?;
    save(device, &bitmap, path)
}
