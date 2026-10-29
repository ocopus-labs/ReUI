//! Test-only: the device art contact sheet. Draws every kind in `design/assets/devices/`
//! the way the island would show it through `Art::Logo` (the same `draw_svg` tint and
//! pixel snapping, the same token sizes): the small art in a compact pill's 24 spot and the
//! detailed art in an open card's 56 spot, next to the Segoe Fluent glyph the device shows
//! today, on the dark and light themes at 100 % and 200 %. A second sheet magnifies the
//! art at its exact pixel sizes so the grid can be judged pixel by pixel.
//!
//! Offscreen (a WIC bitmap), so it needs no window, compositor or hub:
//! `cargo test -p reui-island device_art_sheet -- --ignored`, PNGs in `target/devices-art/`.

use std::path::{Path, PathBuf};

use reui_design::tokens::{self, component, icon, island, opacity, typography, Palette};
use reui_design::Color;
use windows::core::{w, Result, GUID, PCWSTR};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::*;
use windows_numerics::Matrix3x2;

use super::images::{draw_svg, mark_size};
use super::d2d;

/// An art id, its hero name on the card, and the Segoe Fluent glyph the device shows today.
type Entry = (&'static str, &'static str, char);

/// Every kind: what the island shows by default.
const KINDS: &[Entry] = &[
    ("earbuds", "Wireless Earbuds", '\u{F4C0}'),
    ("headphones", "Headphones", '\u{E7F6}'),
    ("speaker", "Speaker", '\u{E7F5}'),
    ("gamepad", "Controller", '\u{E7FC}'),
    ("mouse", "Mouse", '\u{E962}'),
    ("keyboard", "Keyboard", '\u{E92E}'),
    ("pen", "Pen", '\u{EDC6}'),
    ("watch", "Watch", '\u{E916}'),
    ("phone", "Phone", '\u{E8EA}'),
    ("bluetooth", "Bluetooth device", '\u{E702}'),
];

const COLUMNS: usize = 5;
/// Arts across the magnified sheet.
const PIXEL_COLUMNS: usize = 2;
/// One kind's cell: a compact pill over an open card's header, DIPs.
const CELL: (f32, f32) = (320.0, 168.0);
const MARGIN: f32 = 24.0;
const CARD: (f32, f32) = (296.0, 96.0);
const PILL_WIDTH: f32 = 128.0;
/// A wallpaper-ish grey, so both the black and the white island keep their edge.
const BACKDROP: Color = Color { r: 0.42, g: 0.43, b: 0.46, a: 1.0 };
/// The magnified sheet: each pixel this many pixels wide.
const ZOOM: f32 = 6.0;

fn art_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../design/assets/devices")
}

fn out_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/devices-art")
}

fn art(family: &str, small: bool) -> String {
    let name = if small { format!("{family}-small.svg") } else { format!("{family}.svg") };
    let path = art_dir().join(&name);
    super::images::fit_svg(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
}

struct Sheet {
    factory: ID2D1Factory1,
    dwrite: IDWriteFactory,
    wic: IWICImagingFactory,
}

impl Sheet {
    fn new() -> Result<Self> {
        unsafe {
            _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            Ok(Self {
                factory: D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?,
                dwrite: DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?,
                wic: CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?,
            })
        }
    }

    /// A `w` × `h` pixel canvas and a render target drawing into it at 96 DPI (pixels).
    fn canvas(&self, w: u32, h: u32) -> Result<(IWICBitmap, ID2D1RenderTarget)> {
        unsafe {
            let bitmap = self.wic.CreateBitmap(w, h, &GUID_WICPixelFormat32bppPBGRA, WICBitmapCacheOnLoad)?;
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                pixelFormat: D2D1_PIXEL_FORMAT { format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                dpiX: 96.0,
                dpiY: 96.0,
                ..Default::default()
            };
            let rt = self.factory.CreateWicBitmapRenderTarget(&bitmap, &props)?;
            rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            Ok((bitmap, rt))
        }
    }

    fn save(&self, bitmap: &IWICBitmap, path: &Path) -> Result<()> {
        unsafe {
            let stream = self.wic.CreateStream()?;
            let wide: Vec<u16> = path.to_string_lossy().encode_utf16().chain(Some(0)).collect();
            stream.InitializeFromFilename(PCWSTR(wide.as_ptr()), 0x4000_0000)?;
            let encoder = self.wic.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null::<GUID>())?;
            encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
            let mut frame = None;
            encoder.CreateNewFrame(&mut frame, std::ptr::null_mut())?;
            let frame = frame.expect("a frame");
            frame.Initialize(None)?;
            frame.WriteSource(bitmap, std::ptr::null())?;
            frame.Commit()?;
            encoder.Commit()?;
        }
        Ok(())
    }

    fn format(&self, family: PCWSTR, size: f32, weight: DWRITE_FONT_WEIGHT) -> Result<IDWriteTextFormat> {
        unsafe { self.dwrite.CreateTextFormat(family, None, weight, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size, w!("en-us")) }
    }
}

