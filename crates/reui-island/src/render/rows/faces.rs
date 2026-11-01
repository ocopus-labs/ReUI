//! One card's face (`docs/PRD-clipboard-panel-v3.md` §3): its body on top (text, code,
//! a link's site, title and address, an image shown whole, a colour), and its footer at
//! the bottom: what it is (the tag) over its name and age, or, without a name, the tag, a
//! fact and the age on one line. Cards holding media sit on a fine grain.


use reui_design::layout::cards::Face;
use reui_design::tokens::{color, component, icon, opacity};
use reui_design::Color;
use reui_protocol::{ImageId, Item, Preview, Rgb, Tone};
use windows::core::Result;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows_numerics::Matrix3x2;

use crate::render::images::Decoded;
use crate::render::{rgb_color, Align, Renderer};

/// The grain tile's side, in pixels: drawn at the screen's pixels, never scaled.
const GRAIN: u32 = 64;

/// Inks of a footer: the tag, the name, the fact after a tag, the age.
struct Inks {
    tag: Color,
    name: Color,
    fact: Color,
    age: Color,
}

impl Renderer {
    /// Draws `item`'s card into `shape` (in pixels), laid out by `face` (in DIPs).
    #[allow(clippy::too_many_arguments)]
    pub(in crate::render) fn face(&self, rt: &ID2D1RenderTarget, item: &Item, shape: &D2D1_ROUNDED_RECT, face: &Face, selected: bool, accent: Color, grain: Option<&ID2D1BitmapBrush>) -> Result<()> {
        let p = self.pal();
        // The quietest well: a card is a mat on the island, the content does the talking.
        let fill = if selected { p.fill_tertiary } else { p.fill_quaternary };
        let mut inks = Inks { tag: p.label_secondary, name: p.label_primary, fact: p.label_secondary, age: p.label_secondary.with_alpha(0.8) };
        let well = |c: Color| unsafe {
            rt.FillRoundedRectangle(shape, &self.brush(rt, c)?);
            Ok::<(), windows::core::Error>(())
        };
        let grained = || unsafe {
            if let Some(brush) = grain {
                rt.FillRoundedRectangle(shape, brush);
            }
        };
        // Without a name the footer is one line, and the body runs down to it.
        let one_line = item.name.is_none();
        let body_bottom = if one_line { face.tall_body_bottom() } else { face.body_bottom };
        match &item.preview {
            Preview::Text(text) => {
                well(fill)?;
                self.text_block(rt, text.trim(), &self.fonts.card_text, face.left, face.top, face.width(), body_bottom - face.top, p.label_primary)?;
            }
            Preview::Code(text) => {
                well(fill)?;
                self.text_block(rt, text, &self.fonts.card_code, face.left, face.top, face.width(), body_bottom - face.top, p.label_primary)?;
            }
            Preview::Color(rgb) => {
                let swatch = rgb_color(*rgb);
                well(swatch)?;
                let Rgb(r, g, b) = *rgb;
                let light = 0.2126 * f32::from(r) / 255.0 + 0.7152 * f32::from(g) / 255.0 + 0.0722 * f32::from(b) / 255.0 > 0.55;
                let ink = if light { color::label::ON_ACCENT } else { color::label::PRIMARY };
                inks = Inks { tag: ink.with_alpha(0.7), name: ink, fact: ink.with_alpha(0.7), age: ink.with_alpha(0.55) };
            }
            Preview::Link { host, path, title, site, .. } => {
                well(fill)?;
                self.link(rt, face, host, path, title.as_deref(), site.as_deref(), body_bottom)?;
            }
            Preview::Image(id) => {
                well(fill)?;
                grained();
                match self.image(id) {
                    Some(Decoded::Bitmap(image)) => {
                        let (mut iw, mut ih) = (0u32, 0u32);
                        unsafe { image.GetSize(&mut iw, &mut ih)? };
                        // Its pixels as DIPs: a small image isn't blown up past its size.
                        let (iw, ih) = (iw as f32 / self.scale, ih as f32 / self.scale);
                        let (l, t, w, h) = Face { body_bottom, ..*face }.contain(iw, ih);
                        let r = component::card::THUMB_RADIUS * self.scale;
                        let thumb = D2D1_ROUNDED_RECT { rect: self.rect(l, t, l + w, t + h), radiusX: r, radiusY: r };
                        self.cover(rt, *id, &thumb)?;
                    }
                    // Bytes not here yet: the type's glyph, so the card isn't an empty well.
                    _ => self.body_glyph(rt, face, body_bottom, '\u{EB9F}')?,
                }
                if item.tag_tone == Tone::Accent {
                    inks.tag = accent;
                }
            }
            Preview::Files { thumbs, .. } => {
                well(fill)?;
                grained();
                if !self.fan(rt, face, thumbs)? {
                    self.body_glyph(rt, face, body_bottom, '\u{E8A5}')?;
                }
            }
        }
        self.footer(rt, item, face, &inks)
    }

