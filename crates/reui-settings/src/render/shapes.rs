//! Painter primitives beyond fills and text: the cached bitmaps that let scrolling move
//! the page without recording it again, the island stage's backdrop (the wallpaper's top
//! centre), polygons, lines and arcs for the previews, and the Claude mark (SVG).

use reui_design::Color;
use windows::core::{Interface, Result};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::UI::Shell::SHCreateMemStream;
use windows_numerics::{Matrix3x2, Vector2};

use super::{d2d, rect, Painter, Slot, CLAUDE_SVG};
use crate::wallpaper::{Backdrop, Strip};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use crate::ui::layout::Rect;

impl Painter<'_> {
    /// Draws `f` through a cached bitmap `w` × `h` DIPs, placed with its top-left at
    /// (`x`, `y`): `f` runs (into the bitmap, origin at its top-left) only when `fresh` is
    /// false or the size changed; otherwise last frame's pixels are reused. Scrolling the
    /// page moves the bitmap and records nothing. The bitmap is opaque, `background` under
    /// what `f` draws.
    #[allow(clippy::too_many_arguments)]
    pub fn cached(&self, which: Slot, fresh: bool, x: f32, y: f32, w: f32, h: f32, background: Color, f: impl FnOnce() -> Result<()>) -> Result<()> {
        let (pw, ph) = (((w * self.scale).ceil() as u32).max(1), ((h * self.scale).ceil() as u32).max(1));
        let mut cache = self.r.device.cache.borrow_mut();
        let slot = &mut cache[which as usize];
        let reuse = matches!(&*slot, Some((_, cw, ch)) if (*cw, *ch) == (pw, ph));
        if !reuse {
            *slot = Some((self.r.gpu.offscreen(pw, ph, self.scale * 96.0)?, pw, ph));
        }
        let (bitmap, ..) = slot.as_ref().expect("cached bitmap");
        if !reuse || !fresh {
            unsafe {
                let previous = self.dc.GetTarget()?;
                let mut transform = Matrix3x2::default();
                self.dc.GetTransform(&mut transform);
                let image: ID2D1Image = bitmap.cast()?;
                self.dc.SetTarget(&image);
                self.dc.SetTransform(&Matrix3x2::identity());
                self.dc.Clear(Some(&d2d(background)));
                let drawn = f();
                self.dc.SetTarget(&previous);
                self.dc.SetTransform(&transform);
                drawn?;
            }
        }
        let (x, y) = (self.snap(x), self.snap(y));
        unsafe {
            self.dc.DrawBitmap(bitmap, Some(&rect(Rect::new(x, y, x + pw as f32 / self.scale, y + ph as f32 / self.scale))), 1.0, D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR, None, None);
        }
        Ok(())
    }

    /// The top of a container: `r` with only its top corners rounded, showing the top
    /// centre of the user's screen (a page's island stage): the wallpaper's colour, its
    /// strip 1 DIP to 1 DIP over it, then `dim` so Settings stays calm. `fallback` when
    /// the wallpaper hasn't been read.
    pub fn backdrop(&self, r: Rect, radius: f32, backdrop: Option<&Backdrop>, fallback: Color, dim: Color) -> Result<()> {
        let key = [r.w().to_bits(), r.h().to_bits(), radius.to_bits()];
        let cached = self.r.device.shapes.borrow().get(&key).cloned();
        let shape = match cached {
            Some(shape) => shape,
            None => {
                let shape = top_rounded(&self.r.gpu.factory, r.w(), r.h(), radius)?;
                self.r.device.shapes.borrow_mut().insert(key, shape.clone());
                shape
            }
        };
        unsafe {
            let mut previous = Matrix3x2::default();
            self.dc.GetTransform(&mut previous);
            self.dc.SetTransform(&(Matrix3x2::translation(r.l, r.t) * previous));
            self.dc.FillGeometry(&shape, self.brush(backdrop.map_or(fallback, |b| b.color)), None);
            if let Some((b, strip)) = backdrop.and_then(|b| Some((b, b.strip.as_ref()?))) {
                let brush = self.strip_brush(b, strip)?;
                // The strip's centre on the stage's, its top on the stage's top, scaled
                // from the monitor's pixels to DIPs; on the pixel grid, so at the same
                // scale every wallpaper pixel is one screen pixel.
                let k = 1.0 / b.scale;
                let x = self.snap(r.l + r.w() / 2.0 - strip.width as f32 * k / 2.0) - r.l;
                brush.SetTransform(&Matrix3x2 { M11: k, M12: 0.0, M21: 0.0, M22: k, M31: x, M32: 0.0 });
                let shown = Rect::new(x.max(0.0), 0.0, (x + strip.width as f32 * k).min(r.w()), (strip.height as f32 * k).min(r.h()));
                self.dc.PushAxisAlignedClip(&rect(shown), D2D1_ANTIALIAS_MODE_ALIASED);
                self.dc.FillGeometry(&shape, &brush, None);
                self.dc.PopAxisAlignedClip();
            }
            self.dc.FillGeometry(&shape, self.brush(dim), None);
            self.dc.SetTransform(&previous);
        }
        Ok(())
    }

    /// The wallpaper strip as a bitmap brush, uploaded once per read.
    fn strip_brush(&self, b: &Backdrop, strip: &Strip) -> Result<ID2D1BitmapBrush1> {
        if let Some((generation, brush)) = &*self.r.device.wallpaper.borrow() {
            if *generation == b.generation {
                return Ok(brush.clone());
            }
        }
        let brush = unsafe {
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                dpiX: 96.0,
                dpiY: 96.0,
                ..Default::default()
            };
            let size = D2D_SIZE_U { width: strip.width, height: strip.height };
            let bitmap = self.dc.CreateBitmap(size, Some(strip.pixels.as_ptr().cast()), strip.width * 4, &props)?;
            let brush_props = D2D1_BITMAP_BRUSH_PROPERTIES1 { extendModeX: D2D1_EXTEND_MODE_CLAMP, extendModeY: D2D1_EXTEND_MODE_CLAMP, interpolationMode: D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC };
            self.dc.CreateBitmapBrush(&bitmap, Some(&brush_props), None)?
        };
        *self.r.device.wallpaper.borrow_mut() = Some((b.generation, brush.clone()));
        Ok(brush)
    }

    /// A straight line from `a` to `b`.
    pub fn segment(&self, a: (f32, f32), b: (f32, f32), width: f32, color: Color) -> Result<()> {
        unsafe { self.dc.DrawLine(Vector2::new(a.0, a.1), Vector2::new(b.0, b.1), self.brush(color), width, None) };
        Ok(())
    }

    /// `r` filled with `color` fading in from nothing at its left edge to solid at its
    /// right (a scrolling row running on past an edge).
    pub fn fade_in_right(&self, r: Rect, color: Color) -> Result<()> {
        let (a, b, _) = self.pair(color.with_alpha(0.0), color);
        unsafe {
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: Vector2::new(r.l, 0.0), endPoint: Vector2::new(r.r, 0.0) };
            let brush = self.dc.CreateLinearGradientBrush(&props, None, &self.stops(a, b)?)?;
            self.dc.FillRectangle(&rect(r), &brush);
        }
        Ok(())
    }

    /// A filled polygon through `points`.
    pub fn polygon(&self, points: &[(f32, f32)], color: Color) -> Result<()> {
        let Some((&(x0, y0), rest)) = points.split_first() else { return Ok(()) };
        unsafe {
            let path = self.r.gpu.factory.CreatePathGeometry()?;
            let sink = path.Open()?;
            sink.BeginFigure(Vector2::new(x0, y0), D2D1_FIGURE_BEGIN_FILLED);
            for &(x, y) in rest {
                sink.AddLine(Vector2::new(x, y));
            }
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            sink.Close()?;
            self.dc.FillGeometry(&path, self.brush(color), None);
        }
        Ok(())
    }

    /// A line through `points` with round joins and caps (a sparkline).
    pub fn polyline(&self, points: &[(f32, f32)], width: f32, color: Color) -> Result<()> {
        let Some((&(x0, y0), rest)) = points.split_first() else { return Ok(()) };
        unsafe {
            let factory = &self.r.gpu.factory;
            let path = factory.CreatePathGeometry()?;
            let sink = path.Open()?;
            sink.BeginFigure(Vector2::new(x0, y0), D2D1_FIGURE_BEGIN_HOLLOW);
            for &(x, y) in rest {
                sink.AddLine(Vector2::new(x, y));
            }
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
            sink.Close()?;
            let props = D2D1_STROKE_STYLE_PROPERTIES { startCap: D2D1_CAP_STYLE_ROUND, endCap: D2D1_CAP_STYLE_ROUND, lineJoin: D2D1_LINE_JOIN_ROUND, ..Default::default() };
            let base: ID2D1Factory = factory.cast()?;
            let style = base.CreateStrokeStyle(&props, None)?;
            self.dc.DrawGeometry(&path, self.brush(color), width, &style);
        }
        Ok(())
    }

    /// The area under a line through `points` down to `bottom`, filled with `color` at
    /// `top` fading to nothing at `bottom` (a sparkline's fill).
    pub fn area_fading(&self, points: &[(f32, f32)], top: f32, bottom: f32, color: Color) -> Result<()> {
        let (Some(&(first, _)), Some(&(last, _))) = (points.first(), points.last()) else { return Ok(()) };
        let (a, b, _) = self.pair(color, color.with_alpha(0.0));
        unsafe {
            let path = self.r.gpu.factory.CreatePathGeometry()?;
            let sink = path.Open()?;
            sink.BeginFigure(Vector2::new(first, bottom), D2D1_FIGURE_BEGIN_FILLED);
            for &(x, y) in points {
                sink.AddLine(Vector2::new(x, y));
            }
            sink.AddLine(Vector2::new(last, bottom));
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            sink.Close()?;
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: Vector2::new(0.0, top), endPoint: Vector2::new(0.0, bottom) };
            let brush = self.dc.CreateLinearGradientBrush(&props, None, &self.stops(a, b)?)?;
            self.dc.FillGeometry(&path, &brush, None);
        }
        Ok(())
    }

    /// An arc of a circle with round caps, clockwise from 12 o'clock over `sweep` (0..=1).
    pub fn arc(&self, cx: f32, cy: f32, radius: f32, width: f32, sweep: f32, color: Color) -> Result<()> {
        let sweep = sweep.clamp(0.0, 0.9999);
        if sweep <= 0.0 {
            return Ok(());
        }
        let angle = sweep * std::f32::consts::TAU;
        let at = |a: f32| Vector2::new(cx + radius * a.sin(), cy - radius * a.cos());
        unsafe {
            let factory = &self.r.gpu.factory;
            let path = factory.CreatePathGeometry()?;
            let sink = path.Open()?;
            sink.BeginFigure(at(0.0), D2D1_FIGURE_BEGIN_HOLLOW);
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: at(angle),
                size: D2D_SIZE_F { width: radius, height: radius },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: if sweep > 0.5 { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
            });
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
            sink.Close()?;
            let props = D2D1_STROKE_STYLE_PROPERTIES { startCap: D2D1_CAP_STYLE_ROUND, endCap: D2D1_CAP_STYLE_ROUND, ..Default::default() };
            let base: ID2D1Factory = factory.cast()?;
            let style = base.CreateStrokeStyle(&props, None)?;
            self.dc.DrawGeometry(&path, self.brush(color), width, &style);
        }
        Ok(())
    }

    /// The Claude logo filling `r` (square), in its own colours.
    pub fn claude(&self, r: Rect) -> Result<()> {
        let size = r.w();
        let key = size.to_bits();
        let cached = self.r.device.svgs.borrow().get(&key).cloned();
        let document = match cached {
            Some(d) => d,
            None => unsafe {
                let Some(stream) = SHCreateMemStream(Some(fit_svg(CLAUDE_SVG).as_bytes())) else { return Ok(()) };
                let d = self.dc.CreateSvgDocument(&stream, D2D_SIZE_F { width: size, height: size })?;
                self.r.device.svgs.borrow_mut().insert(key, d.clone());
                d
            },
        };
        unsafe {
            let mut previous = Matrix3x2::default();
            self.dc.GetTransform(&mut previous);
            self.dc.SetTransform(&(Matrix3x2::translation(r.l, r.t) * previous));
            self.dc.DrawSvgDocument(&document);
            self.dc.SetTransform(&previous);
        }
        Ok(())
    }
}

