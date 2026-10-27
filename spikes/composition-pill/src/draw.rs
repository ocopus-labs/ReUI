//! Pill content drawn with Direct2D/DirectWrite into composition surfaces.
//! Units: all layout constants are DIPs; `scale` converts them to pixels.

use windows::core::{w, Interface, Result, PCWSTR};
use windows::Foundation::Size;
use windows::Graphics::DirectX::{DirectXAlphaMode, DirectXPixelFormat};
use windows::UI::Composition::{CompositionDrawingSurface, CompositionGraphicsDevice, Compositor};
use windows::Win32::Foundation::{HMODULE, POINT};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::WinRT::Composition::{ICompositionDrawingSurfaceInterop, ICompositorInterop};
use windows_numerics::{Matrix3x2, Vector2};

pub const COMPACT: (f32, f32) = (220.0, 36.0);
pub const PEEK: (f32, f32) = (232.0, 40.0);
pub const EXPANDED: (f32, f32) = (380.0, 156.0);

// Compact bars (live visuals, not drawn): right-aligned, vertically centred.
pub const BAR_W: f32 = 3.0;
pub const BAR_GAP: f32 = 2.5;
pub const BAR_H: f32 = 14.0;
pub const BAR_COUNT: usize = 4;
pub const BAR_RIGHT_PAD: f32 = 14.0;

// Expanded transport row.
const BUTTON_Y: (f32, f32) = (120.0, 152.0);
const BUTTON_HALF_W: f32 = 22.0;
const BUTTON_SPACING: f32 = 64.0;

pub struct Track {
    pub title: &'static str,
    pub artist: &'static str,
    pub from: (f32, f32, f32),
    pub to: (f32, f32, f32),
}

pub const TRACKS: [Track; 3] = [
    Track { title: "Midnight City", artist: "M83", from: (0.96, 0.35, 0.55), to: (0.40, 0.20, 0.85) },
    Track { title: "Nightcall", artist: "Kavinsky", from: (0.98, 0.62, 0.20), to: (0.85, 0.18, 0.30) },
    Track { title: "Intro (a much longer title to test trimming)", artist: "The xx", from: (0.20, 0.75, 0.70), to: (0.15, 0.30, 0.80) },
];

#[derive(Clone, Copy, Debug)]
pub enum Button {
    Prev,
    Play,
    Next,
}

/// Hit-test the expanded card. `x`, `y` are DIPs relative to the expanded content.
pub fn button_at(x: f32, y: f32) -> Option<Button> {
    if y < BUTTON_Y.0 || y > BUTTON_Y.1 {
        return None;
    }
    let mid = EXPANDED.0 / 2.0;
    [(mid - BUTTON_SPACING, Button::Prev), (mid, Button::Play), (mid + BUTTON_SPACING, Button::Next)]
        .into_iter()
        .find(|(cx, _)| (x - cx).abs() <= BUTTON_HALF_W)
        .map(|(_, b)| b)
}

struct Fonts {
    compact_title: IDWriteTextFormat,
    title: IDWriteTextFormat,
    artist: IDWriteTextFormat,
    time: IDWriteTextFormat,
    time_right: IDWriteTextFormat,
    icon: IDWriteTextFormat,
    icon_big: IDWriteTextFormat,
}

pub struct Canvas {
    pub graphics: CompositionGraphicsDevice,
    scale: f32,
    fonts: Fonts,
}

impl Canvas {
    pub fn new(compositor: &Compositor, scale: f32) -> Result<Self> {
        let d3d = create_d3d_device()?;
        let dxgi: IDXGIDevice = d3d.cast()?;
        let factory: ID2D1Factory1 = unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        let device = unsafe { factory.CreateDevice(&dxgi)? };
        let graphics = unsafe { compositor.cast::<ICompositorInterop>()?.CreateGraphicsDevice(&device)? };
        let dwrite: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };

