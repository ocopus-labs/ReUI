//! The horizontally scrolling row of item cards (clipboard history and the like).


use reui_design::tokens::{color, component, icon, space};
use reui_design::Color;
use reui_protocol::{Item, Preview, Rgb};
use windows::core::Result;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows_numerics::Vector2;

use crate::features::clipboard::CardRow;
use crate::render::{rgb_color, Align, Hit, Interaction, Renderer};
use crate::render::images::Decoded;

impl Renderer {
    /// A row of rich preview cards; returns its height. Four across when they fit, else four
    /// and a half, scrolled by `with.scroll` and clipped at the panel's edges; only cards in
    /// view are drawn and hit-tested. Each card carries its own footer (source app icon,
    /// age, one fact), so nothing sits under the cards.
    pub(in crate::render) fn cards(&self, rt: &ID2D1RenderTarget, items: &[Item], y: f32, accent: Color, with: &Interaction, hits: &mut Vec<Hit>) -> Result<f32> {
        let w = self.width.get();
        let row = CardRow::new(w, items.len(), self.look.inset());
        let pad = component::card::PADDING;
        let footer = component::card::FOOTER;
        let cw = row.card_width();
        let ch = component::card::HEIGHT;
        let s = self.scale;
        let scroll = row.clamp(with.scroll);
        let ring = component::card::RING_GAP + component::card::RING_WIDTH;
        // Cut at the content inset, not the surface edge: the pill's corners curve in there.
        let inset = self.look.inset();
        unsafe { rt.PushAxisAlignedClip(&self.rect(inset - ring, y - ring, w - inset + ring, y + ch + ring), D2D1_ANTIALIAS_MODE_ALIASED) };
        let result = (|| -> Result<()> {
            for i in row.visible(scroll) {
                let item = &items[i];
                let l = row.left(i, scroll);
                let (r, b) = (l + cw, y + ch);
                let radius = self.concentric() * s;
                let shape = D2D1_ROUNDED_RECT { rect: self.rect(l, y, r, b), radiusX: radius, radiusY: radius };
                let body = (l + pad, y + pad, cw - 2.0 * pad, ch - footer - 2.0 * pad);
                let selected = with.keyboard && i == with.selected;
                let card_fill = if selected { self.pal().fill_secondary } else { self.pal().fill_tertiary };
                let well = |tint: Color| unsafe {
                    rt.FillRoundedRectangle(&shape, &self.brush(rt, tint)?);
                    Ok::<(), windows::core::Error>(())
                };
                // Footer ink: light over images and dark swatches, quiet over wells.
                let mut on_media = false;
                match &item.preview {
                    Preview::Image(id) => {
                        if matches!(self.images.get(id), Some(Decoded::Bitmap(_))) {
                            // Behind transparent images, so the card keeps its shape.
                            well(self.pal().fill_tertiary)?;
                            self.cover(rt, *id, &shape)?;
                            self.scrim(rt, &shape, b - footer - pad * 2.0, b)?;
                            on_media = true;
                        } else {
                            // Bytes not here yet: the type's glyph, so the card isn't an empty well.
                            well(self.pal().fill_tertiary)?;
                            let g = icon::TILE;
                            self.text(rt, "\u{EB9F}", &self.fonts.icon_tile, l, body.1 + (body.3 - g) / 2.0, cw, Align::Center, self.pal().label_tertiary)?;
                        }
                    }
                    Preview::Text(text) => {
                        well(card_fill)?;
                        // Plain text is the default kind: no badge (code gets one).
                        let first_w = body.2;
                        match text.trim().split_once('\n') {
                            // Several lines: the first reads as a title, the rest quieter.
                            Some((first, rest)) if !first.trim().is_empty() => {
                                let line = self.fonts.card_label.style.line_height;
                                self.text(rt, first.trim(), &self.fonts.card_label, body.0, body.1, first_w, Align::Start, self.pal().label_primary)?;
                                let top = body.1 + line + space::XS;
                                self.text_block(rt, rest.trim_start(), &self.fonts.card_text, body.0, top, body.2, body.1 + body.3 - top, self.pal().label_secondary)?;
                            }
                            _ => self.text_block(rt, text.trim(), &self.fonts.card_text, body.0, body.1, first_w, body.3, self.pal().label_primary)?,
                        }
                    }
                    Preview::Code(text) => {
                        well(card_fill)?;
                        self.kind_glyph(rt, '\u{E943}', r - pad, body.1)?;
                        self.text_block(rt, text, &self.fonts.card_code, body.0, body.1, body.2 - component::card::KIND_ICON - space::XS, body.3, self.pal().label_primary)?;
                    }
                    Preview::Color(rgb) => {
                        let swatch = rgb_color(*rgb);
                        well(swatch)?;
                        let light = 0.2126 * swatch.r + 0.7152 * swatch.g + 0.0722 * swatch.b > 0.55;
                        let ink = if light { color::label::ON_ACCENT } else { color::label::PRIMARY };
                        let Rgb(cr, cg, cb) = *rgb;
                        self.text(rt, &format!("#{cr:02X}{cg:02X}{cb:02X}"), &self.fonts.card_label, body.0, body.1, body.2, Align::Start, ink)?;
                        on_media = !light;
                    }
                    Preview::Link { host, path, title, image, .. } => {
                        well(card_fill)?;
                        let mut text_top = body.1;
                        if let Some(id) = image.filter(|id| matches!(self.images.get(id), Some(Decoded::Bitmap(_)))) {
                            // The page image across the top, rounded only where the card is.
                            let media_b = y + ch * component::card::MEDIA;
                            unsafe { rt.PushAxisAlignedClip(&self.rect(l, y, r, media_b), D2D1_ANTIALIAS_MODE_ALIASED) };
                            self.cover(rt, id, &shape)?;
                            unsafe { rt.PopAxisAlignedClip() };
                            text_top = media_b + pad * 0.75;
                        } else {
                            self.text(rt, "\u{E774}", &self.fonts.icon_compact, body.0, body.1, icon::COMPACT, Align::Start, accent)?;
                            text_top += icon::COMPACT + pad * 0.5;
                        }
                        // The site goes in the footer (the module sends it as the card's fact).
                        let heading = title.clone().unwrap_or_else(|| format!("{host}{path}"));
                        self.text_block(rt, &heading, &self.fonts.card_title, body.0, text_top, body.2, b - footer - pad * 0.5 - text_top, self.pal().label_primary)?;
                    }
                    Preview::Files { name, .. } => {
                        well(card_fill)?;
                        self.text(rt, "\u{E8A5}", &self.fonts.icon_tile, body.0, body.1, icon::TILE, Align::Start, accent)?;
                        let name_top = body.1 + icon::TILE + pad * 0.75;
                        self.text_block(rt, name, &self.fonts.card_title, body.0, name_top, body.2, body.1 + body.3 - name_top, self.pal().label_primary)?;
                    }
                }
                self.card_footer(rt, item, l, b - footer - pad * 0.25, cw, on_media)?;
                if selected {
                    // Concentric with the card: its radius grows by exactly the ring's offset.
                    let out = component::card::RING_GAP + component::card::RING_WIDTH / 2.0;
                    let rr = (self.concentric() + out) * s;
                    let ring = D2D1_ROUNDED_RECT { rect: self.rect(l - out, y - out, r + out, b + out), radiusX: rr, radiusY: rr };
                    unsafe { rt.DrawRoundedRectangle(&ring, &self.brush(rt, accent)?, component::card::RING_WIDTH * s, None) };
                }
                hits.push(Hit { card: Some(i), ..Hit::new(l.max(0.0), y, r.min(w), b, &item.action) });
            }
            // The row runs on past the edge: fade it out there instead of cutting it.
            let fade = component::card::FADE;
            if scroll < row.max_scroll() - 0.5 {
                self.edge_fade(rt, w - inset + ring - fade, w - inset + ring, y - ring, y + ch + ring, true)?;
            }
            if scroll > 0.5 {
                self.edge_fade(rt, inset - ring, inset - ring + fade, y - ring, y + ch + ring, false)?;
            }
            Ok(())
        })();
        unsafe { rt.PopAxisAlignedClip() };
        result?;
        Ok(ch)
    }

