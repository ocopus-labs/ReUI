//! Drawing, with Direct2D and DirectWrite into the window, like the island: every colour,
//! size and face from the tokens. The render target works in DIPs (its DPI is the
//! window's), so layout numbers are used as they are and snapped to pixels where edges
//! must stay crisp. Here: the [`Renderer`] (device resources) and the [`Painter`]
//! primitives; what gets drawn lives in the child modules.

mod controls;
mod fonts;
pub mod frame;
pub mod menu;
mod page;

use std::cell::Cell;

use reui_design::Color;
use windows::core::{Interface, Result};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows_numerics::Vector2;

pub use self::controls::part_of;
pub use self::fonts::Fonts;
pub use self::frame::View;
use crate::ui::layout::{Font, Rect};

const D2DERR_RECREATE_TARGET: i32 = 0x8899_000C_u32 as i32;

pub struct Renderer {
    factory: ID2D1Factory,
    dwrite: IDWriteFactory,
    pub fonts: Fonts,
    tabular: IDWriteTypography,
    target: Option<ID2D1HwndRenderTarget>,
    dpi: f32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Start,
    Center,
    End,
}

impl Renderer {
    pub fn new(dpi: f32) -> Result<Self> {
        let factory: ID2D1Factory = unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        let dwrite: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        let tabular = unsafe {
            let t = dwrite.CreateTypography()?;
            t.AddFontFeature(DWRITE_FONT_FEATURE { nameTag: DWRITE_FONT_FEATURE_TAG_TABULAR_FIGURES, parameter: 1 })?;
            t
        };
        let fonts = Fonts::new(&dwrite)?;
        Ok(Self { factory, dwrite, fonts, tabular, target: None, dpi })
    }

    pub fn set_dpi(&mut self, dpi: f32) {
        self.dpi = dpi;
        if let Some(target) = &self.target {
            unsafe { target.SetDpi(dpi, dpi) };
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if let Some(target) = &self.target {
            unsafe { _ = target.Resize(&D2D_SIZE_U { width, height }) };
        }
    }

    /// Draws a frame; the target is made (again) as needed.
    pub fn draw(&mut self, hwnd: HWND, width: u32, height: u32, f: impl FnOnce(&Painter) -> Result<()>) -> Result<()> {
        if self.target.is_none() {
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                pixelFormat: D2D1_PIXEL_FORMAT { format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_IGNORE },
                dpiX: self.dpi,
                dpiY: self.dpi,
                ..Default::default()
            };
            let hwnd_props = D2D1_HWND_RENDER_TARGET_PROPERTIES { hwnd, pixelSize: D2D_SIZE_U { width, height }, presentOptions: D2D1_PRESENT_OPTIONS_NONE };
            self.target = Some(unsafe { self.factory.CreateHwndRenderTarget(&props, &hwnd_props)? });
        }
        let target = self.target.clone().expect("render target");
        let rt: ID2D1RenderTarget = target.cast()?;
        let painter = Painter { rt, factory: &self.factory, dwrite: &self.dwrite, fonts: &self.fonts, tabular: &self.tabular, scale: self.dpi / 96.0, alpha: Cell::new(1.0) };
        unsafe {
            painter.rt.BeginDraw();
            painter.rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            painter.rt.Clear(Some(&d2d(reui_design::tokens::settings::color::BACKGROUND)));
        }
        let drawn = f(&painter);
        let ended = unsafe { painter.rt.EndDraw(None, None) };
        if let Err(e) = &ended {
            if e.code().0 == D2DERR_RECREATE_TARGET {
                self.target = None;
            }
        }
        drawn.and(ended)
    }

    pub fn dwrite(&self) -> &IDWriteFactory {
        &self.dwrite
    }

    pub fn tabular(&self) -> &IDWriteTypography {
        &self.tabular
    }
}

/// One frame's drawing calls, in DIPs.
pub struct Painter<'a> {
    rt: ID2D1RenderTarget,
    factory: &'a ID2D1Factory,
    dwrite: &'a IDWriteFactory,
    pub fonts: &'a Fonts,
    tabular: &'a IDWriteTypography,
    scale: f32,
    /// Multiplies every colour's alpha (disabled rows draw quieter).
    pub alpha: Cell<f32>,
}

