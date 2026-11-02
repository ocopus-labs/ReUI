//! Test-only: draws the clipboard panel through the GPU path the island uses (the shared
//! D2D device behind the compositor, a device context) and through the software path the
//! review sheets use (a WIC render target), then compares a card's colour where only its
//! fill and grain are. They should match; a card that reads whitish on screen but dark in
//! the sheets would show here as a difference.
//!
//! `cargo test -p reui-island gpu_and_software_cards_match -- --ignored --nocapture`

use windows::core::Interface;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::System::Com::*;
use windows::Win32::System::WinRT::Composition::ICompositionGraphicsDeviceInterop;
use windows::Win32::System::WinRT::{CreateDispatcherQueueController, DispatcherQueueOptions, DQTAT_COM_NONE, DQTYPE_THREAD_CURRENT};
use windows::UI::Composition::Compositor;

use reui_design::look::{Appearance, Look};
use reui_design::layout::expanded_surface;
use reui_protocol::Rgb;

use super::{d2d, rgb, Device, Interaction, Renderer};

/// Mean (r, g, b) over `x0..x1`, `y0..y1` of a BGRA pixel buffer `w` wide.
fn mean(px: &[u8], w: u32, (x0, y0, x1, y1): (u32, u32, u32, u32)) -> (f32, f32, f32) {
    let (mut r, mut g, mut b, mut n) = (0.0, 0.0, 0.0, 0.0);
    for y in y0..y1 {
        for x in x0..x1 {
            let i = ((y * w + x) * 4) as usize;
            b += px[i] as f32;
            g += px[i + 1] as f32;
            r += px[i + 2] as f32;
            n += 1.0;
        }
    }
    (r / n, g / n, b / n)
}

#[test]
#[ignore = "draws on the GPU: cargo test -p reui-island gpu_and_software_cards_match -- --ignored --nocapture"]
fn gpu_and_software_cards_match() {
    unsafe { _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let _queue = unsafe { CreateDispatcherQueueController(DispatcherQueueOptions { dwSize: size_of::<DispatcherQueueOptions>() as u32, threadType: DQTYPE_THREAD_CURRENT, apartmentType: DQTAT_COM_NONE }) }.unwrap();
    let compositor = Compositor::new().unwrap();
    let device = Device::new(&compositor).unwrap();
    let renderer = Renderer::new(&device, 1.0, Look::new(&Appearance::default(), false, None)).unwrap();
    let view = super::panel_sheet::view_for_probe();
    let accent = rgb(Rgb(10, 132, 255));
    let (w, h) = (expanded_surface(renderer.look()).0 as u32, 260u32);
    let black = d2d(renderer.look().palette.background);
    // A strip of the first card above its content: only the fill and the grain.
    let strip = (40, 66, 120, 74);

    // Software: what the sheets draw.
    let soft = unsafe {
        let (bitmap, rt) = super::panel_sheet::canvas(&device, w, h).unwrap();
        rt.BeginDraw();
        rt.Clear(Some(&black));
        renderer.expanded_into(&rt, Some(&view), accent, &Interaction::default()).unwrap();
        rt.EndDraw(None, None).unwrap();
        let mut px = vec![0u8; (w * h * 4) as usize];
        bitmap.CopyPixels(std::ptr::null(), w * 4, &mut px).unwrap();
        px
    };

    // GPU: the compositor's own D2D device, as the island's surfaces are drawn with.
    let gpu = unsafe {
        let interop: ICompositionGraphicsDeviceInterop = device.graphics.cast().unwrap();
        let d2d_device: ID2D1Device = interop.GetRenderingDevice().unwrap().cast().unwrap();
        let dc = d2d_device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE).unwrap();
        let format = D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED };
        let size = D2D_SIZE_U { width: w, height: h };
        let target = dc.CreateBitmap(size, None, 0, &D2D1_BITMAP_PROPERTIES1 { pixelFormat: format, dpiX: 96.0, dpiY: 96.0, bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET, ..Default::default() }).unwrap();
        dc.SetTarget(&target);
        dc.SetDpi(96.0, 96.0);
        // Twice, as the island redraws: the second must look like the first.
        for _ in 0..2 {
            dc.BeginDraw();
            dc.Clear(Some(&black));
            dc.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let rt: &ID2D1RenderTarget = &dc;
            renderer.expanded_into(rt, Some(&view), accent, &Interaction::default()).unwrap();
            dc.EndDraw(None, None).unwrap();
        }
        let read = dc
            .CreateBitmap(size, None, 0, &D2D1_BITMAP_PROPERTIES1 { pixelFormat: format, dpiX: 96.0, dpiY: 96.0, bitmapOptions: D2D1_BITMAP_OPTIONS_CPU_READ | D2D1_BITMAP_OPTIONS_CANNOT_DRAW, ..Default::default() })
            .unwrap();
        read.CopyFromBitmap(None, &target, None).unwrap();
        let mapped = read.Map(D2D1_MAP_OPTIONS_READ).unwrap();
        let mut px = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            let src = std::slice::from_raw_parts(mapped.bits.add((y * mapped.pitch) as usize), (w * 4) as usize);
            px[(y * w * 4) as usize..((y + 1) * w * 4) as usize].copy_from_slice(src);
        }
        read.Unmap().unwrap();
        px
    };

    let (s, g) = (mean(&soft, w, strip), mean(&gpu, w, strip));
    println!("software card: {s:?}\ngpu card:      {g:?}");
    assert!((s.0 - g.0).abs() < 6.0 && (s.1 - g.1).abs() < 6.0 && (s.2 - g.2).abs() < 6.0, "the GPU draws the card differently: {s:?} vs {g:?}");
}
