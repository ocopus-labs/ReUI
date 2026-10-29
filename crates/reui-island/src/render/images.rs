//! Images: decoding what the hub sends (WIC bitmaps, SVG kept as text), and drawing art,
//! covers, scrims and tinted SVG marks.

use std::collections::HashSet;

use reui_design::tokens::{component, opacity};
use reui_design::Color;
use reui_protocol::{Art, ImageId};
use windows::core::{Interface, Result};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::UI::Shell::SHCreateMemStream;
use windows_numerics::{Matrix3x2, Vector2};

use crate::render::{d2d, rgb, Align, Renderer, Slot};

/// Longest side images are kept at after decoding: enough for a 56 DIP tile at 4x.
const IMAGE_MAX_PX: u32 = 256;

pub(super) enum Decoded {
    Bitmap(IWICBitmap),
    /// Kept as text: it's re-tinted per draw by setting the root `fill`.
    Svg(String),
}

impl Renderer {
    /// Decodes image bytes once: SVG is kept as text, anything else is decoded with WIC and
    /// downscaled so the short side is at most `IMAGE_MAX_PX`.
    pub fn add_image(&mut self, id: ImageId, bytes: &[u8]) -> Result<()> {
        let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]);
        if head.contains("<svg") {
            self.images.insert(id, Decoded::Svg(fit_svg(&String::from_utf8_lossy(bytes))));
            return Ok(());
        }
        unsafe {
            let stream = self.wic.CreateStream()?;
            stream.InitializeFromMemory(bytes)?;
            let decoder = self.wic.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)?;
            let frame = decoder.GetFrame(0)?;
            let (mut w, mut h) = (0u32, 0u32);
            frame.GetSize(&mut w, &mut h)?;
            let k = (IMAGE_MAX_PX as f32 / w.min(h).max(1) as f32).min(1.0);
            let scaler = self.wic.CreateBitmapScaler()?;
            scaler.Initialize(&frame, ((w as f32 * k) as u32).max(1), ((h as f32 * k) as u32).max(1), WICBitmapInterpolationModeFant)?;
            let converter = self.wic.CreateFormatConverter()?;
            converter.Initialize(&scaler, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeMedianCut)?;
            let bitmap = self.wic.CreateBitmapFromSource(&converter, WICBitmapCacheOnLoad)?;
            self.images.insert(id, Decoded::Bitmap(bitmap));
        }
        Ok(())
    }

    /// Drops decoded images the latest snapshot no longer shows; the hub forgets having
    /// sent them by the same rule (`clipboard::referenced`), so it sends them again if needed.
    pub fn has_image(&self, id: ImageId) -> bool {
        self.images.contains_key(&id)
    }

    pub fn retain_images(&mut self, keep: &HashSet<ImageId>) {
        self.images.retain(|id, _| keep.contains(id));
    }

    pub(super) fn art(&self, rt: &ID2D1RenderTarget, art: &Art, slot: Slot, left: f32, top: f32, accent: Color) -> Result<()> {
        let (size, corner) = self.art_shape(slot);
        let r = self.rect(left, top, left + size, top + size);
        let shape = D2D1_ROUNDED_RECT { rect: r, radiusX: corner * self.scale, radiusY: corner * self.scale };
        unsafe {
            match art {
                Art::Gradient { from, to } => {
                    let stops = [
                        D2D1_GRADIENT_STOP { position: 0.0, color: d2d(rgb(*from)) },
                        D2D1_GRADIENT_STOP { position: 1.0, color: d2d(rgb(*to)) },
                    ];
                    let collection = rt.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)?;
                    let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: Vector2::new(r.left, r.top), endPoint: Vector2::new(r.right, r.bottom) };
                    rt.FillRoundedRectangle(&shape, &rt.CreateLinearGradientBrush(&props, None, &collection)?);
                }
                Art::Glyph(glyph) => {
                    rt.FillRoundedRectangle(&shape, &self.brush(rt, accent.with_alpha(opacity::TINT_FILL))?);
                    let font = if matches!(slot, Slot::Compact | Slot::Minimal) { &self.fonts.icon_tile_compact } else { &self.fonts.icon_tile };
                    let g = font.style.size;
                    self.text(rt, &glyph.to_string(), font, left, top + (size - g) / 2.0, size, Align::Center, accent)?;
                }
                Art::Logo { image, animated } => {
                    // The mark alone, no tile: its own colours (Claude's orange), else the
                    // primary label colour (OpenAI, OpenCode). An animated compact logo is
                    // drawn by the pill so it can spin.
                    if !(*animated && slot == Slot::Compact) {
                        let mark = size * opacity::LOGO_BARE;
                        self.svg(rt, *image, left + (size - mark) / 2.0, top + (size - mark) / 2.0, mark, self.pal().label_primary)?;
                    }
                }
                Art::Image(id) => match self.images.get(id) {
                    Some(Decoded::Bitmap(_)) => {
                        self.cover(rt, *id, &shape)?;
                        // A hairline just inside the edge, so a cover on black still reads
                        // as a tile on the black island.
                        let inner = D2D1_ROUNDED_RECT {
                            rect: D2D_RECT_F { left: r.left + 0.5, top: r.top + 0.5, right: r.right - 0.5, bottom: r.bottom - 0.5 },
                            radiusX: (shape.radiusX - 0.5).max(0.0),
                            radiusY: (shape.radiusY - 0.5).max(0.0),
                        };
                        rt.DrawRoundedRectangle(&inner, &self.brush(rt, self.pal().label_primary.with_alpha(opacity::ART_STROKE))?, 1.0, None);
                    }
                    Some(Decoded::Svg(_)) => {
                        rt.FillRoundedRectangle(&shape, &self.brush(rt, self.pal().fill_tertiary)?);
                        let mark = size * opacity::LOGO;
                        self.svg(rt, *id, left + (size - mark) / 2.0, top + (size - mark) / 2.0, mark, self.pal().label_primary)?;
                    }
                    // Not arrived yet (or undecodable): a quiet placeholder of the same shape.
                    None => rt.FillRoundedRectangle(&shape, &self.brush(rt, self.pal().fill_tertiary)?),
                },
            }
        }
        Ok(())
    }

    /// Fills `shape` with an image, cover-fitted (centre-cropped to the shape's aspect).
    pub(super) fn cover(&self, rt: &ID2D1RenderTarget, id: ImageId, shape: &D2D1_ROUNDED_RECT) -> Result<()> {
        let Some(Decoded::Bitmap(image)) = self.images.get(&id) else { return Ok(()) };
        let r = shape.rect;
        unsafe {
            let bitmap = rt.CreateBitmapFromWicBitmap(image, None)?;
            let D2D_SIZE_F { width: iw, height: ih } = bitmap.GetSize();
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            let k = (w / iw).max(h / ih);
            let brush = rt.CreateBitmapBrush(
                &bitmap,
                Some(&D2D1_BITMAP_BRUSH_PROPERTIES {
                    extendModeX: D2D1_EXTEND_MODE_CLAMP,
                    extendModeY: D2D1_EXTEND_MODE_CLAMP,
                    interpolationMode: D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                }),
                None,
            )?;
            brush.SetTransform(&Matrix3x2 { M11: k, M12: 0.0, M21: 0.0, M22: k, M31: r.left + (w - iw * k) / 2.0, M32: r.top + (h - ih * k) / 2.0 });
            rt.FillRoundedRectangle(shape, &brush);
        }
        Ok(())
    }

    /// A dark gradient over the bottom of a card so the footer reads on any image.
    pub(super) fn scrim(&self, rt: &ID2D1RenderTarget, shape: &D2D1_ROUNDED_RECT, from: f32, to: f32) -> Result<()> {
        let s = self.scale;
        unsafe {
            let stops = [
                D2D1_GRADIENT_STOP { position: 0.0, color: D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 } },
                D2D1_GRADIENT_STOP { position: 1.0, color: D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.7 } },
            ];
            let collection = rt.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)?;
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: Vector2::new(0.0, from * s), endPoint: Vector2::new(0.0, to * s) };
            let brush = rt.CreateLinearGradientBrush(&props, None, &collection)?;
            rt.PushAxisAlignedClip(&D2D_RECT_F { left: shape.rect.left, top: from * s, right: shape.rect.right, bottom: shape.rect.bottom }, D2D1_ANTIALIAS_MODE_ALIASED);
            rt.FillRoundedRectangle(shape, &brush);
            rt.PopAxisAlignedClip();
        }
        Ok(())
    }

    /// Draws a single-colour SVG mark into a `size` square at (`left`, `top`), filled with
    /// `color`. Missing or non-SVG images draw nothing.
    pub(super) fn svg(&self, rt: &ID2D1RenderTarget, image: ImageId, left: f32, top: f32, size: f32, color: Color) -> Result<()> {
        let Some(Decoded::Svg(text)) = self.images.get(&image) else { return Ok(()) };
        let [r, g, b, _] = color.to_rgba8();
        let tinted = text.replacen("<svg", &format!("<svg fill=\"#{r:02X}{g:02X}{b:02X}\" fill-opacity=\"{}\"", color.a), 1);
        let s = self.scale;
        unsafe {
            let Some(stream) = SHCreateMemStream(Some(tinted.as_bytes())) else { return Ok(()) };
            let dc: ID2D1DeviceContext5 = rt.cast()?;
            let document = dc.CreateSvgDocument(&stream, D2D_SIZE_F { width: size * s, height: size * s })?;
            let mut previous = Matrix3x2::default();
            rt.GetTransform(&mut previous);
            rt.SetTransform(&Matrix3x2 { M31: previous.M31 + left * s, M32: previous.M32 + top * s, ..previous });
            dc.DrawSvgDocument(&document);
            rt.SetTransform(&previous);
        }
        Ok(())
    }
}