impl Painter<'_> {
    /// `v` on the pixel grid.
    pub fn snap(&self, v: f32) -> f32 {
        (v * self.scale).round() / self.scale
    }

    fn brush(&self, color: Color) -> Result<ID2D1SolidColorBrush> {
        let c = color.with_alpha(color.a * self.alpha.get());
        unsafe { self.rt.CreateSolidColorBrush(&d2d(c), None) }
    }

    pub fn fill(&self, r: Rect, radius: f32, color: Color) -> Result<()> {
        let shape = D2D1_ROUNDED_RECT { rect: rect(r), radiusX: radius, radiusY: radius };
        unsafe { self.rt.FillRoundedRectangle(&shape, &self.brush(color)?) };
        Ok(())
    }

    /// A rounded rectangle whose top and bottom corners differ (rows inside a group).
    pub fn fill_corners(&self, r: Rect, top: f32, bottom: f32, color: Color) -> Result<()> {
        if (top - bottom).abs() < 0.01 {
            return self.fill(r, top, color);
        }
        unsafe {
            let path = self.factory.CreatePathGeometry()?;
            let sink = path.Open()?;
            let arc = |sink: &ID2D1GeometrySink, x: f32, y: f32, radius: f32| {
                sink.AddArc(&D2D1_ARC_SEGMENT {
                    point: Vector2::new(x, y),
                    size: D2D_SIZE_F { width: radius, height: radius },
                    rotationAngle: 0.0,
                    sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                    arcSize: D2D1_ARC_SIZE_SMALL,
                })
            };
            sink.BeginFigure(Vector2::new(r.l + top, r.t), D2D1_FIGURE_BEGIN_FILLED);
            sink.AddLine(Vector2::new(r.r - top, r.t));
            arc(&sink, r.r, r.t + top, top);
            sink.AddLine(Vector2::new(r.r, r.b - bottom));
            arc(&sink, r.r - bottom, r.b, bottom);
            sink.AddLine(Vector2::new(r.l + bottom, r.b));
            arc(&sink, r.l, r.b - bottom, bottom);
            sink.AddLine(Vector2::new(r.l, r.t + top));
            arc(&sink, r.l + top, r.t, top);
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            sink.Close()?;
            self.rt.FillGeometry(&path, &self.brush(color)?, None);
        }
        Ok(())
    }

    /// A ring along a rounded rectangle, centred on its edge.
    pub fn stroke(&self, r: Rect, radius: f32, width: f32, color: Color) -> Result<()> {
        let shape = D2D1_ROUNDED_RECT { rect: rect(r), radiusX: radius, radiusY: radius };
        unsafe { self.rt.DrawRoundedRectangle(&shape, &self.brush(color)?, width, None) };
        Ok(())
    }

    pub fn circle(&self, cx: f32, cy: f32, radius: f32, color: Color) -> Result<()> {
        let e = D2D1_ELLIPSE { point: Vector2::new(cx, cy), radiusX: radius, radiusY: radius };
        unsafe { self.rt.FillEllipse(&e, &self.brush(color)?) };
        Ok(())
    }

    /// From `color` at the top to transparent at the bottom (content scrolling under the
    /// caption fades out rather than being cut).
    pub fn fade_down(&self, r: Rect, color: Color) -> Result<()> {
        unsafe {
            let stops = [D2D1_GRADIENT_STOP { position: 0.0, color: d2d(color) }, D2D1_GRADIENT_STOP { position: 1.0, color: d2d(color.with_alpha(0.0)) }];
            let collection = self.rt.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)?;
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: Vector2::new(r.l, r.t), endPoint: Vector2::new(r.l, r.b) };
            let brush = self.rt.CreateLinearGradientBrush(&props, None, &collection)?;
            self.rt.FillRectangle(&rect(r), &brush);
        }
        Ok(())
    }

    pub fn clip(&self, r: Rect, f: impl FnOnce() -> Result<()>) -> Result<()> {
        unsafe { self.rt.PushAxisAlignedClip(&rect(r), D2D1_ANTIALIAS_MODE_ALIASED) };
        let result = f();
        unsafe { self.rt.PopAxisAlignedClip() };
        result
    }

    fn layout(&self, text: &str, font: Font, width: f32, height: f32, tabular: bool) -> Result<IDWriteTextLayout> {
        let wide: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            let layout = self.dwrite.CreateTextLayout(&wide, self.fonts.format(font), width.max(0.0), height.max(0.0))?;
            if tabular {
                layout.SetTypography(self.tabular, DWRITE_TEXT_RANGE { startPosition: 0, length: wide.len() as u32 })?;
            }
            Ok(layout)
        }
    }

    /// One line, vertically centred in `r`, ellipsised if it doesn't fit.
    pub fn text(&self, text: &str, font: Font, r: Rect, align: Align, color: Color) -> Result<()> {
        self.line(text, font, r, align, color, false)
    }

    /// The same with tabular figures (numbers that change in place).
    pub fn figures(&self, text: &str, font: Font, r: Rect, align: Align, color: Color) -> Result<()> {
        self.line(text, font, r, align, color, true)
    }

    fn line(&self, text: &str, font: Font, r: Rect, align: Align, color: Color, tabular: bool) -> Result<()> {
        let layout = self.layout(text, font, r.w(), r.h(), tabular)?;
        unsafe {
            layout.SetTextAlignment(match align {
                Align::Start => DWRITE_TEXT_ALIGNMENT_LEADING,
                Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
                Align::End => DWRITE_TEXT_ALIGNMENT_TRAILING,
            })?;
            layout.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            self.rt.DrawTextLayout(Vector2::new(r.l, self.snap(r.t)), &layout, &self.brush(color)?, D2D1_DRAW_TEXT_OPTIONS_CLIP | D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT);
        }
        Ok(())
    }

    /// Wrapped text from the top of `r`.
    pub fn paragraph(&self, text: &str, font: Font, r: Rect, color: Color) -> Result<()> {
        let layout = self.layout(text, font, r.w(), r.h(), false)?;
        unsafe {
            layout.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
            layout.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)?;
            let line = self.fonts.style(font).line_height;
            layout.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, line, line * 0.8)?;
            self.rt.DrawTextLayout(Vector2::new(r.l, r.t), &layout, &self.brush(color)?, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT);
        }
        Ok(())
    }

    /// A Segoe Fluent Icons glyph centred in `r`.
    pub fn glyph(&self, glyph: char, size: f32, r: Rect, color: Color) -> Result<()> {
        let wide: Vec<u16> = glyph.to_string().encode_utf16().collect();
        unsafe {
            let layout = self.dwrite.CreateTextLayout(&wide, self.fonts.icon(size), r.w(), r.h())?;
            layout.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            layout.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            self.rt.DrawTextLayout(Vector2::new(r.l, r.t), &layout, &self.brush(color)?, D2D1_DRAW_TEXT_OPTIONS_NONE);
        }
        Ok(())
    }

    pub fn measure(&self, text: &str, font: Font) -> f32 {
        fonts::width(self.dwrite, self.fonts, self.tabular, text, font)
    }
}

fn rect(r: Rect) -> D2D_RECT_F {
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