#[allow(clippy::too_many_arguments)]
fn text(rt: &ID2D1RenderTarget, format: &IDWriteTextFormat, s: &str, x: f32, y: f32, w: f32, h: f32, color: Color) -> Result<()> {
    let wide: Vec<u16> = s.encode_utf16().collect();
    unsafe {
        let brush = rt.CreateSolidColorBrush(&d2d(color), None)?;
        rt.DrawText(&wide, format, &D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h }, &brush, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
    }
    Ok(())
}

fn fill(rt: &ID2D1RenderTarget, l: f32, t: f32, w: f32, h: f32, radius: f32, color: Color) -> Result<()> {
    unsafe {
        let brush = rt.CreateSolidColorBrush(&d2d(color), None)?;
        rt.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: D2D_RECT_F { left: l, top: t, right: l + w, bottom: t + h }, radiusX: radius, radiusY: radius }, &brush);
    }
    Ok(())
}

/// A bare mark in a `spot` DIP square at (`x`, `y`) DIPs, as `Renderer::art` draws an
/// `Art::Logo`: `opacity.logo-bare` of the spot, snapped to whole pixels.
fn mark(rt: &ID2D1RenderTarget, svg: &str, x: f32, y: f32, spot: f32, s: f32, color: Color) -> Result<()> {
    let m = mark_size(spot, opacity::LOGO_BARE);
    draw_svg(rt, svg, ((x + (spot - m) / 2.0) * s).round(), ((y + (spot - m) / 2.0) * s).round(), (m * s).round(), color)
}

fn contact(sheet: &Sheet, set: &[Entry], pal: &Palette, s: f32, path: &Path) -> Result<()> {
    let rows = set.len().div_ceil(COLUMNS);
    let (w, h) = (MARGIN * 2.0 + CELL.0 * COLUMNS as f32, MARGIN * 2.0 + CELL.1 * rows as f32);
    let (bitmap, rt) = sheet.canvas((w * s) as u32, (h * s) as u32)?;
    let accent = tokens::color::accent::DEFAULT;
    let px = |v: f32| (v * s).round();
    let display = |size: f32, weight| sheet.format(w!("Segoe UI Variable Display"), size * s, weight);
    let body = |size: f32, weight| sheet.format(w!("Segoe UI Variable Text"), size * s, weight);
    let eyebrow = body(typography::EYEBROW.size, DWRITE_FONT_WEIGHT_SEMI_BOLD)?;
    let hero = display(typography::HEADLINE.size, DWRITE_FONT_WEIGHT_SEMI_BOLD)?;
    let detail = body(typography::DETAIL.size, DWRITE_FONT_WEIGHT_NORMAL)?;
    let compact = body(typography::COMPACT.size, DWRITE_FONT_WEIGHT_SEMI_BOLD)?;
    let caption = body(typography::CAPTION.size, DWRITE_FONT_WEIGHT_NORMAL)?;
    let glyph = sheet.format(w!("Segoe Fluent Icons"), icon::TRAILING * s, DWRITE_FONT_WEIGHT_NORMAL)?;
    let glyph_compact = sheet.format(w!("Segoe Fluent Icons"), icon::COMPACT * s, DWRITE_FONT_WEIGHT_NORMAL)?;
    unsafe {
        rt.BeginDraw();
        rt.Clear(Some(&d2d(BACKDROP)));
        rt.SetTransform(&Matrix3x2::identity());
    }
    for (i, (id, name, kind)) in set.iter().enumerate() {
        let (cx, cy) = (MARGIN + CELL.0 * (i % COLUMNS) as f32, MARGIN + CELL.1 * (i / COLUMNS) as f32);
        // Compact: the small art in the leading 24 spot, the battery at the trailing end,
        // and today's Fluent glyph bare beside it, for weight.
        let height = island::HEIGHT;
        fill(&rt, px(cx), px(cy), px(PILL_WIDTH), px(height), height / 2.0 * s, pal.background)?;
        let margin = island::compact::SIDE_MARGIN;
        mark(&rt, &art(id, true), cx + margin, cy + margin, component::art::COMPACT, s, pal.label_primary)?;
        let line = typography::COMPACT.line_height;
        text(&rt, &compact, "82%", px(cx + PILL_WIDTH - island::compact::TEXT_MARGIN - 40.0), px(cy + (height - line) / 2.0), 40.0 * s, line * s, pal.label_primary)?;
        let g = icon::COMPACT;
        text(&rt, &glyph_compact, &kind.to_string(), px(cx + PILL_WIDTH + 12.0), px(cy + (height - g) / 2.0), g * 2.0 * s, g * 1.5 * s, BACKDROP_INK)?;
        text(&rt, &caption, id, px(cx + PILL_WIDTH + 40.0), px(cy + (height - 16.0) / 2.0), 140.0 * s, 20.0 * s, BACKDROP_INK)?;
        // Open card: the detailed art in the leading 56 spot, eyebrow / hero / detail, and
        // the Fluent glyph at the trailing size.
        let top = cy + height + 12.0;
        let radius = island::expanded::RADIUS;
        fill(&rt, px(cx), px(top), px(CARD.0), px(CARD.1), radius * s, pal.background)?;
        let inset = island::expanded::INSET;
        let spot = component::art::EXPANDED;
        mark(&rt, &art(id, false), cx + inset, top + inset, spot, s, pal.label_primary)?;
        let tx = cx + inset + spot + island::expanded::COLUMN_GAP;
        let ty = top + inset + (spot - 56.0) / 2.0;
        text(&rt, &eyebrow, "Connected", px(tx), px(ty), 160.0 * s, 16.0 * s, accent)?;
        text(&rt, &hero, name, px(tx), px(ty + 16.0), 190.0 * s, 24.0 * s, pal.label_primary)?;
        text(&rt, &detail, "82%", px(tx), px(ty + 40.0), 160.0 * s, 20.0 * s, pal.label_secondary)?;
        let g = icon::TRAILING;
        text(&rt, &glyph, &kind.to_string(), px(cx + CARD.0 - inset - g), px(top + inset), g * 1.5 * s, g * 1.5 * s, pal.label_tertiary)?;
    }
    unsafe { rt.EndDraw(None, None)? };
    sheet.save(&bitmap, path)
}