        let text = w!("Segoe UI Variable Text");
        let display = w!("Segoe UI Variable Display");
        let icons = w!("Segoe Fluent Icons");
        let fonts = Fonts {
            compact_title: format(&dwrite, text, DWRITE_FONT_WEIGHT_SEMI_BOLD, 13.0 * scale, true, false, true)?,
            title: format(&dwrite, display, DWRITE_FONT_WEIGHT_SEMI_BOLD, 17.0 * scale, true, false, false)?,
            artist: format(&dwrite, text, DWRITE_FONT_WEIGHT_NORMAL, 14.0 * scale, true, false, false)?,
            time: format(&dwrite, text, DWRITE_FONT_WEIGHT_NORMAL, 11.0 * scale, false, false, false)?,
            time_right: format(&dwrite, text, DWRITE_FONT_WEIGHT_NORMAL, 11.0 * scale, false, true, false)?,
            icon: format(&dwrite, icons, DWRITE_FONT_WEIGHT_NORMAL, 20.0 * scale, false, false, true)?,
            icon_big: format(&dwrite, icons, DWRITE_FONT_WEIGHT_NORMAL, 26.0 * scale, false, false, true)?,
        };
        for f in [&fonts.icon, &fonts.icon_big] {
            unsafe { f.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)? };
        }
        Ok(Self { graphics, scale, fonts })
    }

    pub fn surface(&self, dips: (f32, f32)) -> Result<CompositionDrawingSurface> {
        let size = Size { Width: (dips.0 * self.scale).ceil(), Height: (dips.1 * self.scale).ceil() };
        self.graphics
            .CreateDrawingSurface(size, DirectXPixelFormat::B8G8R8A8UIntNormalized, DirectXAlphaMode::Premultiplied)
    }

    pub fn draw_compact(&self, surface: &CompositionDrawingSurface, track: &Track) -> Result<()> {
        let s = self.scale;
        draw(surface, |rt| unsafe {
            artwork(rt, track, rect(6.0, 6.0, 30.0, 30.0, s), 7.0 * s, None)?;
            let white = brush(rt, 1.0, 1.0, 1.0, 1.0)?;
            let text_right = COMPACT.0 - BAR_RIGHT_PAD - bars_width() - 10.0;
            text(rt, track.title, &self.fonts.compact_title, rect(40.0, 0.0, text_right, COMPACT.1, s), &white);
            Ok(())
        })
    }

    pub fn draw_expanded(&self, surface: &CompositionDrawingSurface, track: &Track, playing: bool) -> Result<()> {
        let s = self.scale;
        draw(surface, |rt| unsafe {
            artwork(rt, track, rect(20.0, 20.0, 84.0, 84.0, s), 14.0 * s, Some(&self.fonts.icon_big))?;

            let white = brush(rt, 1.0, 1.0, 1.0, 1.0)?;
            let dim = brush(rt, 1.0, 1.0, 1.0, 0.58)?;
            let track_bg = brush(rt, 1.0, 1.0, 1.0, 0.22)?;
            text(rt, track.title, &self.fonts.title, rect(100.0, 26.0, 360.0, 50.0, s), &white);
            text(rt, track.artist, &self.fonts.artist, rect(100.0, 52.0, 360.0, 72.0, s), &dim);

            let progress = 0.38;
            rounded(rt, rect(20.0, 100.0, 360.0, 104.0, s), 2.0 * s, &track_bg);
            rounded(rt, rect(20.0, 100.0, 20.0 + 340.0 * progress, 104.0, s), 2.0 * s, &white);
            text(rt, "1:24", &self.fonts.time, rect(20.0, 108.0, 100.0, 122.0, s), &dim);
            text(rt, "-2:05", &self.fonts.time_right, rect(280.0, 108.0, 360.0, 122.0, s), &dim);

            let mid = EXPANDED.0 / 2.0;
            let row = |cx: f32| rect(cx - BUTTON_HALF_W, BUTTON_Y.0, cx + BUTTON_HALF_W, BUTTON_Y.1, s);
            text(rt, "\u{E892}", &self.fonts.icon, row(mid - BUTTON_SPACING), &white);
            text(rt, if playing { "\u{E769}" } else { "\u{E768}" }, &self.fonts.icon_big, row(mid), &white);
            text(rt, "\u{E893}", &self.fonts.icon, row(mid + BUTTON_SPACING), &white);
            Ok(())
        })
    }
}

pub fn bars_width() -> f32 {
    BAR_COUNT as f32 * BAR_W + (BAR_COUNT - 1) as f32 * BAR_GAP
}

