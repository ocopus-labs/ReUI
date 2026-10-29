//! The user's own wallpaper, as the island previews' backdrop: the strip across the top
//! centre of the primary monitor, cropped exactly as Windows shows it there (fit mode,
//! background colour), so a preview reads as a screenshot of the top of their screen.
//!
//! Read once on a thread when the window opens (IDesktopWallpaper, else the registry and
//! `SPI_GETDESKWALLPAPER`), decoded and composed with WIC into a small premultiplied BGRA
//! strip, and again when Windows says the wallpaper changed (`WM_SETTINGCHANGE` with
//! `SPI_SETDESKWALLPAPER`). Nothing polls. The image is only read, never copied or sent
//! anywhere. Without an image (a solid colour, a slideshow or Spotlight file that's gone,
//! a decode failure) the backdrop is the wallpaper's solid colour.

pub mod fit;

use std::sync::{Arc, Mutex};

use reui_design::Color;
use windows::core::{Result, HSTRING, PCWSTR};
use windows::Win32::Foundation::{GENERIC_READ, HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTOPRIMARY};
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Shell::{DesktopWallpaper, IDesktopWallpaper};
use windows::Win32::UI::WindowsAndMessaging::*;

use self::fit::{Area, Fit};

/// The window hears that a new backdrop is ready through this message.
pub const WM_WALLPAPER: u32 = WM_APP + 4;

/// How much of the monitor's top centre is kept, in the monitor's DIPs: wider than any
/// header card and taller than any preview's stage.
const STRIP_DIPS: (f32, f32) = (720.0, 400.0);

/// The top of the primary monitor, as the previews show it.
pub struct Backdrop {
    /// The wallpaper's solid colour: under a letterboxed or centred image, or all of it.
    pub color: Color,
    /// The strip, when there's an image.
    pub strip: Option<Strip>,
    /// Monitor pixels per DIP, so the strip is shown 1 DIP to 1 DIP.
    pub scale: f32,
    /// Changes with every read, so the renderer knows to upload the strip again.
    pub generation: u64,
}

/// Premultiplied BGRA pixels, `width` × `height`, the monitor's top centre.
pub struct Strip {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

static CURRENT: Mutex<Option<Arc<Backdrop>>> = Mutex::new(None);
static GENERATION: Mutex<u64> = Mutex::new(0);

/// The backdrop as last read, if it has been.
pub fn current() -> Option<Arc<Backdrop>> {
    CURRENT.lock().ok().and_then(|c| c.clone())
}

/// Reads the wallpaper on a thread, then posts [`WM_WALLPAPER`] to `hwnd`.
pub fn load(hwnd: HWND) {
    let target = hwnd.0 as isize;
    _ = std::thread::Builder::new().name("wallpaper".into()).spawn(move || {
        read_now();
        unsafe { _ = PostMessageW(Some(HWND(target as *mut _)), WM_WALLPAPER, WPARAM(0), LPARAM(0)) };
    });
}

/// Reads the wallpaper on this thread (screenshots, which draw straight away).
pub fn read_now() {
    let generation = GENERATION.lock().map(|mut g| {
        *g += 1;
        *g
    });
    let backdrop = unsafe {
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let backdrop = read(generation.unwrap_or(0));
        if com {
            CoUninitialize();
        }
        backdrop
    };
    if let Ok(mut current) = CURRENT.lock() {
        *current = Some(Arc::new(backdrop));
    }
}

/// A solid colour instead of the wallpaper (screenshots for a public page, which must not
/// show the user's own desktop).
pub fn set_solid(color: Color) {
    let generation = GENERATION.lock().map(|mut g| {
        *g += 1;
        *g
    });
    if let Ok(mut current) = CURRENT.lock() {
        *current = Some(Arc::new(Backdrop { color, strip: None, scale: 1.0, generation: generation.unwrap_or(0) }));
    }
}

/// Whether a `WM_SETTINGCHANGE` is about the wallpaper.
pub fn changed(wparam: WPARAM) -> bool {
    wparam.0 as u32 == SPI_SETDESKWALLPAPER.0
}

/// What Windows says is on the primary monitor.
struct Source {
    path: Option<String>,
    fit: Fit,
    color: Color,
}

unsafe fn read(generation: u64) -> Backdrop {
    let (monitor, scale) = primary();
    let desktop = Area::new(GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN), GetSystemMetrics(SM_CXVIRTUALSCREEN), GetSystemMetrics(SM_CYVIRTUALSCREEN));
    let source = from_shell(monitor).unwrap_or_else(from_registry);
    let strip_area = fit::strip((monitor.w, monitor.h), (STRIP_DIPS.0 * scale).round() as i32, (STRIP_DIPS.1 * scale).round() as i32);
    // The file Windows was given, else its own copy of what it's showing (a slideshow's
    // or Spotlight's image may have moved on); none at all for a solid colour.
    let strip = source.path.as_deref().filter(|p| !p.is_empty()).and_then(|path| {
        let transcoded = std::env::var("APPDATA").ok().map(|d| format!(r"{d}\Microsoft\Windows\Themes\TranscodedWallpaper"));
        [Some(path.to_string()), transcoded].into_iter().flatten().find_map(|p| compose(&p, source.fit, source.color, monitor, desktop, strip_area).ok())
    });
    Backdrop { color: source.color, strip, scale, generation }
}