/// Labels on the backdrop.
const BACKDROP_INK: Color = Color { r: 0.92, g: 0.92, b: 0.94, a: 1.0 };

/// Every art at its exact pixel size at 100 % (21 and 48 px), rendered as the island would,
/// then blown up `ZOOM` times with nearest-neighbour so each pixel shows; beside each, the
/// Fluent glyph at the same em size, for stroke weight and optical size.
fn pixels(sheet: &Sheet, set: &[Entry], pal: &Palette, path: &Path) -> Result<()> {
    let small = mark_size(component::art::COMPACT, opacity::LOGO_BARE);
    let big = mark_size(component::art::EXPANDED, opacity::LOGO_BARE);
    let gap = 4.0 * ZOOM;
    let cell = (small + big) * 2.0 * ZOOM + gap * 6.0;
    let row = big * ZOOM + gap;
    let rows = set.len().div_ceil(PIXEL_COLUMNS);
    let (bitmap, rt) = sheet.canvas((cell * PIXEL_COLUMNS as f32 + 32.0) as u32, (row * rows as f32 + 32.0) as u32)?;
    unsafe {
        rt.BeginDraw();
        rt.Clear(Some(&d2d(BACKDROP)));
    }
    for (i, (id, _, glyph)) in set.iter().enumerate() {
        let (mut x, y) = (16.0 + cell * (i % PIXEL_COLUMNS) as f32, 16.0 + row * (i / PIXEL_COLUMNS) as f32);
        for (small_art, size) in [(true, small), (false, big)] {
            for fluent in [false, true] {
                let (tile, trt) = sheet.canvas(size as u32, size as u32)?;
                unsafe {
                    trt.BeginDraw();
                    trt.Clear(Some(&d2d(pal.background)));
                    if fluent {
                        let format = sheet.format(w!("Segoe Fluent Icons"), size, DWRITE_FONT_WEIGHT_NORMAL)?;
                        format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
                        format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
                        text(&trt, &format, &glyph.to_string(), 0.0, 0.0, size, size, pal.label_primary)?;
                    } else {
                        draw_svg(&trt, &art(id, small_art), 0.0, 0.0, size, pal.label_primary)?;
                    }
                    trt.EndDraw(None, None)?;
                    let b = rt.CreateBitmapFromWicBitmap(&tile, None)?;
                    let top = y + (big - size) * ZOOM / 2.0;
                    let r = D2D_RECT_F { left: x, top, right: x + size * ZOOM, bottom: top + size * ZOOM };
                    rt.DrawBitmap(&b, Some(&r), 1.0, D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR, None);
                }
                x += size * ZOOM + if fluent { gap * 2.0 } else { gap };
            }
        }
    }
    unsafe { rt.EndDraw(None, None)? };
    sheet.save(&bitmap, path)
}

#[test]
#[ignore = "writes PNGs for review: cargo test -p reui-island device_art_sheet -- --ignored"]
fn device_art_sheet() -> Result<()> {
    let sheet = Sheet::new()?;
    let dir = out_dir();
    std::fs::create_dir_all(&dir).expect("target/devices-art");
    for (name, set) in [("kinds", KINDS)] {
        for (theme, pal) in [("dark", tokens::palette::DARK), ("light", tokens::palette::LIGHT)] {
            for scale in [1.0, 2.0] {
                contact(&sheet, set, &pal, scale, &dir.join(format!("{name}-{theme}-{}.png", (scale * 100.0) as u32)))?;
            }
            pixels(&sheet, set, &pal, &dir.join(format!("{name}-pixels-{theme}.png")))?;
        }
    }
    Ok(())
}
