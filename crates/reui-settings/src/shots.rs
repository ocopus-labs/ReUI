//! `--shots <dir>`: draws every page (and a few states: hover, a menu, keyboard focus,
//! search, scrolled, the welcome) offscreen at 150 % and saves them as PNGs, then exits.
//! The window is never shown. Use with `REUI_CONFIG_DIR` so nothing real is read.

use std::path::Path;

use windows::core::{Result, GUID, PCWSTR};
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::*;

use crate::app::App;
use crate::pages::Page;
use crate::ui::{Act, Target};

const DPI: u32 = 144;

pub fn run(app: &mut App, dir: &Path) -> Result<()> {
    _ = std::fs::create_dir_all(dir);
    let scale = DPI as f32 / 96.0;
    app.set_dpi(DPI);
    let (w, h) = ((1000.0 * scale) as u32, (700.0 * scale) as u32);
    app.resize(w, h);
    let save = |app: &mut App, name: &str| -> Result<()> {
        let pixels = app.snapshot()?;
        png(&dir.join(format!("{name}.png")), w, h, &pixels)
    };
    for (i, page) in Page::ALL.into_iter().enumerate() {
        app.show(page);
        save(app, &format!("{}-{}", i + 1, format!("{page:?}").to_lowercase()))?;
    }
    // States.
    app.show(Page::General);
    app.rest_pointer(120.0, 220.0);
    save(app, "state-hover-nav")?;
    app.show(Page::NowPlaying);
    app.act_for_shot(&Target::new("retract", 0), Act::Press);
    save(app, "state-menu")?;
    app.show(Page::Clipboard);
    for _ in 0..4 {
        app.key(0x09, false, false);
    }
    save(app, "state-focus")?;
    app.show(Page::Clipboard);
    app.scroll_shot(420.0);
    save(app, "state-scrolled")?;
    app.show(Page::General);
    app.type_search("lyr");
    app.key(0x0D, false, false);
    save(app, "state-search")?;
    app.type_search("");
    for (i, page) in Page::WELCOME.into_iter().enumerate() {
        app.show(page);
        save(app, &format!("welcome-{}", i + 1))?;
    }
    Ok(())
}

/// Writes BGRA pixels (alpha ignored) as a PNG.
fn png(path: &Path, w: u32, h: u32, pixels: &[u8]) -> Result<()> {
    unsafe {
        _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let stream = factory.CreateStream()?;
        let wide: Vec<u16> = path.to_string_lossy().encode_utf16().chain(Some(0)).collect();
        stream.InitializeFromFilename(PCWSTR(wide.as_ptr()), 0x4000_0000)?;
        let encoder = factory.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null::<GUID>())?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let mut frame = None;
        encoder.CreateNewFrame(&mut frame, std::ptr::null_mut())?;
        let frame = frame.expect("a frame");
        frame.Initialize(None)?;
        frame.SetSize(w, h)?;
        let mut format = GUID_WICPixelFormat32bppBGRA;
        frame.SetPixelFormat(&mut format)?;
        // The swap chain ignores alpha; a PNG keeps it, so make every pixel opaque.
        let opaque: Vec<u8> = pixels.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2], 255]).collect();
        frame.WritePixels(h, w * 4, &opaque)?;
        frame.Commit()?;
        encoder.Commit()?;
    }
    Ok(())
}