fn create_d3d_device() -> Result<ID3D11Device> {
    let mut result = Err(windows::core::Error::empty());
    for driver in [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP] {
        let mut device = None;
        result = unsafe {
            D3D11CreateDevice(
                None,
                driver,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                None,
            )
        }
        .map(|_| device.unwrap());
        if result.is_ok() {
            break;
        }
    }
    result
}

fn format(
    dwrite: &IDWriteFactory,
    family: PCWSTR,
    weight: DWRITE_FONT_WEIGHT,
    size: f32,
    trim: bool,
    right: bool,
    v_center: bool,
) -> Result<IDWriteTextFormat> {
    unsafe {
        let f = dwrite.CreateTextFormat(family, None, weight, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size, w!("en-us"))?;
        f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        if trim {
            let sign = dwrite.CreateEllipsisTrimmingSign(&f)?;
            let options = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
            f.SetTrimming(&options, &sign)?;
        }
        if right {
            f.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_TRAILING)?;
        }
        if v_center {
            f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        }
        Ok(f)
    }
}

/// Begin/end a draw on a composition surface; the closure gets a render target in pixels.
fn draw(surface: &CompositionDrawingSurface, f: impl FnOnce(&ID2D1RenderTarget) -> Result<()>) -> Result<()> {
    let interop: ICompositionDrawingSurfaceInterop = surface.cast()?;
    let mut offset = POINT::default();
    let dc: ID2D1DeviceContext = unsafe { interop.BeginDraw(None, &mut offset)? };
    let rt: &ID2D1RenderTarget = &dc;
    let result = unsafe {
        rt.SetDpi(96.0, 96.0);
        rt.SetTransform(&Matrix3x2::translation(offset.x as f32, offset.y as f32));
        rt.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
        rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        f(rt)
    };
    unsafe { interop.EndDraw()? };
    result
}

fn rect(l: f32, t: f32, r: f32, b: f32, s: f32) -> D2D_RECT_F {
    D2D_RECT_F { left: l * s, top: t * s, right: r * s, bottom: b * s }
}

unsafe fn brush(rt: &ID2D1RenderTarget, r: f32, g: f32, b: f32, a: f32) -> Result<ID2D1SolidColorBrush> {
    rt.CreateSolidColorBrush(&D2D1_COLOR_F { r, g, b, a }, None)
}

unsafe fn rounded(rt: &ID2D1RenderTarget, r: D2D_RECT_F, radius: f32, b: &ID2D1SolidColorBrush) {
    rt.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: r, radiusX: radius, radiusY: radius }, b);
}

unsafe fn text(rt: &ID2D1RenderTarget, s: &str, format: &IDWriteTextFormat, r: D2D_RECT_F, b: &ID2D1SolidColorBrush) {
    let wide: Vec<u16> = s.encode_utf16().collect();
    rt.DrawText(&wide, format, &r, b, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT, DWRITE_MEASURING_MODE_NATURAL);
}

/// Placeholder album art: a diagonal gradient, optionally with a music-note glyph.
unsafe fn artwork(rt: &ID2D1RenderTarget, track: &Track, r: D2D_RECT_F, radius: f32, glyph: Option<&IDWriteTextFormat>) -> Result<()> {
    let color = |(r, g, b): (f32, f32, f32)| D2D1_COLOR_F { r, g, b, a: 1.0 };
    let stops = [
        D2D1_GRADIENT_STOP { position: 0.0, color: color(track.from) },
        D2D1_GRADIENT_STOP { position: 1.0, color: color(track.to) },
    ];
    let collection = rt.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)?;
    let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
        startPoint: Vector2::new(r.left, r.top),
        endPoint: Vector2::new(r.right, r.bottom),
    };
    let gradient = rt.CreateLinearGradientBrush(&props, None, &collection)?;
    rt.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: r, radiusX: radius, radiusY: radius }, &gradient);
    if let Some(format) = glyph {
        let note = brush(rt, 1.0, 1.0, 1.0, 0.55)?;
        let wide: Vec<u16> = "\u{EC4F}".encode_utf16().collect();
        format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        rt.DrawText(&wide, format, &r, &note, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
    }
    Ok(())
}
