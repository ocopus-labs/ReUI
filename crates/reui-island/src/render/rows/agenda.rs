//! An agenda (a calendar's rest of the day): one line per thing coming up, the time in a
//! column of its own so the titles line up, the title, and the quieter where at the line's
//! end. Where things go is `reui_design::layout::rows::agenda_*`.

use reui_design::layout::rows;
use reui_design::tokens::{component, typography};
use reui_protocol::AgendaEntry;
use windows::core::Result;
use windows::Win32::Graphics::Direct2D::*;

use crate::render::{Align, Renderer};

impl Renderer {
    /// The agenda's lines from `y`; returns the row's height.
    pub(in crate::render) fn agenda(&self, rt: &ID2D1RenderTarget, entries: &[AgendaEntry], y: f32) -> Result<f32> {
        let p = self.pal();
        let font = &self.fonts.fact;
        let mut time_w: f32 = 0.0;
        for e in entries {
            time_w = time_w.max(self.text_width(&rows::caption_widest(&e.time), font)?);
        }
        let inset = self.look.inset();
        let step = typography::FACT.line_height + component::agenda::LINE_GAP;
        for (i, e) in entries.iter().enumerate() {
            let top = y + i as f32 * step;
            let detail_w = match &e.detail {
                Some(d) => self.text_width(d, font)?,
                None => 0.0,
            };
            let (title_left, title_w, detail_left, detail_w) = rows::agenda_line(&self.look, self.width.get(), time_w, detail_w);
            self.text(rt, &e.time, font, inset, top, time_w, Align::Start, p.label_secondary)?;
            self.text(rt, &e.title, font, title_left, top, title_w, Align::Start, p.label_primary)?;
            if let (Some(d), true) = (&e.detail, detail_w > 0.0) {
                self.text(rt, d, font, detail_left, top, detail_w, Align::End, p.label_tertiary)?;
            }
        }
        Ok(rows::agenda_height(entries.len()))
    }
}
