//! The tab row, scrolled so the selected tab stays in view.


use reui_design::tokens::component;
use reui_protocol::Tab;
use windows::core::Result;
use windows::Win32::Graphics::Direct2D::*;

use crate::features::clipboard;
use crate::render::{Align, Hit, Renderer};

impl Renderer {
    /// Filter tabs: "Label count" capsules, the selected one filled. When they're wider than
    /// the panel the row slides to keep the selected one in view, clipped at the edges.
    pub(in crate::render) fn tabs(&self, rt: &ID2D1RenderTarget, tabs: &[Tab], y: f32, hits: &mut Vec<Hit>) -> Result<f32> {
        let h = component::tabs::HEIGHT;
        let pad = component::tabs::PADDING;
        let gap = component::tabs::GAP;
        let inset = self.look.inset();
        let w = self.width.get();
        let mut sizes = Vec::with_capacity(tabs.len());
        for tab in tabs {
            let label = self.text_width(&tab.label, &self.fonts.tab)?;
            let count = self.text_width(&tab.count.to_string(), &self.fonts.tab_count)?;
            sizes.push((label, count, pad + label + component::tabs::COUNT_GAP + count + pad));
        }
        let widths: Vec<f32> = sizes.iter().map(|s| s.2).collect();
        let offset = clipboard::tabs_offset(&widths, gap, tabs.iter().position(|t| t.selected), w - 2.0 * inset);
        let line = self.fonts.tab.style.line_height;
        let top = y + (h - line) / 2.0;
        unsafe { rt.PushAxisAlignedClip(&self.rect(inset, y, w - inset, y + h), D2D1_ANTIALIAS_MODE_ALIASED) };
        let result = (|| -> Result<()> {
            let mut x = inset - offset;
            for (tab, (label_w, count_w, tw)) in tabs.iter().zip(sizes) {
                if x + tw > 0.0 && x < w {
                    // Plain text; the selected tab sits on a soft fill (no bright pill).
                    let (ink, quiet) = if tab.selected { (self.pal().label_primary, self.pal().label_secondary) } else { (self.pal().label_secondary, self.pal().label_tertiary) };
                    if tab.selected {
                        unsafe { self.round_rect(rt, x, y, x + tw, y + h, self.capsule(h, inset), self.pal().fill_secondary)? };
                    }
                    self.text(rt, &tab.label, &self.fonts.tab, x + pad, top, label_w + 1.0, Align::Start, ink)?;
                    self.text(rt, &tab.count.to_string(), &self.fonts.tab_count, x + tw - pad - count_w, top, count_w + 1.0, Align::Start, quiet)?;
                    hits.push(Hit::new(x.max(0.0), y, (x + tw).min(w), y + h, &tab.action));
                }
                x += tw + gap;
            }
            Ok(())
        })();
        unsafe { rt.PopAxisAlignedClip() };
        result?;
        Ok(h)
    }
}
