//! Gauges side by side (System stats' CPU and Memory) and the line of facts under them.
//! Where everything goes is `reui_design::layout::gauges`; this draws it.
//!
//! One accent for all of it (rings, lines, the area under them); the only other colour is
//! a number that has stayed high, in `Warning`. Nothing here moves by itself: a new
//! reading is a new snapshot, and the card is drawn again once.

use reui_design::layout::gauges;
use reui_design::tokens::{component, opacity, typography};
use reui_design::Color;
use reui_protocol::{Fact, Gauge, Tone, GAUGE_HISTORY};
use windows::core::Result;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows_numerics::Vector2;

use crate::render::{d2d, Align, Renderer, Slot};

impl Renderer {
    /// The gauges in equal columns from `y`; returns the row's height.
    pub(in crate::render) fn gauges(&self, rt: &ID2D1RenderTarget, list: &[Gauge], y: f32, accent: Color) -> Result<f32> {
        let (lefts, col_w) = gauges::columns(&self.look, self.width.get(), list.len());
        let g = gauges::gauge(&self.look, col_w, list.iter().any(|g| g.detail.is_some()));
        for (gauge, x) in list.iter().zip(lefts) {
            self.gauge(rt, gauge, &g, x, y, col_w, accent)?;
        }
        Ok(g.height)
    }

    #[allow(clippy::too_many_arguments)]
    fn gauge(&self, rt: &ID2D1RenderTarget, gauge: &Gauge, g: &gauges::Gauge, x: f32, y: f32, col_w: f32, accent: Color) -> Result<()> {
        let p = self.pal();
        let (cx, cy) = (x + g.ring.0, y + g.ring.1);
        self.ring(rt, gauge.fraction, Slot::Leading, cx, cy, accent)?;
        let glyph = component::gauge::GLYPH;
        self.text(rt, &gauge.glyph.to_string(), &self.fonts.icon_gauge, cx - glyph / 2.0, cy - glyph / 2.0, glyph, Align::Center, p.label_secondary)?;

        let (tx, tw) = (x + g.text_left, g.text_w);
        self.text(rt, &gauge.label, &self.fonts.gauge_label, tx, y + g.label, tw, Align::Start, p.label_secondary)?;
        let ink = match gauge.tone {
            Tone::Primary => p.label_primary,
            tone => self.tone_color(tone, accent),
        };
        // The unit follows the number's colour when it warns, so "97%" reads as one thing.
        let unit_ink = if gauge.tone == Tone::Primary { p.label_secondary } else { ink };
        self.value_with_unit(rt, &gauge.value, &gauge.unit, &self.fonts.value, typography::GAUGE_UNIT, tx, y + g.value, tw, ink, unit_ink)?;
        if let Some(detail) = &gauge.detail {
            self.text(rt, detail, &self.fonts.detail, tx, y + g.detail, tw, Align::Start, p.label_secondary)?;
        }
        self.sparkline(rt, &gauge.history, x, x + col_w, y + g.spark, accent)
    }

    /// The last minute: the zero line in the track colour across the whole span, the
    /// accent area fading down to it, the line, and the newest point marked.
    fn sparkline(&self, rt: &ID2D1RenderTarget, history: &[f32], left: f32, right: f32, top: f32, accent: Color) -> Result<()> {
        let s = self.scale;
        let zero = gauges::spark_zero(top);
        unsafe {
            rt.DrawLine(self.point(left, zero), self.point(right, zero), &self.brush(rt, self.pal().fill_track)?, component::gauge::SPARK_TRACK * s, &self.round);
        }
        let points = gauges::spark_points(history, GAUGE_HISTORY, left, right, top);
        let Some(&(nx, ny)) = points.last() else { return Ok(()) };
        if points.len() > 1 {
            let base: &ID2D1Factory = &self.factory;
            unsafe {
                // The area: down from the line to the zero line, closed.
                let area = base.CreatePathGeometry()?;
                let sink = area.Open()?;
                sink.BeginFigure(self.point(points[0].0, zero), D2D1_FIGURE_BEGIN_FILLED);
                for &(px, py) in &points {
                    sink.AddLine(self.point(px, py));
                }
                sink.AddLine(self.point(nx, zero));
                sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                sink.Close()?;
                let stops = [
                    D2D1_GRADIENT_STOP { position: 0.0, color: d2d(accent.with_alpha(accent.a * opacity::SPARK_FILL)) },
                    D2D1_GRADIENT_STOP { position: 1.0, color: d2d(accent.with_alpha(0.0)) },
                ];
                let collection = rt.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)?;
                let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: Vector2::new(0.0, top * s), endPoint: Vector2::new(0.0, zero * s) };
                let fill = rt.CreateLinearGradientBrush(&props, None, &collection)?;
                rt.FillGeometry(&area, &fill, None);

                let line = base.CreatePathGeometry()?;
                let sink = line.Open()?;
                sink.BeginFigure(self.point(points[0].0, points[0].1), D2D1_FIGURE_BEGIN_HOLLOW);
                for &(px, py) in &points[1..] {
                    sink.AddLine(self.point(px, py));
                }
                sink.EndFigure(D2D1_FIGURE_END_OPEN);
                sink.Close()?;
                rt.DrawGeometry(&line, &self.brush(rt, accent)?, component::gauge::SPARK_STROKE * s, &self.round);
            }
        }
        let r = component::gauge::SPARK_DOT / 2.0 * s;
        unsafe { rt.FillEllipse(&D2D1_ELLIPSE { point: self.point(nx, ny), radiusX: r, radiusY: r }, &self.brush(rt, accent)?) };
        Ok(())
    }

    /// Facts spread across the card from `y`, each a quiet label over its value; returns
    /// the row's height.
    pub(in crate::render) fn facts(&self, rt: &ID2D1RenderTarget, facts: &[Fact], y: f32, accent: Color) -> Result<f32> {
        let p = self.pal();
        let mut widths = Vec::with_capacity(facts.len());
        for f in facts {
            widths.push(self.text_width(&f.label, &self.fonts.fact_label)?.max(self.text_width(&f.value, &self.fonts.fact)?));
        }
        let value_top = gauges::fact_value_top();
        for (f, (x, w)) in facts.iter().zip(gauges::facts(&self.look, self.width.get(), &widths)) {
            let ink = match f.tone {
                Tone::Primary => p.label_primary,
                tone => self.tone_color(tone, accent),
            };
            self.text(rt, &f.label, &self.fonts.fact_label, x, y, w, Align::Start, p.label_tertiary)?;
            self.text(rt, &f.value, &self.fonts.fact, x, y + value_top, w, Align::Start, ink)?;
        }
        Ok(gauges::facts_height())
    }
}