/// The primary monitor's rectangle (desktop pixels) and its scale.
unsafe fn primary() -> (Area, f32) {
    let handle = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    _ = GetMonitorInfoW(handle, &mut info);
    let (mut dpi, mut _y) = (96u32, 96u32);
    _ = GetDpiForMonitor(handle, MDT_EFFECTIVE_DPI, &mut dpi, &mut _y);
    (area(info.rcMonitor), dpi.max(96) as f32 / 96.0)
}

fn area(r: RECT) -> Area {
    Area::new(r.left, r.top, r.right - r.left, r.bottom - r.top)
}

fn colorref(c: u32) -> Color {
    Color::from_rgb8((c & 0xFF) as u8, (c >> 8 & 0xFF) as u8, (c >> 16 & 0xFF) as u8)
}

/// IDesktopWallpaper: the primary monitor's own image (each monitor can have one), the
/// fit and the background colour.
unsafe fn from_shell(monitor: Area) -> Option<Source> {
    let shell: IDesktopWallpaper = CoCreateInstance(&DesktopWallpaper, None, CLSCTX_ALL).ok()?;
    let fit = Fit::from_position(shell.GetPosition().ok()?.0);
    let color = colorref(shell.GetBackgroundColor().ok()?.0);
    let mut path = None;
    for i in 0..shell.GetMonitorDevicePathCount().unwrap_or(0) {
        let Ok(id) = shell.GetMonitorDevicePathAt(i) else { continue };
        let on_primary = shell.GetMonitorRECT(PCWSTR(id.0)).is_ok_and(|r| area(r) == monitor);
        if on_primary {
            path = shell.GetWallpaper(PCWSTR(id.0)).ok().map(|p| {
                let text = p.to_string().unwrap_or_default();
                CoTaskMemFree(Some(p.0 as *const _));
                text
            });
        }
        CoTaskMemFree(Some(id.0 as *const _));
        if path.is_some() {
            break;
        }
    }
    // Span puts one image across every monitor; ask for the shared one then.
    if path.is_none() {
        path = shell.GetWallpaper(PCWSTR::null()).ok().map(|p| {
            let text = p.to_string().unwrap_or_default();
            CoTaskMemFree(Some(p.0 as *const _));
            text
        });
    }
    Some(Source { path, fit, color })
}

/// The older way: `SPI_GETDESKWALLPAPER`, `Control Panel\Desktop` and `Colors`.
fn from_registry() -> Source {
    let mut buffer = [0u16; 520];
    let path = unsafe { SystemParametersInfoW(SPI_GETDESKWALLPAPER, buffer.len() as u32, Some(buffer.as_mut_ptr().cast()), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)) }
        .ok()
        .map(|()| String::from_utf16_lossy(&buffer[..buffer.iter().position(|&c| c == 0).unwrap_or(0)]));
    let desktop = r"Control Panel\Desktop";
    let fit = Fit::from_registry(&crate::system::user_string(desktop, "WallpaperStyle").unwrap_or_default(), &crate::system::user_string(desktop, "TileWallpaper").unwrap_or_default());
    // "R G B"
    let color = crate::system::user_string(r"Control Panel\Colors", "Background")
        .and_then(|text| {
            let v: Vec<u8> = text.split_whitespace().filter_map(|n| n.parse().ok()).collect();
            (v.len() == 3).then(|| Color::from_rgb8(v[0], v[1], v[2]))
        })
        .unwrap_or(Color::from_rgb8(0, 0, 0));
    Source { path, fit, color }
}

/// The strip: the background colour, then each part of the image that lands in it,
/// scaled by WIC to the size Windows draws it at.
unsafe fn compose(path: &str, fit_mode: Fit, color: Color, monitor: Area, desktop: Area, strip: Area) -> Result<Strip> {
    let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
    let decoder = wic.CreateDecoderFromFilename(&HSTRING::from(path), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand)?;
    let frame = decoder.GetFrame(0)?;
    let converter = wic.CreateFormatConverter()?;
    converter.Initialize(&frame, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)?;
    let (mut iw, mut ih) = (0u32, 0u32);
    converter.GetSize(&mut iw, &mut ih)?;
    let (w, h) = (strip.w as u32, strip.h as u32);
    let stride = w * 4;
    let [r, g, b, _] = color.to_rgba8();
    let mut pixels: Vec<u8> = std::iter::repeat_n([b, g, r, 255], (w * h) as usize).flatten().collect();
    let frames = fit::frames(fit_mode, (iw as i32, ih as i32), monitor, desktop, strip);
    for copy in fit::copies(&frames, strip) {
        let scaler = wic.CreateBitmapScaler()?;
        scaler.Initialize(&converter, copy.frame.w as u32, copy.frame.h as u32, WICBitmapInterpolationModeHighQualityCubic)?;
        let rect = WICRect { X: copy.src.x, Y: copy.src.y, Width: copy.src.w, Height: copy.src.h };
        // Straight into the strip at the copy's place: rows `stride` apart.
        let start = (copy.y as u32 * stride + copy.x as u32 * 4) as usize;
        let end = start + ((copy.src.h as u32 - 1) * stride + copy.src.w as u32 * 4) as usize;
        scaler.CopyPixels(&rect, stride, &mut pixels[start..end])?;
    }
    Ok(Strip { width: w, height: h, pixels })
}
