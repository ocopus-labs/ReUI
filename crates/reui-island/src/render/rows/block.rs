//! A text block (a full command, an edit's changed lines, a plan): at most
//! `block.max-height` tall, scrolling inside the card with the wheel instead of growing it.

use reui_design::tokens::{color, component, island, radius};
use reui_design::Color;
use reui_protocol::{diff_tone, BlockKind};
use windows::core::Result;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;

use crate::render::text::utf16_prefix;
use crate::render::{tone_color, Align, Hit, Renderer};

/// More than this is never worth laying out in a card that shows a few lines of it.
const MAX_LINES: usize = 400;
const MAX_PROSE: usize = 20_000;

impl Renderer {
    /// The block at `y`, scrolled by `scroll` (clamped here); returns its height and how far
    /// it can scroll. Code and diffs sit in a quiet well (concentric with the island) as cut
    /// monospace lines, diff lines tinted by what they do; prose is bare wrapped text.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::render) fn block(&self, rt: &ID2D1RenderTarget, text: &str, kind: BlockKind, y: f32, scroll: f32, accent: Color, hits: &mut Vec<Hit>) -> Result<(f32, f32)> {
        let inset = island::expanded::INSET;
        let (l, r) = (inset, self.width.get() - inset);
        let s = self.scale;
        if kind == BlockKind::Prose {
            let font = &self.fonts.block_prose;
            let wide = utf16_prefix(text, MAX_PROSE);
            let layout = unsafe { self.dwrite.CreateTextLayout(&wide, &font.format, (r - l) * s, 100_000.0)? };
            let mut metrics = DWRITE_TEXT_METRICS::default();
            unsafe { layout.GetMetrics(&mut metrics)? };
            let content = (metrics.height / s).ceil();
            let shown = content.min(component::block::MAX_HEIGHT);
            let max = content - shown;
            let scroll = scroll.clamp(0.0, max);
            unsafe {
                rt.PushAxisAlignedClip(&self.rect(l, y, r, y + shown), D2D1_ANTIALIAS_MODE_ALIASED);
                rt.DrawTextLayout(self.point(l, y - scroll), &layout, &self.brush(rt, color::label::SECONDARY)?, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT);
                rt.PopAxisAlignedClip();
            }
            hits.push(Hit { block: true, ..Hit::new(l, y, r, y + shown, "") });
            return Ok((shown, max));
        }

        let font = &self.fonts.block_code;
        let line = font.style.line_height;
        let pad = component::block::PADDING;
        let lines: Vec<&str> = text.lines().take(MAX_LINES).collect();
        let content = lines.len().max(1) as f32 * line;
        // Whole lines only, so the last one shown is never cut through.
        let shown = content.min(((component::block::MAX_HEIGHT - 2.0 * pad) / line).floor() * line);
        let h = shown + 2.0 * pad;
        let max = content - shown;
        let scroll = scroll.clamp(0.0, max);
        unsafe { self.round_rect(rt, l, y, r, y + h, radius::CONCENTRIC.min(h / 2.0), color::fill::QUATERNARY)? };
        let (top, bottom) = (y + pad, y + pad + shown);
        unsafe { rt.PushAxisAlignedClip(&self.rect(l, top, r, bottom), D2D1_ANTIALIAS_MODE_ALIASED) };
        let drawn = (|| -> Result<()> {
            let first = (scroll / line).floor() as usize;
            for (i, text) in lines.iter().enumerate().skip(first) {
                let at = top + i as f32 * line - scroll;
                if at >= bottom {
                    break;
                }
                let ink = match kind {
                    BlockKind::Diff => tone_color(diff_tone(text), accent),
                    _ => color::label::PRIMARY,
                };
                self.text(rt, text, font, l + pad, at, r - l - 2.0 * pad, Align::Start, ink)?;
            }
            Ok(())
        })();
        unsafe { rt.PopAxisAlignedClip() };
        drawn?;
        hits.push(Hit { block: true, ..Hit::new(l, y, r, y + h, "") });
        Ok((h, max))
    }
}