    /// A link: its site (or host) after the link glyph, the page's title, then the address.
    #[allow(clippy::too_many_arguments)]
    fn link(&self, rt: &ID2D1RenderTarget, face: &Face, host: &str, path: &str, title: Option<&str>, site: Option<&str>, bottom: f32) -> Result<()> {
        let p = self.pal();
        let label = &self.fonts.card_label;
        let line = label.style.line_height;
        let g = icon::STAR + 3.0;
        self.text(rt, "\u{E71B}", &self.fonts.icon_kind, face.left, face.top + (line - g) / 2.0, g, Align::Start, p.label_secondary)?;
        let after = face.left + g + 6.0;
        self.text(rt, site.unwrap_or(host), label, after, face.top, face.right - after, Align::Start, p.label_secondary)?;
        let address = format!("{host}{}", if path == "/" { "" } else { path });
        let title_top = face.top + line + 6.0;
        let title_lines = ((bottom - title_top - line - 2.0) / self.fonts.card_title.style.line_height).floor().clamp(0.0, 2.0);
        let title_h = title_lines * self.fonts.card_title.style.line_height;
        match title {
            Some(title) => {
                self.text_block(rt, title, &self.fonts.card_title, face.left, title_top, face.width(), title_h, p.label_primary)?;
                let lines = self.lines(title, &self.fonts.card_title, face.width())?.clamp(1.0, title_lines.max(1.0));
                let url_top = title_top + lines * self.fonts.card_title.style.line_height + 2.0;
                if url_top + line <= bottom {
                    self.text(rt, &address, label, face.left, url_top, face.width(), Align::Start, p.label_tertiary)?;
                }
            }
            // No title (yet, or none): the address is the heading.
            None => self.text_block(rt, &address, &self.fonts.card_title, face.left, title_top, face.width(), title_h, p.label_primary)?,
        }
        Ok(())
    }

    /// The files' pictures fanned out in the body (`Face::fan`), each with a hairline so
    /// overlapping ones keep their edges; `false` when none has arrived yet.
    fn fan(&self, rt: &ID2D1RenderTarget, face: &Face, thumbs: &[ImageId]) -> Result<bool> {
        let mut shown = Vec::new();
        for id in thumbs {
            if let Some(Decoded::Bitmap(image)) = self.image(id) {
                let (mut w, mut h) = (0u32, 0u32);
                unsafe { image.GetSize(&mut w, &mut h)? };
                shown.push((*id, (w as f32 / self.scale, h as f32 / self.scale)));
            }
        }
        if shown.is_empty() {
            return Ok(false);
        }
        let sizes: Vec<(f32, f32)> = shown.iter().map(|s| s.1).collect();
        let placed = face.fan(&sizes);
        let ids: Vec<ImageId> = shown.iter().rev().map(|s| s.0).collect();
        let s = self.scale;
        let r = component::card::THUMB_RADIUS * 0.75 * s;
        let mut before = Matrix3x2::default();
        unsafe { rt.GetTransform(&mut before) };
        let edge = self.brush(rt, self.pal().label_primary.with_alpha(opacity::ART_STROKE))?;
        for (p, id) in placed.iter().zip(ids) {
            let (cx, cy) = ((p.left + p.width / 2.0) * s, (p.top + p.height / 2.0) * s);
            unsafe { rt.SetTransform(&(Matrix3x2::rotation_around(p.angle, windows_numerics::Vector2::new(cx, cy)) * before)) };
            let shape = D2D1_ROUNDED_RECT { rect: self.rect(p.left, p.top, p.left + p.width, p.top + p.height), radiusX: r, radiusY: r };
            // Behind a picture with transparency (a type's icon), so it reads as a sheet.
            unsafe { rt.FillRoundedRectangle(&shape, &self.brush(rt, self.pal().background)?) };
            self.cover(rt, id, &shape)?;
            unsafe { rt.DrawRoundedRectangle(&shape, &edge, 1.0, None) };
        }
        unsafe { rt.SetTransform(&before) };
        Ok(true)
    }

    /// A quiet glyph in the middle of the body, standing in for media not shown.
    fn body_glyph(&self, rt: &ID2D1RenderTarget, face: &Face, bottom: f32, glyph: char) -> Result<()> {
        let g = icon::TILE;
        let top = face.top + (bottom - face.top - g) / 2.0;
        self.text(rt, &glyph.to_string(), &self.fonts.icon_tile, face.left, top, face.width(), Align::Center, self.pal().label_tertiary)
    }

