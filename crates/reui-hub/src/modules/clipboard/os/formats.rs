//! PNG and DIB byte formats: header sizes, alpha detection, 32 bpp DIB decoding and the
//! DIBs written for a paste-back.

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

pub(super) fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    if png.get(..8)? != PNG_SIGNATURE || png.get(12..16)? != b"IHDR" {
        return None;
    }
    let be = |at: usize| png.get(at..at + 4).map(|b| u32::from_be_bytes(b.try_into().unwrap()));
    Some((be(16)?, be(20)?))
}

/// PNG chunks (type, offset of the chunk) up to and including IEND.
fn png_chunks(png: &[u8]) -> impl Iterator<Item = (&[u8], usize)> {
    let mut at = PNG_SIGNATURE.len();
    let mut done = false;
    std::iter::from_fn(move || {
        if done {
            return None;
        }
        let head = png.get(at..at + 8)?;
        let len = u32::from_be_bytes(head[0..4].try_into().unwrap()) as usize;
        let chunk = (&head[4..8], at);
        done = chunk.0 == b"IEND";
        at = at.saturating_add(12).saturating_add(len);
        Some(chunk)
    })
}

/// Length of the PNG file in a (padded) allocation.
pub(super) fn png_end(png: &[u8]) -> Option<usize> {
    png_chunks(png).find(|(kind, _)| *kind == b"IEND").map(|(_, at)| at + 12).filter(|&end| end <= png.len())
}

/// Grey+alpha or RGBA, or a tRNS chunk (palette or colour-key transparency).
pub(super) fn png_has_alpha(png: &[u8]) -> bool {
    if matches!(png.get(25), Some(4 | 6)) {
        return true;
    }
    png_chunks(png).take_while(|(kind, _)| *kind != b"IDAT").any(|(kind, _)| kind == b"tRNS")
}

/// Width and height from a DIB header (BITMAPCOREHEADER or BITMAPINFOHEADER and later).
pub(super) fn dib_size(dib: &[u8]) -> Option<(u64, u64)> {
    let header_size = u32::from_le_bytes(dib.get(0..4)?.try_into().ok()?);
    if header_size == 12 {
        let w = u16::from_le_bytes(dib.get(4..6)?.try_into().ok()?);
        let h = u16::from_le_bytes(dib.get(6..8)?.try_into().ok()?);
        return Some((u64::from(w), u64::from(h)));
    }
    if header_size < 40 {
        return None;
    }
    let w = i32::from_le_bytes(dib.get(4..8)?.try_into().ok()?);
    let h = i32::from_le_bytes(dib.get(8..12)?.try_into().ok()?);
    Some((u64::from(w.unsigned_abs()), u64::from(h.unsigned_abs())))
}

/// A 32-bit DIB's pixels as top-down straight BGRA.
#[derive(Debug, PartialEq)]
pub(super) struct Pixels {
    pub(super) w: u32,
    pub(super) h: u32,
    pub(super) bgra: Vec<u8>,
    /// Some pixel had a non-zero alpha. Most apps leave the fourth byte at 0 ("reserved"),
    /// so an all-zero alpha means opaque, and `bgra` then carries 255 there.
    pub(super) alpha: bool,
}

