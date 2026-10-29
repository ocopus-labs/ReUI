//! Drawing, with Direct2D and DirectWrite on the GPU ([`crate::gpu`]), every colour, size and
//! face from the tokens. The device context works in DIPs (its DPI is the window's), so
//! layout numbers are used as they are and snapped to pixels where edges must stay crisp.
//! Here: the [`Renderer`] (device, caches) and the [`Painter`] primitives; what gets
//! drawn lives in the child modules.
//!
//! Everything that costs something to make is kept: text layouts ([`text`]), one solid
//! brush whose colour changes, gradient brushes per colour pair, corner geometries per
//! size, and the SVG mark per size.

mod controls;
mod fonts;
pub mod frame;
pub mod icons;
pub mod menu;
mod page;
pub mod preview;
mod shapes;
mod sidebar;
mod text;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use reui_design::Color;
use windows::core::{Interface, Result};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::{DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET};
use windows_numerics::{Matrix3x2, Vector2};

pub use self::controls::part_of;
pub use self::frame::View;
use self::text::{Face, Kind, TextCache};
use crate::gpu::{Gpu, D2DERR_RECREATE_TARGET};
use crate::ui::layout::{Font, Measure, Rect};

pub(super) const CLAUDE_SVG: &str = include_str!("../../../../design/assets/agents/claude.svg");

pub struct Renderer {
    gpu: Gpu,
    texts: RefCell<TextCache>,
    device: Device,
    dpi: f32,
}

/// An SVG document's cache key: its markup (by address: the marks are `'static`), its
/// size's bits and its ink (none: its own colours).
type SvgKey = (usize, u32, Option<[u8; 4]>);

/// Resources tied to the GPU device: made again when it's lost.
#[derive(Default)]
struct Device {
    brush: Option<ID2D1SolidColorBrush>,
    linear: RefCell<HashMap<[u8; 8], ID2D1LinearGradientBrush>>,
    /// Top-rounded shapes (island stages) by size.
    shapes: RefCell<HashMap<[u32; 3], ID2D1PathGeometry>>,
    /// The page and the sidebar as last drawn ([`Slot`]), and their sizes in pixels.
    cache: RefCell<[Option<(ID2D1Bitmap1, u32, u32)>; 2]>,
    svgs: RefCell<HashMap<SvgKey, ID2D1SvgDocument>>,
    /// The wallpaper strip ([`crate::wallpaper`]) as uploaded, and which read it is.
    wallpaper: RefCell<Option<(u64, ID2D1BitmapBrush1)>>,
}

/// What a cached bitmap holds.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Page = 0,
    Sidebar = 1,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Start,
    Center,
    End,
}

impl Renderer {
    pub fn new(dpi: f32) -> Result<Self> {
        Ok(Self { gpu: Gpu::new()?, texts: RefCell::new(TextCache::new()?), device: Device::default(), dpi })
    }

    pub fn set_dpi(&mut self, dpi: f32) {
        self.dpi = dpi;
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if self.gpu.resize(width, height).is_err() {
            self.lost();
        }
    }

    /// Signalled when the window's swap chain can take another frame.
    pub fn waitable(&self) -> Option<windows::Win32::Foundation::HANDLE> {
        self.gpu.waitable()
    }

    /// The GPU went away (driver update, sleep): start again on a new device.
    fn lost(&mut self) {
        if let Ok(gpu) = Gpu::new() {
            self.gpu = gpu;
        }
        self.device = Device::default();
    }

    /// Draws a frame into the window and presents it.
    pub fn draw(&mut self, hwnd: HWND, width: u32, height: u32, background: Color, f: impl FnOnce(&Painter) -> Result<()>) -> Result<()> {
        self.gpu.attach(hwnd, width, height)?;
        if let Err(e) = self.gpu.begin(self.dpi) {
            self.lost();
            return Err(e);
        }
        let drawn = self.frame(background, f);
        let at = std::time::Instant::now();
        let presented = drawn.and_then(|()| self.gpu.present());
        crate::perf::part("present", at.elapsed().as_secs_f64() * 1000.0);
        if let Err(e) = &presented {
            if [D2DERR_RECREATE_TARGET, DXGI_ERROR_DEVICE_REMOVED.0, DXGI_ERROR_DEVICE_RESET.0].contains(&e.code().0) {
                self.lost();
            }
        }
        self.texts.borrow_mut().end_frame();
        presented
    }

