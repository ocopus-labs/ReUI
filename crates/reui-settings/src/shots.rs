//! `--shots <dir>`: draws every page (and a few states: hover, a menu, keyboard focus,
//! search, scrolled, the welcome) offscreen at 150 % and saves them as PNGs, then exits.
//! The window is never shown. Use with `REUI_CONFIG_DIR` so nothing real is read.
//!
//! `--backdrop-color #RRGGBB` puts that colour on the island stages instead of the user's
//! wallpaper, so the pictures can go on a public page. `--stages` saves only the stages,
//! each one alone at 200 %, as wide as its island and 96 DIPs either side, in the look the settings hold and in its light
//! and Notch variants (`stage-<moment>[-light|-notch].png`): the README's pictures.

use std::path::Path;

use windows::core::{Result, GUID, PCWSTR};
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::*;

use reui_design::look::{Appearance, Look, Style, Theme};
use reui_design::Color;

use crate::app::App;
use crate::pages::Page;
use crate::ui::{stage, Act, Moment, Preview, Target};

const DPI: u32 = 144;
/// The stages alone: twice the pixels, for high-density screens.
const STAGE_DPI: u32 = 192;
/// Backdrop either side of the island on a stage alone, DIPs.
const STAGE_SIDE: f32 = 96.0;
const STAGE_MOMENTS: [Moment; 8] = [Moment::Compact, Moment::Music, Moment::Clipboard, Moment::Meter, Moment::Devices, Moment::Timer, Moment::Stats, Moment::Permission];

/// What the flags beside `--shots` ask for.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Options {
    /// Instead of the wallpaper.
    pub backdrop: Option<Color>,
    /// Only the stages, cropped.
    pub stages: bool,
}

impl Options {
    pub fn from_args(args: &[String]) -> Self {
        let backdrop = args.iter().position(|a| a == "--backdrop-color").and_then(|i| args.get(i + 1)).and_then(|c| hex(c));
        Self { backdrop, stages: args.iter().any(|a| a == "--stages") }
    }
}

/// `#RRGGBB` (the `#` optional).
fn hex(text: &str) -> Option<Color> {
    let digits = text.strip_prefix('#').unwrap_or(text);
    if digits.len() != 6 || !digits.is_ascii() {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).ok();
    Some(Color::from_rgb8(byte(0)?, byte(2)?, byte(4)?))
}

pub fn run(app: &mut App, dir: &Path, options: Options) -> Result<()> {
    _ = std::fs::create_dir_all(dir);
    // The stages show the wallpaper (or the colour asked for); set it before the first
    // page is drawn.
    match options.backdrop {
        Some(color) => crate::wallpaper::set_solid(color),
        None => crate::wallpaper::read_now(),
    }
    if options.stages {
        return stages(app, dir);
    }
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

/// Each moment's stage alone, in the settings' look and its light and Notch variants.
fn stages(app: &mut App, dir: &Path) -> Result<()> {
    app.set_dpi(STAGE_DPI);
    let base = app.state.files.appearance();
    let variants = [("", base), ("-light", Appearance { theme: Theme::Light, ..base }), ("-notch", Appearance { style: Style::Notch, ..base })];
    for (suffix, appearance) in variants {
        let look = Look::new(&appearance, crate::system::apps_light(), crate::system::accent_color());
        for moment in STAGE_MOMENTS {
            // Centred on the island; the compact pill is centred alone, its bubble beside.
            let (w, _) = stage::size(moment, &look);
            let w = if moment == Moment::Compact { 2.0 * w - stage::music_pill().width } else { w };
            let width = w * look.size + 2.0 * STAGE_SIDE;
            let switcher = app.state.files.doc(crate::model::files::File::Island).bool("view_switcher");
            let (w, h, pixels) = app.stage_snapshot(Preview { moment, look, switcher }, width.ceil())?;
            png(&dir.join(format!("stage-{}{suffix}.png", format!("{moment:?}").to_lowercase())), w, h, &pixels)?;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn a_backdrop_colour_is_six_hex_digits() {
        assert_eq!(hex("#4A4D52"), Some(Color::from_rgb8(0x4A, 0x4D, 0x52)));
        assert_eq!(hex("ffffff"), Some(Color::from_rgb8(255, 255, 255)));
        for bad in ["", "#", "#123", "#12345g", "#1234567", "#\u{e9}\u{e9}\u{e9}"] {
            assert_eq!(hex(bad), None, "{bad}");
        }
    }

    #[test]
    fn options_come_from_the_flags_beside_shots() {
        assert_eq!(Options::from_args(&args(&["--shots", "out"])), Options::default());
        let o = Options::from_args(&args(&["--shots", "out", "--backdrop-color", "#202124", "--stages"]));
        assert_eq!(o, Options { backdrop: Some(Color::from_rgb8(0x20, 0x21, 0x24)), stages: true });
        assert_eq!(Options::from_args(&args(&["--backdrop-color", "blue"])).backdrop, None);
    }
}