/// Decodes a 32 bpp DIB (BI_RGB, BI_BITFIELDS or BI_ALPHABITFIELDS, any header version);
/// `None` for other bit depths, which go through WIC's BMP decoder instead (no alpha).
pub(super) fn dib_pixels(dib: &[u8]) -> Option<Pixels> {
    let u32_at = |at: usize| dib.get(at..at + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    let header = u32_at(0)? as usize;
    if header < 40 || u16::from_le_bytes(dib.get(14..16)?.try_into().ok()?) != 32 {
        return None;
    }
    let (w, h) = (u32_at(4)? as i32, u32_at(8)? as i32);
    let (top_down, w, h) = (h < 0, w.unsigned_abs(), h.unsigned_abs());
    if w == 0 || h == 0 {
        return None;
    }
    // Masks live in the header from V2 (52 bytes) on, else right after a 40-byte header.
    let (mut masks, extra) = match u32_at(16)? {
        0 => ([0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000], 0),
        3 | 6 if header >= 52 => ([u32_at(40)?, u32_at(44)?, u32_at(48)?, if header >= 56 { u32_at(52)? } else { 0 }], 0),
        3 => ([u32_at(40)?, u32_at(44)?, u32_at(48)?, 0], 12),
        6 => ([u32_at(40)?, u32_at(44)?, u32_at(48)?, u32_at(52)?], 16),
        _ => return None,
    };
    if masks[3] == 0 {
        // No alpha mask given: the bits the colours don't use (the "reserved" byte).
        masks[3] = !(masks[0] | masks[1] | masks[2]);
    }
    let offset = header + extra + u32_at(32)? as usize * 4;
    let stride = w as usize * 4;
    let pixels = dib.get(offset..offset.checked_add(stride.checked_mul(h as usize)?)?)?;
    let channel = |px: u32, mask: u32| -> u8 {
        if mask == 0 {
            return 0;
        }
        let shift = mask.trailing_zeros();
        let (value, max) = (u64::from((px & mask) >> shift), u64::from(mask >> shift));
        ((value * 255 + max / 2) / max) as u8
    };
    let mut bgra = Vec::with_capacity(stride * h as usize);
    let mut alpha = false;
    for row in 0..h as usize {
        let src = if top_down { row } else { h as usize - 1 - row };
        for px in pixels[src * stride..(src + 1) * stride].chunks_exact(4) {
            let px = u32::from_le_bytes(px.try_into().unwrap());
            let a = channel(px, masks[3]);
            alpha |= a != 0;
            bgra.extend_from_slice(&[channel(px, masks[2]), channel(px, masks[1]), channel(px, masks[0]), a]);
        }
    }
    if !alpha {
        bgra.chunks_exact_mut(4).for_each(|px| px[3] = 255);
    }
    Some(Pixels { w, h, bgra, alpha })
}

/// Top-down straight BGRA → a bottom-up 32 bpp DIB. `v5`: CF_DIBV5 with an alpha mask (sRGB);
/// else CF_DIB, flattened onto white with the reserved byte 0, for apps that ignore alpha.
pub(super) fn dib_from_bgra(w: u32, h: u32, bgra: &[u8], v5: bool) -> Vec<u8> {
    let stride = w as usize * 4;
    let mut dib = Vec::with_capacity(124 + bgra.len());
    let put = |dib: &mut Vec<u8>, values: &[u32]| values.iter().for_each(|v| dib.extend_from_slice(&v.to_le_bytes()));
    // Size, width, height (positive: bottom-up), planes 1 | bit count 32, compression, image
    // size, pixels per metre ×2, colours used, colours important.
    put(&mut dib, &[if v5 { 124 } else { 40 }, w, h, 1 | (32 << 16), if v5 { 3 } else { 0 }, (stride * h as usize) as u32, 0, 0, 0, 0]);
    if v5 {
        // RGBA masks, LCS_sRGB, 36 bytes of endpoints + 3 gammas, LCS_GM_IMAGES, profile ×2, reserved.
        put(&mut dib, &[0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000, 0x7352_4742]);
        put(&mut dib, &[0; 12]);
        put(&mut dib, &[4, 0, 0, 0]);
    }
    for row in bgra.chunks_exact(stride).rev() {
        if v5 {
            dib.extend_from_slice(row);
        } else {
            for px in row.chunks_exact(4) {
                let a = u32::from(px[3]);
                let over_white = |c: u8| ((u32::from(c) * a + 255 * (255 - a) + 127) / 255) as u8;
                dib.extend_from_slice(&[over_white(px[0]), over_white(px[1]), over_white(px[2]), 0]);
            }
        }
    }
    dib
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dib_sizes_from_headers() {
        let mut info = vec![0u8; 40];
        info[0..4].copy_from_slice(&40u32.to_le_bytes());
        info[4..8].copy_from_slice(&7680i32.to_le_bytes());
        info[8..12].copy_from_slice(&(-4320i32).to_le_bytes());
        assert_eq!(dib_size(&info), Some((7680, 4320)));
        let mut core = vec![0u8; 12];
        core[0..4].copy_from_slice(&12u32.to_le_bytes());
        core[4..6].copy_from_slice(&10u16.to_le_bytes());
        core[6..8].copy_from_slice(&20u16.to_le_bytes());
        assert_eq!(dib_size(&core), Some((10, 20)));
        assert_eq!(dib_size(&[0; 8]), None);
    }

    fn png_with(color_type: u8, extra: &[&[u8; 4]]) -> Vec<u8> {
        let mut png = PNG_SIGNATURE.to_vec();
        let mut chunk = |kind: &[u8; 4], data: &[u8]| {
            png.extend_from_slice(&(data.len() as u32).to_be_bytes());
            png.extend_from_slice(kind);
            png.extend_from_slice(data);
            png.extend_from_slice(&[0; 4]);
        };
        let mut ihdr = [0u8; 13];
        ihdr[0..4].copy_from_slice(&640u32.to_be_bytes());
        ihdr[4..8].copy_from_slice(&480u32.to_be_bytes());
        ihdr[8] = 8;
        ihdr[9] = color_type;
        chunk(b"IHDR", &ihdr);
        for kind in extra {
            chunk(kind, &[1, 2, 3]);
        }
        chunk(b"IDAT", &[0; 5]);
        chunk(b"IEND", &[]);
        png
    }

    #[test]
    fn png_headers() {
        let rgba = png_with(6, &[]);
        assert_eq!(png_size(&rgba), Some((640, 480)));
        assert!(png_has_alpha(&rgba));
        assert!(!png_has_alpha(&png_with(2, &[])));
        assert!(png_has_alpha(&png_with(3, &[b"PLTE", b"tRNS"])));
        let mut padded = rgba.clone();
        padded.extend_from_slice(&[0; 9]);
        assert_eq!(png_end(&padded), Some(rgba.len()));
        assert_eq!(png_size(b"GIF89a.........................."), None);
    }

    /// A 2×2 bottom-up 32 bpp DIB with a 40-byte header.
    fn dib40(compression: u32, masks: &[u32], pixels: [u32; 4]) -> Vec<u8> {
        let mut dib = Vec::new();
        for v in [40, 2, 2, 1 | (32 << 16), compression, 16, 0, 0, 0, 0].iter().chain(masks).chain(&pixels) {
            dib.extend_from_slice(&v.to_le_bytes());
        }
        dib
    }

    #[test]
    fn dib_alpha_detection() {
        // Reserved byte all zero: opaque, alpha forced to 255. Rows are bottom-up.
        let opaque = dib_pixels(&dib40(0, &[], [0x00112233, 0x00445566, 0x00778899, 0x00AABBCC])).unwrap();
        assert!(!opaque.alpha);
        assert_eq!(&opaque.bgra[..8], &[0x99, 0x88, 0x77, 255, 0xCC, 0xBB, 0xAA, 255]);
        // One non-zero alpha anywhere: honoured everywhere.
        let alpha = dib_pixels(&dib40(0, &[], [0x00112233, 0x80445566, 0, 0])).unwrap();
        assert!(alpha.alpha);
        assert_eq!(&alpha.bgra[8..16], &[0x33, 0x22, 0x11, 0, 0x66, 0x55, 0x44, 0x80]);
        // BI_BITFIELDS with swapped red/blue masks after a 40-byte header.
        let swapped = dib_pixels(&dib40(3, &[0x0000_00FF, 0x0000_FF00, 0x00FF_0000], [0x00112233, 0, 0, 0])).unwrap();
        assert_eq!(&swapped.bgra[8..12], &[0x11, 0x22, 0x33, 255]);
        // Not 32 bpp: left to WIC.
        let mut dib24 = dib40(0, &[], [0; 4]);
        dib24[14] = 24;
        assert_eq!(dib_pixels(&dib24), None);
    }

    #[test]
    fn dibv5_round_trips_alpha() {
        let bgra: Vec<u8> = vec![10, 20, 30, 255, 40, 50, 60, 128, 70, 80, 90, 0, 1, 2, 3, 64, 5, 6, 7, 255, 8, 9, 10, 32];
        let v5 = dib_from_bgra(3, 2, &bgra, true);
        assert_eq!(v5.len(), 124 + 24);
        assert_eq!(dib_size(&v5), Some((3, 2)));
        assert_eq!(dib_pixels(&v5), Some(Pixels { w: 3, h: 2, bgra: bgra.clone(), alpha: true }));
        // CF_DIB: flattened onto white, read back as opaque.
        let flat = dib_pixels(&dib_from_bgra(3, 2, &bgra, false)).unwrap();
        assert!(!flat.alpha);
        assert_eq!(&flat.bgra[..12], &[10, 20, 30, 255, 147, 152, 157, 255, 255, 255, 255, 255]);
    }
}
