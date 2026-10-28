//! Painter primitives beyond fills and text: the cached bitmaps that let scrolling move
//! the page without recording it again, the island stage's top-rounded backdrop, polygons
//! and arcs for the previews, and the Claude mark (SVG).

use reui_design::Color;
use windows::core::{Interface, Result};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::UI::Shell::SHCreateMemStream;
use windows_numerics::{Matrix3x2, Vector2};

use super::{d2d, rect, Painter, Slot, CLAUDE_SVG};
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

    /// The top of a container: `r` with only its top corners rounded, filled with `color`
    /// and then a soft glow of `glow` from its top centre (a page's island stage).
    pub fn stage(&self, r: Rect, radius: f32, color: Color, glow: Color) -> Result<()> {
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
        let (a, b, colors) = self.pair(glow, glow.with_alpha(0.0));
        let cached = self.r.device.radial.borrow().get(&colors).cloned();
        let brush = match cached {
            Some(brush) => brush,
            None => {
                let props = D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES { center: Vector2::new(0.0, 0.0), gradientOriginOffset: Vector2::new(0.0, 0.0), radiusX: 1.0, radiusY: 1.0 };
                let brush = unsafe { self.dc.CreateRadialGradientBrush(&props, None, &self.stops(a, b)?)? };
                self.r.device.radial.borrow_mut().insert(colors, brush.clone());
                brush
            }
        };
        unsafe {
            let mut previous = Matrix3x2::default();
            self.dc.GetTransform(&mut previous);
            self.dc.SetTransform(&(Matrix3x2::translation(r.l, r.t) * previous));
            self.dc.FillGeometry(&shape, self.brush(color), None);
            brush.SetCenter(Vector2::new(r.w() / 2.0, 0.0));
            brush.SetRadiusX(r.w() * 0.62);
            brush.SetRadiusY(r.h() * 1.25);
            self.dc.FillGeometry(&shape, &brush, None);
            self.dc.SetTransform(&previous);
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