/// A `w` × `h` rectangle at the origin with its top corners rounded.
fn top_rounded(factory: &ID2D1Factory1, w: f32, h: f32, radius: f32) -> Result<ID2D1PathGeometry> {
    unsafe {
        let path = factory.CreatePathGeometry()?;
        let sink = path.Open()?;
        let arc = |x: f32, y: f32| D2D1_ARC_SEGMENT { point: Vector2::new(x, y), size: D2D_SIZE_F { width: radius, height: radius }, rotationAngle: 0.0, sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE, arcSize: D2D1_ARC_SIZE_SMALL };
        sink.BeginFigure(Vector2::new(0.0, h), D2D1_FIGURE_BEGIN_FILLED);
        sink.AddLine(Vector2::new(0.0, radius));
        sink.AddArc(&arc(radius, 0.0));
        sink.AddLine(Vector2::new(w - radius, 0.0));
        sink.AddArc(&arc(w, radius));
        sink.AddLine(Vector2::new(w, h));
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        sink.Close()?;
        path.cast()
    }
}

/// Drops `width`/`height` from the root `<svg>` tag so the document fills its viewport.
fn fit_svg(text: &str) -> String {
    let Some(start) = text.find("<svg") else { return text.to_string() };
    let Some(len) = text[start..].find('>') else { return text.to_string() };
    let mut tag = text[start..start + len].to_string();
    for attr in [" width=\"1em\"", " height=\"1em\""] {
        tag = tag.replace(attr, "");
    }
    format!("{}{}{}", &text[..start], tag, &text[start + len..])
}