    /// The footer: the tag over the name with the age at its right; or, without a name,
    /// the tag, " · " and the fact, and the age, on one line. A pinned card's star follows
    /// the tag.
    fn footer(&self, rt: &ID2D1RenderTarget, item: &Item, face: &Face, inks: &Inks) -> Result<()> {
        let age_font = &self.fonts.card_age;
        let age_w = self.text_width(&item.age, age_font)?.min(face.width() * 0.5);
        let name_line = self.fonts.card_name.style.line_height;
        let age_top = face.name_top + (name_line - age_font.style.line_height) / 2.0;
        self.text(rt, &item.age, age_font, face.right - age_w, age_top, age_w + 1.0, Align::End, inks.age)?;
        let room = (face.right - age_w - component::card::PADDING * 0.5 - face.left).max(0.0);
        let star = |x: f32, top: f32, line: f32| -> Result<()> {
            let s = icon::STAR;
            self.text(rt, "\u{E735}", &self.fonts.icon_star, x, top + (line - s) / 2.0, s, Align::Center, inks.tag)
        };
        match &item.name {
            Some(name) => {
                let tag_font = &self.fonts.card_tag;
                let mut x = face.left;
                if let Some(tag) = &item.tag {
                    let tw = self.text_width(tag, tag_font)?.min(face.width());
                    self.text(rt, tag, tag_font, x, face.tag_top, tw + 1.0, Align::Start, inks.tag)?;
                    x += tw + 4.0;
                }
                if item.pinned {
                    star(x, face.tag_top, tag_font.style.line_height)?;
                }
                self.text(rt, name, &self.fonts.card_name, face.left, face.name_top, room, Align::Start, inks.name)?;
            }
            None => {
                let font = &self.fonts.card_label;
                let top = face.name_top + (name_line - font.style.line_height) / 2.0;
                let mut x = face.left;
                if let Some(tag) = &item.tag {
                    let tw = self.text_width(tag, font)?.min(room);
                    // Alone, the tag is quiet; before a fact it reads as the label of it.
                    let ink = if item.meta.is_some() { inks.name } else { inks.tag };
                    self.text(rt, tag, font, x, top, tw + 1.0, Align::Start, ink)?;
                    x += tw;
                }
                if item.pinned {
                    star(x + 4.0, top, font.style.line_height)?;
                    x += 4.0 + icon::STAR;
                }
                if let Some(meta) = &item.meta {
                    let text = if item.tag.is_some() { format!(" \u{00B7} {meta}") } else { meta.clone() };
                    let end = face.left + room;
                    if end > x {
                        self.text(rt, &text, font, x, top, end - x, Align::Start, inks.fact)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// The grain tile as a brush repeating at the screen's pixels, faint; `None` where the
    /// tile can't be made (the cards are simply flat).
    pub(in crate::render) fn grain(&self, rt: &ID2D1RenderTarget) -> Result<Option<ID2D1BitmapBrush>> {
        let pixels = noise(GRAIN);
        let props = D2D1_BITMAP_PROPERTIES { pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED }, dpiX: 96.0, dpiY: 96.0 };
        unsafe {
            let Ok(bitmap) = rt.CreateBitmap(D2D_SIZE_U { width: GRAIN, height: GRAIN }, Some(pixels.as_ptr().cast()), GRAIN * 4, &props) else { return Ok(None) };
            let brush = rt.CreateBitmapBrush(
                &bitmap,
                Some(&D2D1_BITMAP_BRUSH_PROPERTIES { extendModeX: D2D1_EXTEND_MODE_WRAP, extendModeY: D2D1_EXTEND_MODE_WRAP, interpolationMode: D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR }),
                None,
            )?;
            brush.SetOpacity(component::card::GRAIN);
            brush.SetTransform(&Matrix3x2::identity());
            Ok(Some(brush))
        }
    }

    /// How many lines `text` wraps to at `width`.
    fn lines(&self, text: &str, font: &crate::render::fonts::Font, width: f32) -> Result<f32> {
        let full = self.text_width(text, font)?;
        Ok((full / width.max(1.0)).ceil().max(1.0))
    }
}

/// A `side` × `side` tile of grey noise, opaque, premultiplied BGRA: the same every time
/// (a fixed seed), so cards don't shimmer between redraws.
fn noise(side: u32) -> Vec<u8> {
    let mut state: u32 = 0x9E37_79B9;
    let mut out = Vec::with_capacity((side * side * 4) as usize);
    for _ in 0..side * side {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        let v = (state >> 24) as u8;
        out.extend_from_slice(&[v, v, v, 255]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_grain_is_opaque_grey_and_the_same_every_time() {
        let a = noise(8);
        assert_eq!(a.len(), 8 * 8 * 4);
        assert_eq!(a, noise(8));
        assert!(a.chunks(4).all(|p| p[0] == p[1] && p[1] == p[2] && p[3] == 255));
        // Noise, not a flat colour.
        let distinct: std::collections::HashSet<u8> = a.chunks(4).map(|p| p[0]).collect();
        assert!(distinct.len() > 16);
    }
}