    /// Draws a frame offscreen, `width` × `height` pixels, and returns its BGRA pixels.
    pub fn draw_offscreen(&mut self, width: u32, height: u32, background: Color, f: impl FnOnce(&Painter) -> Result<()>) -> Result<Vec<u8>> {
        let target = self.gpu.offscreen(width, height, self.dpi)?;
        let image: ID2D1Image = target.cast()?;
        unsafe {
            self.gpu.dc.SetTarget(&image);
            self.gpu.dc.SetDpi(self.dpi, self.dpi);
        }
        self.frame(background, f)?;
        self.texts.borrow_mut().end_frame();
        let pixels = self.gpu.read(&target, width, height);
        unsafe { self.gpu.dc.SetTarget(None) };
        pixels
    }

    fn frame(&mut self, background: Color, f: impl FnOnce(&Painter) -> Result<()>) -> Result<()> {
        let dc = self.gpu.dc.clone();
        if self.device.brush.is_none() {
            self.device.brush = Some(unsafe { dc.CreateSolidColorBrush(&d2d(background), None)? });
        }
        let painter = Painter { r: self, dc: &dc, rt: dc.cast()?, brush: self.device.brush.as_ref().expect("brush"), scale: self.dpi / 96.0, alpha: Cell::new(1.0) };
        unsafe {
            dc.BeginDraw();
            dc.SetTransform(&Matrix3x2::identity());
            dc.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            dc.Clear(Some(&d2d(background)));
        }
        let at = std::time::Instant::now();
        let drawn = f(&painter);
        crate::perf::part("record", at.elapsed().as_secs_f64() * 1000.0);
        let at = std::time::Instant::now();
        let ended = unsafe { dc.EndDraw(None, None) };
        crate::perf::part("end-draw", at.elapsed().as_secs_f64() * 1000.0);
        drawn.and(ended)
    }
}

impl Measure for Renderer {
    fn width(&self, text: &str, font: Font) -> f32 {
        self.texts.borrow_mut().width(text, font)
    }

    fn height(&self, text: &str, font: Font, max_width: f32) -> f32 {
        self.texts.borrow_mut().height(text, font, max_width)
    }
}

/// One frame's drawing calls, in DIPs.
pub struct Painter<'a> {
    r: &'a Renderer,
    dc: &'a ID2D1DeviceContext5,
    /// The same context as a plain render target (some calls are overloaded further down).
    rt: ID2D1RenderTarget,
    brush: &'a ID2D1SolidColorBrush,
    scale: f32,
    /// Multiplies every colour's alpha (disabled rows draw quieter).
    pub alpha: Cell<f32>,
}

