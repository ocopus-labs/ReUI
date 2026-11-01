//! Test-only: the clipboard panel drawn offscreen by the island's own renderer (the same
//! `expanded_into` the pill paints with), with the clips of the design reference
//! (`docs/images/design/clipboard-v3.png`), so the two can be compared side by side without
//! opening a window or taking the keyboard. Dark and light, at 100 % and 150 %.
//!
//! `cargo test -p reui-island clipboard_panel_sheet -- --ignored`, PNGs in
//! `target/clipboard-panel/`.

use std::path::{Path, PathBuf};

use reui_design::look::{Appearance, Look, Theme};
use reui_design::layout::expanded_surface;
use reui_protocol::{Button, ButtonContent, ButtonRole, CardSize, Element, ExpandedView, Headline, ImageId, Item, Preview, Row, Tab, Tone};
use windows::core::{Result, GUID, PCWSTR};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::WinRT::{CreateDispatcherQueueController, DispatcherQueueOptions, DQTAT_COM_NONE, DQTYPE_THREAD_CURRENT};
use windows::UI::Composition::Compositor;

use super::{d2d, rgb, Device, Interaction, Renderer};

const IMAGE: ImageId = ImageId(7);

/// The design's four clips (and a colour after them), as the hub sends them.
fn view() -> ExpandedView {
    let tab = |action: &str, glyph: char, selected: bool| Tab { label: action.into(), count: 0, selected, action: action.into(), glyph: Some(glyph), menu: vec![] };
    let filters = vec![tab("all", '\u{E81C}', true), tab("text", 'T', false), tab("links", '\u{E71B}', false), tab("images", '\u{EB9F}', false), tab("files", '\u{E8A5}', false), tab("colors", '\u{E790}', false)];
    let buttons = vec![
        Button { action: String::new(), content: ButtonContent::Count(4), role: ButtonRole::Secondary },
        Button { action: "clear".into(), content: ButtonContent::Labelled('\u{E74D}', "Clear".into()), role: ButtonRole::Secondary },
        Button { action: "settings".into(), content: ButtonContent::Glyph('\u{E713}'), role: ButtonRole::Plain },
    ];
    let item = |preview, tag: &str, tone, name: Option<&str>, meta: Option<&str>, age: &str| Item {
        action: "paste".into(),
        preview,
        icon: None,
        age: age.into(),
        tag: Some(tag.into()),
        tag_tone: tone,
        name: name.map(Into::into),
        meta: meta.map(Into::into),
        pinned: false,
        actions: vec![],
        drag: None,
    };
    let items = vec![
        item(Preview::Files { name: "Itinerary.pdf".into(), count: 3 }, "3 files", Tone::Secondary, Some("Itinerary.pdf"), None, "just now"),
        item(
            Preview::Link { host: "en.wikipedia.org".into(), path: "/wiki/Sea_of_clouds".into(), title: Some("Sea of clouds".into()), site: Some("Wikipedia".into()), image: None },
            "links",
            Tone::Secondary,
            None,
            Some("en.wikipedia.org"),
            "1m ago",
        ),
        item(Preview::Image(IMAGE), "image", Tone::Accent, Some("Summit.png"), None, "2m ago"),
        item(Preview::Text("A sea of clouds forms when a cloud fills the valleys below a summit, and those above it look down on a white ocean.".into()), "text", Tone::Secondary, None, None, "4m ago"),
        item(Preview::Color(reui_protocol::Rgb(94, 92, 230)), "color", Tone::Secondary, Some("#5E5CE6"), None, "1h ago"),
    ];
    ExpandedView {
        leading: Element::Empty,
        eyebrow: None,
        headline: Headline::Text(String::new()),
        detail: None,
        trailing: Element::Empty,
        rows: vec![Row::Search { query: String::new(), buttons, placeholder: Some("Search history\u{2026}".into()), glyph: None, filters }, Row::Items(items)],
        context: None,
        size: CardSize::Gallery,
        tint: false,
        pager: None,
        open: None,
    }
}