/// Drops `width`/`height` from the root `<svg>` tag so the document fills whatever viewport
/// it's drawn into (icon sets often say `width="1em"`), scaling by its `viewBox`.
fn fit_svg(text: &str) -> String {
    let Some(start) = text.find("<svg") else { return text.to_string() };
    let Some(len) = text[start..].find('>') else { return text.to_string() };
    let tag = &text[start..start + len];
    let mut fitted = tag.to_string();
    for attr in [" width=", " height="] {
        if let Some(at) = fitted.find(attr) {
            let value_start = at + attr.len();
            let quote = fitted[value_start..].chars().next().unwrap_or('"');
            if let Some(end) = fitted[value_start + 1..].find(quote) {
                fitted.replace_range(at..value_start + 1 + end + 1, "");
            }
        }
    }
    format!("{}{}{}", &text[..start], fitted, &text[start + len..])
}

impl Renderer {
    /// (size, corner radius), concentric with the edge the art sits against: the compact
    /// capsule's or bubble's end (a circle at full roundness), the open card's corner in the
    /// leading slot, its side in the trailing slot (a circle while that allows).
    pub(super) fn art_shape(&self, slot: Slot) -> (f32, f32) {
        match slot {
            Slot::Compact | Slot::Minimal => (component::art::COMPACT, reui_design::layout::compact::art_corner(&self.look)),
            Slot::Leading => (component::art::EXPANDED, self.concentric()),
            Slot::Trailing => (component::art::TRAILING, self.capsule(component::art::TRAILING, self.look.inset())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fit_svg;

    #[test]
    fn svg_root_size_is_dropped_so_it_fills_its_slot() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="1em" height='1em' viewBox="0 0 128 128"><path width="5"/></svg>"#;
        assert_eq!(fit_svg(svg), r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 128 128"><path width="5"/></svg>"#);
        assert_eq!(fit_svg("<svg viewBox=\"0 0 24 24\"/>"), "<svg viewBox=\"0 0 24 24\"/>");
    }
}
