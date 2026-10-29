//! A quiet centred note standing in for empty content.


use windows::core::Result;
use windows::Win32::Graphics::Direct2D::*;

use crate::render::{Align, Renderer};

impl Renderer {
    /// One quiet centred line in a row `height` tall.
    pub(in crate::render) fn note(&self, rt: &ID2D1RenderTarget, text: &str, y: f32, height: f32) -> Result<f32> {
        let inset = self.look.inset();
        let line = self.fonts.note.style.line_height;
        self.text(rt, text, &self.fonts.note, inset, y + (height - line) / 2.0, self.width.get() - 2.0 * inset, Align::Center, self.pal().label_tertiary)?;
        Ok(height)
    }
}