#[test]
#[ignore = "draws PNGs for review: cargo test -p reui-island clipboard_panel_sheet -- --ignored"]
fn clipboard_panel_sheet() {
    unsafe { _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let _queue = unsafe { CreateDispatcherQueueController(DispatcherQueueOptions { dwSize: size_of::<DispatcherQueueOptions>() as u32, threadType: DQTYPE_THREAD_CURRENT, apartmentType: DQTAT_COM_NONE }) }.expect("a dispatcher queue");
    let compositor = Compositor::new().expect("a compositor");
    let device = Device::new(&compositor).expect("a device");
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/clipboard-panel");
    std::fs::create_dir_all(&dir).unwrap();
    let image = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/images/hero.png")).unwrap();
    for (theme, name) in [(Theme::Dark, "dark"), (Theme::Light, "light")] {
        for scale in [1.0f32, 1.5] {
            let look = Look::new(&Appearance { theme, ..Appearance::default() }, false, None);
            let mut renderer = Renderer::new(&device, scale, look).unwrap();
            renderer.add_image(IMAGE, &image).unwrap();
            let path = dir.join(format!("{name}-{}.png", (scale * 100.0) as u32));
            draw(&device, &renderer, &path).unwrap();
            println!("{}", path.display());
        }
    }
}

/// The panel on a blue backdrop (the reference's), its material behind the content.
fn draw(device: &Device, renderer: &Renderer, path: &Path) -> Result<()> {
    let s = renderer.scale;
    let (sw, _) = expanded_surface(renderer.look());
    let margin = 40.0;
    let view = view();
    unsafe {
        let probe = canvas(device, 1, 1)?;
        let layout = renderer.expanded_into(&probe.1, Some(&view), rgb(reui_protocol::Rgb(10, 132, 255)), &Interaction::default())?;
        let (w, h) = (((sw + 2.0 * margin) * s).ceil() as u32, ((layout.height + 2.0 * margin) * s).ceil() as u32);
        let (bitmap, rt) = canvas(device, w, h)?;
        rt.BeginDraw();
        rt.Clear(Some(&d2d(rgb(reui_protocol::Rgb(16, 48, 208)))));
        let dx = (sw - layout.width) / 2.0;
        let shape = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F { left: (margin + dx) * s, top: margin * s, right: (margin + dx + layout.width) * s, bottom: (margin + layout.height) * s },
            radiusX: layout.radius * s,
            radiusY: layout.radius * s,
        };
        rt.FillRoundedRectangle(&shape, &rt.CreateSolidColorBrush(&d2d(renderer.look().palette.background), None)?);
        rt.SetTransform(&windows_numerics::Matrix3x2::translation(margin * s, margin * s));
        renderer.expanded_into(&rt, Some(&view), rgb(reui_protocol::Rgb(10, 132, 255)), &Interaction::default())?;
        rt.EndDraw(None, None)?;
        save(device, &bitmap, path)
    }
}

unsafe fn canvas(device: &Device, w: u32, h: u32) -> Result<(IWICBitmap, ID2D1RenderTarget)> {
    let bitmap = device.wic.CreateBitmap(w, h, &GUID_WICPixelFormat32bppPBGRA, WICBitmapCacheOnLoad)?;
    let props = D2D1_RENDER_TARGET_PROPERTIES {
        pixelFormat: D2D1_PIXEL_FORMAT { format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
        dpiX: 96.0,
        dpiY: 96.0,
        ..Default::default()
    };
    let base: &ID2D1Factory = &device.factory;
    let rt = base.CreateWicBitmapRenderTarget(&bitmap, &props)?;
    rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
    Ok((bitmap, rt))
}

unsafe fn save(device: &Device, bitmap: &IWICBitmap, path: &Path) -> Result<()> {
    let stream = device.wic.CreateStream()?;
    let wide: Vec<u16> = path.to_string_lossy().encode_utf16().chain(Some(0)).collect();
    stream.InitializeFromFilename(PCWSTR(wide.as_ptr()), 0x4000_0000)?;
    let encoder = device.wic.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null::<GUID>())?;
    encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
    let mut frame = None;
    encoder.CreateNewFrame(&mut frame, std::ptr::null_mut())?;
    let frame = frame.expect("a frame");
    frame.Initialize(None)?;
    frame.WriteSource(bitmap, std::ptr::null())?;
    frame.Commit()?;
    encoder.Commit()
}