    /// The island's background fading in across `left..right` (towards the right edge when
    /// `rightward`), over the card row.
    #[allow(clippy::too_many_arguments)]
    fn edge_fade(&self, rt: &ID2D1RenderTarget, left: f32, right: f32, top: f32, bottom: f32, rightward: bool) -> Result<()> {
        let s = self.scale;
        let bg = self.pal().background;
        let clear = D2D1_COLOR_F { r: bg.r, g: bg.g, b: bg.b, a: 0.0 };
        let solid = D2D1_COLOR_F { r: bg.r, g: bg.g, b: bg.b, a: 1.0 };
        let (from, to) = if rightward { (clear, solid) } else { (solid, clear) };
        unsafe {
            let stops = [D2D1_GRADIENT_STOP { position: 0.0, color: from }, D2D1_GRADIENT_STOP { position: 1.0, color: to }];
            let collection = rt.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)?;
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: Vector2::new(left * s, 0.0), endPoint: Vector2::new(right * s, 0.0) };
            let brush = rt.CreateLinearGradientBrush(&props, None, &collection)?;
            rt.FillRectangle(&self.rect(left, top, right, bottom), &brush);
        }
        Ok(())
    }

    /// A card's small type glyph, right-aligned at `right`, top at `top`.
    fn kind_glyph(&self, rt: &ID2D1RenderTarget, glyph: char, right: f32, top: f32) -> Result<()> {
        let size = component::card::KIND_ICON;
        self.text(rt, &glyph.to_string(), &self.fonts.icon_kind, right - size, top, size, Align::Center, self.pal().label_tertiary)
    }

    /// Source app icon, age, a star when pinned, and one fact on the right, inside the
    /// bottom of a card. The age keeps its width; the fact ellipsises in what's left.
    fn card_footer(&self, rt: &ID2D1RenderTarget, item: &Item, l: f32, top: f32, w: f32, on_media: bool) -> Result<()> {
        let pad = component::card::PADDING;
        let icon = component::card::ICON;
        let mid = top + component::card::FOOTER / 2.0;
        let (ink, quiet) = if on_media { (color::label::PRIMARY, color::label::PRIMARY.with_alpha(0.75)) } else { (self.pal().label_secondary, self.pal().label_tertiary) };
        let mut x = l + pad;
        let right = l + w - pad;
        if let Some(id) = item.icon.filter(|id| matches!(self.images.get(id), Some(Decoded::Bitmap(_)))) {
            let r = icon * 0.25 * self.scale;
            let shape = D2D1_ROUNDED_RECT { rect: self.rect(x, mid - icon / 2.0, x + icon, mid + icon / 2.0), radiusX: r, radiusY: r };
            self.cover(rt, id, &shape)?;
            x += icon + pad * 0.5;
        }
        let line = self.fonts.caption.style.line_height;
        let age_w = self.text_width(&item.age, &self.fonts.caption)?.min((right - x).max(0.0));
        self.text(rt, &item.age, &self.fonts.caption, x, mid - line / 2.0, age_w, Align::Start, ink)?;
        x += age_w;
        if item.pinned {
            let star = icon::STAR;
            x += pad * 0.5;
            self.text(rt, "\u{E735}", &self.fonts.icon_star, x, mid - star / 2.0, star, Align::Center, ink)?;
            x += star;
        }
        if let Some(meta) = &item.meta {
            let from = x + pad * 0.75;
            if right > from {
                self.text(rt, meta, &self.fonts.caption, from, mid - line / 2.0, right - from, Align::End, quiet)?;
            }
        }
        Ok(())
    }
}
