//! Progress and timeline bars: the meter, its seek thumb, and the times beside or under it.


use reui_design::tokens::{component, space};
use reui_design::Color;
use reui_protocol::Button;
use windows::core::Result;
use windows::Win32::Graphics::Direct2D::*;

use crate::render::{Align, Hit, Renderer};

/// A bar row as drawn: progress, a timeline, or a live stream (no bar, a fact instead).
pub(in crate::render) struct Meter {
    pub(in crate::render) fraction: Option<f32>,
    pub(in crate::render) leading: Option<String>,
    pub(in crate::render) trailing: Option<String>,
    /// The track length, when the bar seeks.
    pub(in crate::render) seek: Option<u64>,
}

impl Meter {
    pub(in crate::render) fn has_times(&self) -> bool {
        self.leading.is_some() || self.trailing.is_some()
    }
}

impl Renderer {
    /// A bar from `span.0` to `span.1` at `y`: track, accent fill, and a thumb at the fill's
    /// end while hovered or dragged. A seekable one gets a generous hit around it. Returns
    /// its height.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::render) fn meter(&self, rt: &ID2D1RenderTarget, span: (f32, f32), y: f32, fraction: f32, seek: Option<u64>, thumb: bool, accent: Color, hits: &mut Vec<Hit>) -> Result<f32> {
        let (l, r) = span;
        let h = component::progress::HEIGHT;
        let end = l + (r - l) * fraction.clamp(0.0, 1.0);
        unsafe {
            self.round_rect(rt, l, y, r, y + h, h / 2.0, self.pal().fill_track)?;
            self.round_rect(rt, l, y, end, y + h, h / 2.0, accent)?;
        }
        if let Some(length) = seek {
            if thumb {
                self.thumb(rt, end, y + h / 2.0)?;
            }
            let pad = component::seek::HIT_PAD;
            hits.push(Hit { seek: Some((l, r, length)), ..Hit::new(l, y - pad, r, y + h + pad, "seek") });
        }
        Ok(h)
    }

    /// The round handle of a seekable bar, centred on (`x`, `mid`).
    pub(in crate::render) fn thumb(&self, rt: &ID2D1RenderTarget, x: f32, mid: f32) -> Result<()> {
        let r = component::thumb::SIZE / 2.0 * self.scale;
        let circle = D2D1_ELLIPSE { point: self.point(x, mid), radiusX: r, radiusY: r };
        unsafe { rt.FillEllipse(&circle, &self.brush(rt, self.pal().label_primary)?) };
        Ok(())
    }

    /// Do the times fit at the ends of a row of these glyph controls?
    pub(in crate::render) fn fits_between(&self, buttons: &[Button], meter: &Meter) -> Result<bool> {
        let n = buttons.len() as f32;
        let group = n * component::button::TRANSPORT + (n - 1.0).max(0.0) * component::button::TRANSPORT_GAP;
        let free = (self.width.get() - 2.0 * self.look.inset() - group) / 2.0;
        let widest = self.caption_width(meter.leading.as_deref())?.max(self.caption_width(meter.trailing.as_deref())?);
        Ok(widest + space::SM <= free)
    }

    /// Width a caption needs, with every digit counted as the widest so a ticking time
    /// doesn't move what's next to it (tabular figures make that every digit).
    pub(in crate::render) fn caption_width(&self, text: Option<&str>) -> Result<f32> {
        match text {
            Some(t) => self.text_width(&t.replace(|c: char| c.is_ascii_digit(), "0"), &self.fonts.caption),
            None => Ok(0.0),
        }
    }

    /// Elapsed / remaining times at the two ends of the bar, centred on `mid`.
    pub(in crate::render) fn captions(&self, rt: &ID2D1RenderTarget, leading: Option<&str>, trailing: Option<&str>, mid: f32) -> Result<()> {
        let inset = self.look.inset();
        let (l, r) = (inset, self.width.get() - inset);
        let top = mid - self.fonts.caption.style.line_height / 2.0;
        if let Some(text) = leading {
            self.text(rt, text, &self.fonts.caption, l, top, (r - l) / 2.0, Align::Start, self.pal().label_tertiary)?;
        }
        if let Some(text) = trailing {
            self.text(rt, text, &self.fonts.caption, l + (r - l) / 2.0, top, (r - l) / 2.0, Align::End, self.pal().label_tertiary)?;
        }
        Ok(())
    }
}