impl Painter<'_> {
    /// `v` on the pixel grid.
    pub fn snap(&self, v: f32) -> f32 {
        (v * self.scale).round() / self.scale
    }

    /// One physical pixel, in DIPs.
    pub fn pixel(&self) -> f32 {
        1.0 / self.scale
    }

    fn brush(&self, color: Color) -> &ID2D1SolidColorBrush {
        let c = color.with_alpha(color.a * self.alpha.get());
        unsafe { self.brush.SetColor(&d2d(c)) };
        self.brush
    }

    pub fn fill(&self, r: Rect, radius: f32, color: Color) -> Result<()> {
        unsafe {
            if radius <= 0.0 {
                self.dc.FillRectangle(&rect(r), self.brush(color));
            } else {
                self.dc.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: rect(r), radiusX: radius, radiusY: radius }, self.brush(color));
            }
        }
        Ok(())
    }

    /// A ring along a rounded rectangle, centred on its edge.
    pub fn stroke(&self, r: Rect, radius: f32, width: f32, color: Color) -> Result<()> {
        let shape = D2D1_ROUNDED_RECT { rect: rect(r), radiusX: radius, radiusY: radius };
        unsafe { self.dc.DrawRoundedRectangle(&shape, self.brush(color), width, None) };
        Ok(())
    }

    /// A hairline just inside the edge of a rounded rectangle (containers keep their edge).
    pub fn edge(&self, r: Rect, radius: f32, color: Color) -> Result<()> {
        let half = self.pixel() / 2.0;
        self.stroke(r.inflate(-half), radius - half, self.pixel(), color)
    }

    /// A horizontal hairline (one physical pixel) from `l` to `r` at `y`.
    pub fn hairline(&self, l: f32, r: f32, y: f32, color: Color) -> Result<()> {
        let y = self.snap(y);
        self.fill(Rect::new(l, y, r, y + self.pixel()), 0.0, color)
    }

    pub fn circle(&self, cx: f32, cy: f32, radius: f32, color: Color) -> Result<()> {
        let e = D2D1_ELLIPSE { point: Vector2::new(cx, cy), radiusX: radius, radiusY: radius };
        unsafe { self.dc.FillEllipse(&e, self.brush(color)) };
        Ok(())
    }

    fn stops(&self, a: Color, b: Color) -> Result<ID2D1GradientStopCollection> {
        let list = [D2D1_GRADIENT_STOP { position: 0.0, color: d2d(a) }, D2D1_GRADIENT_STOP { position: 1.0, color: d2d(b) }];
        unsafe { self.rt.CreateGradientStopCollection(&list, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP) }
    }

    /// The colours with the painter's alpha, as a cache key.
    fn pair(&self, a: Color, b: Color) -> (Color, Color, [u8; 8]) {
        let alpha = self.alpha.get();
        let (a, b) = (a.with_alpha(a.a * alpha), b.with_alpha(b.a * alpha));
        let (x, y) = (a.to_rgba8(), b.to_rgba8());
        (a, b, [x[0], x[1], x[2], x[3], y[0], y[1], y[2], y[3]])
    }

    /// A rounded rectangle filled `top` at its top edge to `bottom` at its bottom edge.
    pub fn gradient(&self, r: Rect, radius: f32, top: Color, bottom: Color) -> Result<()> {
        let (a, b, key) = self.pair(top, bottom);
        let cached = self.r.device.linear.borrow().get(&key).cloned();
        let brush = match cached {
            Some(brush) => brush,
            None => {
                let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: Vector2::new(0.0, 0.0), endPoint: Vector2::new(0.0, 1.0) };
                let brush = unsafe { self.dc.CreateLinearGradientBrush(&props, None, &self.stops(a, b)?)? };
                self.r.device.linear.borrow_mut().insert(key, brush.clone());
                brush
            }
        };
        unsafe {
            brush.SetStartPoint(Vector2::new(r.l, r.t));
            brush.SetEndPoint(Vector2::new(r.l, r.b));
            self.dc.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: rect(r), radiusX: radius, radiusY: radius }, &brush);
        }
        Ok(())
    }

    pub fn clip(&self, r: Rect, f: impl FnOnce() -> Result<()>) -> Result<()> {
        unsafe { self.dc.PushAxisAlignedClip(&rect(r), D2D1_ANTIALIAS_MODE_ALIASED) };
        let result = f();
        unsafe { self.dc.PopAxisAlignedClip() };
        result
    }

    /// Draws `f` scaled by `scale` about the origin, then moved to (`x`, `y`): the island
    /// previews are drawn at the island's own sizes.
    pub fn scaled(&self, x: f32, y: f32, scale: f32, f: impl FnOnce() -> Result<()>) -> Result<()> {
        unsafe {
            let mut previous = Matrix3x2::default();
            self.dc.GetTransform(&mut previous);
            let local = Matrix3x2 { M11: scale, M12: 0.0, M21: 0.0, M22: scale, M31: x, M32: y };
            self.dc.SetTransform(&(local * previous));
            let result = f();
            self.dc.SetTransform(&previous);
            result
        }
    }

    fn layout(&self, text: &str, face: Face, r: Rect, kind: Kind) -> Result<IDWriteTextLayout> {
        self.r.texts.borrow_mut().layout(text, face, r.w(), r.h(), kind)
    }

    fn draw_layout(&self, layout: &IDWriteTextLayout, x: f32, y: f32, color: Color) {
        unsafe { self.rt.DrawTextLayout(Vector2::new(x, y), layout, self.brush(color), D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT) };
    }

    /// One line, vertically centred in `r`, ellipsised if it doesn't fit.
    pub fn text(&self, text: &str, font: Font, r: Rect, align: Align, color: Color) -> Result<()> {
        self.line(text, font, r, align, color, false)
    }

    /// Numerals with a smaller, quieter unit on their baseline ("32" "%"), from the left of
    /// `r`, vertically centred in it.
    #[allow(clippy::too_many_arguments)]
    pub fn figures_with_unit(&self, value: &str, unit: &str, font: Font, unit_style: reui_design::TextStyle, r: Rect, color: Color, unit_color: Color) -> Result<()> {
        let (layout, range) = self.r.texts.borrow().with_unit(value, unit, font, unit_style, r.w(), r.h())?;
        if range.length > 0 {
            let c = unit_color.with_alpha(unit_color.a * self.alpha.get());
            unsafe {
                let brush = self.dc.CreateSolidColorBrush(&d2d(c), None)?;
                layout.SetDrawingEffect(&brush, range)?;
            }
        }
        self.draw_layout(&layout, r.l, self.snap(r.t), color);
        Ok(())
    }

    /// One line with tabular figures (numbers that change in place).
    pub fn figures(&self, text: &str, font: Font, r: Rect, align: Align, color: Color) -> Result<()> {
        self.line(text, font, r, align, color, true)
    }

    fn line(&self, text: &str, font: Font, r: Rect, align: Align, color: Color, tabular: bool) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        let align = match align {
            Align::Start => 0,
            Align::Center => 1,
            Align::End => 2,
        };
        let layout = self.layout(text, Face::Text(font), r, Kind::Line { align, tabular })?;
        self.draw_layout(&layout, r.l, self.snap(r.t), color);
        Ok(())
    }

    /// Wrapped text from the top of `r`.
    pub fn paragraph(&self, text: &str, font: Font, r: Rect, color: Color) -> Result<()> {
        let layout = self.layout(text, Face::Text(font), r, Kind::Paragraph)?;
        self.draw_layout(&layout, r.l, self.snap(r.t), color);
        Ok(())
    }

    /// A Segoe Fluent Icons glyph centred in `r`.
    pub fn glyph(&self, glyph: char, size: f32, r: Rect, color: Color) -> Result<()> {
        let mut buf = [0u8; 4];
        let layout = self.layout(glyph.encode_utf8(&mut buf), Face::Icon(size.to_bits()), r, Kind::Line { align: 1, tabular: false })?;
        self.draw_layout(&layout, r.l, r.t, color);
        Ok(())
    }

    pub fn measure(&self, text: &str, font: Font) -> f32 {
        self.r.width(text, font)
    }
}

pub(super) fn rect(r: Rect) -> D2D_RECT_F {
    D2D_RECT_F { left: r.l, top: r.t, right: r.r, bottom: r.b }
}

pub fn d2d(c: Color) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: c.r, g: c.g, b: c.b, a: c.a }
}

/// `a` blended toward `b` by `t`.
pub fn mix(a: Color, b: Color, t: f32) -> Color {
    let l = |x: f32, y: f32| x + (y - x) * t.clamp(0.0, 1.0);
    Color { r: l(a.r, b.r), g: l(a.g, b.g), b: l(a.b, b.b), a: l(a.a, b.a) }
}

/// `c` with its alpha scaled by `t`.
pub fn fade(c: Color, t: f32) -> Color {
    c.with_alpha(c.a * t.clamp(0.0, 1.0))
}
