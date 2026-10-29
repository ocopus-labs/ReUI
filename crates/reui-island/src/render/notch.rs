//! The Notch style's ears: where the notch meets the screen's top edge, its sides curve out
//! into the edge (a square with a quarter circle cut out), so it reads as part of the
//! screen's frame. Drawn once per look into a small surface the pill places beside the
//! shape, following its width spring.

use reui_design::tokens::notch;
use windows::core::Result;
use windows::UI::Composition::CompositionDrawingSurface;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;

use crate::render::{d2d, Renderer};

/// An ear's surface in DIPs: the ear, and one DIP of solid material that overlaps the
/// notch's side, so the two never show a seam between them at fractional scales.
pub const EAR_SURFACE: (f32, f32) = (notch::EAR + 1.0, notch::EAR);

impl Renderer {
    /// The left ear (its notch to the right), or the right one mirrored. The theme's
    /// hairline follows the curve, continuing the notch's own edge down its side.
    pub fn draw_ear(&self, surface: &CompositionDrawingSurface, right: bool) -> Result<()> {
        let e = notch::EAR;
        let s = self.scale;
        // In DIPs, for the left ear: the curve's centre is at (0, e), so it runs from the
        // screen's edge at (0, 0) down to the notch's side at (e, e).
        let flip = |x: f32| if right { EAR_SURFACE.0 - x } else { x };
        self.paint(surface, |rt| unsafe {
            let base: &ID2D1Factory = &self.factory;
            let path = base.CreatePathGeometry()?;
            let sink = path.Open()?;
            sink.BeginFigure(self.point(flip(0.0), 0.0), D2D1_FIGURE_BEGIN_FILLED);
            sink.AddLine(self.point(flip(e + 1.0), 0.0));
            sink.AddLine(self.point(flip(e + 1.0), e));
            sink.AddLine(self.point(flip(e), e));
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: self.point(flip(0.0), 0.0),
                size: D2D_SIZE_F { width: e * s, height: e * s },
                rotationAngle: 0.0,
                sweepDirection: if right { D2D1_SWEEP_DIRECTION_CLOCKWISE } else { D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE },
                arcSize: D2D1_ARC_SIZE_SMALL,
            });
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            sink.Close()?;
            rt.FillGeometry(&path, &self.brush(rt, self.pal().background)?, None);
            let hairline = self.pal().hairline;
            if hairline.a > 0.0 {
                // One physical pixel inside the material (outside the cut-out circle).
                let half = 0.5 / s;
                let edge = base.CreatePathGeometry()?;
                let sink = edge.Open()?;
                sink.BeginFigure(self.point(flip(e + half), e), D2D1_FIGURE_BEGIN_HOLLOW);
                sink.AddArc(&D2D1_ARC_SEGMENT {
                    point: self.point(flip(0.0), -half),
                    size: D2D_SIZE_F { width: (e + half) * s, height: (e + half) * s },
                    rotationAngle: 0.0,
                    sweepDirection: if right { D2D1_SWEEP_DIRECTION_CLOCKWISE } else { D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE },
                    arcSize: D2D1_ARC_SIZE_SMALL,
                });
                sink.EndFigure(D2D1_FIGURE_END_OPEN);
                sink.Close()?;
                let brush = rt.CreateSolidColorBrush(&d2d(hairline), None)?;
                rt.DrawGeometry(&edge, &brush, 1.0, None);
            }
            Ok(())
        })
    }
}
