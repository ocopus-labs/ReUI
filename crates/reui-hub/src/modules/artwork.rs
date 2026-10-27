//! Artwork for Now Playing: covers scaled for sending, video frames trimmed of their
//! letterbox bars, and an accent colour (the most vivid hue in the image, not its usually
//! muddy average). WIC-based and blocking: call from a blocking thread.

use reui_protocol::Rgb;
use windows::core::Result;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};

const HUE_BUCKETS: usize = 12;

/// Near-black for letterbox purposes (JPEG noise included).
const BAR_LEVEL: u8 = 28;
/// A bar row may have this share of brighter pixels (a logo, compression ringing).
const BAR_OUTLIERS: f32 = 0.02;

/// A cover ready to send.
pub struct Prepared {
    pub png: Vec<u8>,
    pub accent: Option<Rgb>,
}

/// Any encoded image → a PNG no larger than `max_px`, cropped to its content when it's a
/// letterboxed video frame, with its accent taken from what's left (bars would otherwise
/// pull it towards nothing). Initialises COM on the calling thread.
pub fn prepare(bytes: &[u8], max_px: u32) -> Option<Prepared> {
    let png = super::clipboard_os::thumbnail(bytes, max_px)?;
    let Ok((w, h, pixels)) = rgb(&png) else { return Some(Prepared { png, accent: None }) };
    let (x, y, cw, ch) = content_bounds(&pixels, w, h);
    if (cw, ch) == (w, h) {
        return Some(Prepared { accent: vivid(&pixels), png });
    }
    let cropped: Vec<[u8; 3]> = (y..y + ch).flat_map(|row| pixels[(row * w + x) as usize..(row * w + x + cw) as usize].iter().copied()).collect();
    let png = super::clipboard_os::thumbnail(&bmp(cw, ch, &cropped), max_px).unwrap_or(png);
    Some(Prepared { accent: vivid(&cropped), png })
}

/// The image at full size as straight RGB pixels, row by row.
fn rgb(bytes: &[u8]) -> Result<(u32, u32, Vec<[u8; 3]>)> {
    unsafe {
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let stream = wic.CreateStream()?;
        stream.InitializeFromMemory(bytes)?;
        let decoder = wic.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)?;
        let converter = wic.CreateFormatConverter()?;
        converter.Initialize(&decoder.GetFrame(0)?, &GUID_WICPixelFormat24bppRGB, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeMedianCut)?;
        let (mut w, mut h) = (0, 0);
        converter.GetSize(&mut w, &mut h)?;
        let mut buffer = vec![0u8; (w * h * 3) as usize];
        converter.CopyPixels(std::ptr::null(), w * 3, &mut buffer)?;
        Ok((w, h, buffer.chunks_exact(3).map(|p| [p[0], p[1], p[2]]).collect()))
    }
}

/// `(x, y, width, height)` of the picture inside letterbox / pillarbox bars: uniform
/// near-black rows (columns) trimmed from both edges. Bars come in pairs of about the same
/// size, so a cover that's simply dark along one edge is left alone, and so is an image
/// that's nearly all dark.
pub fn content_bounds(pixels: &[[u8; 3]], w: u32, h: u32) -> (u32, u32, u32, u32) {
    let full = (0, 0, w, h);
    if w == 0 || h == 0 || pixels.len() < (w * h) as usize {
        return full;
    }
    let dark = |p: &[u8; 3]| p.iter().all(|&c| c <= BAR_LEVEL);
    let bar = |count: u32, bright: usize| bright as f32 <= count as f32 * BAR_OUTLIERS;
    let row_bar = |y: u32| bar(w, (0..w).filter(|&x| !dark(&pixels[(y * w + x) as usize])).count());
    let top = (0..h).take_while(|&y| row_bar(y)).count() as u32;
    let bottom = (0..h).rev().take_while(|&y| row_bar(y)).count() as u32;
    let (y, ch) = trimmed(top, bottom, h);
    let col_bar = |x: u32| bar(ch, (y..y + ch).filter(|&row| !dark(&pixels[(row * w + x) as usize])).count());
    let left = (0..w).take_while(|&x| col_bar(x)).count() as u32;
    let right = (0..w).rev().take_while(|&x| col_bar(x)).count() as u32;
    let (x, cw) = trimmed(left, right, w);
    (x, y, cw, ch)
}

/// `(start, length)` after trimming a pair of bars, or the whole span if they aren't one.
fn trimmed(a: u32, b: u32, size: u32) -> (u32, u32) {
    let paired = a > 0 && b > 0 && a.abs_diff(b) <= 3.max(a.max(b) / 3);
    let left = size.saturating_sub(a + b);
    if paired && left >= size / 4 {
        (a, left)
    } else {
        (0, size)
    }
}

/// A 24-bit BMP of RGB pixels, so WIC can scale and re-encode a crop.
fn bmp(w: u32, h: u32, pixels: &[[u8; 3]]) -> Vec<u8> {
    let stride = (w * 3).div_ceil(4) * 4;
    let size = 54 + stride * h;
    let mut out = Vec::with_capacity(size as usize);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(-(h as i32)).to_le_bytes()); // top-down
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]);
    for row in pixels.chunks_exact(w as usize) {
        let start = out.len();
        out.extend(row.iter().flat_map(|&[r, g, b]| [b, g, r]));
        out.resize(start + stride as usize, 0);
    }
    out
}

/// Buckets pixels by hue, weighting each by saturation × brightness (greys and near-blacks
/// count for nothing), and averages the winning bucket. Lifts it so it reads on black.
fn vivid(pixels: &[[u8; 3]]) -> Option<Rgb> {
    let mut buckets = [(0.0f32, [0.0f32; 3]); HUE_BUCKETS];
    for &[r, g, b] in pixels {
        let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let chroma = max - min;
        if chroma < 0.12 || max < 0.2 {
            continue;
        }
        let hue = if max == r {
            ((g - b) / chroma).rem_euclid(6.0)
        } else if max == g {
            (b - r) / chroma + 2.0
        } else {
            (r - g) / chroma + 4.0
        } / 6.0;
        let weight = (chroma / max) * max * chroma;
        let bucket = &mut buckets[((hue * HUE_BUCKETS as f32) as usize).min(HUE_BUCKETS - 1)];
        bucket.0 += weight;
        bucket.1[0] += r * weight;
        bucket.1[1] += g * weight;
        bucket.1[2] += b * weight;
    }
    let (weight, sum) = buckets.into_iter().max_by(|a, b| a.0.total_cmp(&b.0))?;
    if weight <= 0.0 {
        return None;
    }
    let mut colour = sum.map(|c| c / weight);
    // Accents sit on black: make sure the brightest channel is at least 0.75.
    let max = colour[0].max(colour[1]).max(colour[2]);
    if max < 0.75 {
        colour = colour.map(|c| c * 0.75 / max);
    }
    let [r, g, b] = colour.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
    Some(Rgb(r, g, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_vivid_hue_over_a_larger_grey_area() {
        let mut pixels = vec![[120, 120, 120]; 400]; // mostly grey
        pixels.extend(vec![[20, 20, 20]; 100]); // near-black
        pixels.extend(vec![[230, 40, 90]; 60]); // a small pink area
        let Rgb(r, g, b) = vivid(&pixels).unwrap();
        assert!(r > 200 && g < 80 && b > 60 && b < 120, "got {r},{g},{b}");
    }

    #[test]
    fn dark_colours_are_lifted_to_read_on_black() {
        let Rgb(r, g, b) = vivid(&vec![[90, 20, 20]; 100]).unwrap();
        assert!(r >= 190 && g < r && b < r, "got {r},{g},{b}");
    }

    /// A `w`×`h` black image with a grey picture at `(x, y, cw, ch)`.
    fn framed(w: u32, h: u32, (x, y, cw, ch): (u32, u32, u32, u32)) -> Vec<[u8; 3]> {
        (0..h).flat_map(|r| (0..w).map(move |c| if (x..x + cw).contains(&c) && (y..y + ch).contains(&r) { [140, 90, 60] } else { [6, 6, 6] })).collect()
    }

    #[test]
    fn letterbox_and_pillarbox_bars_are_trimmed() {
        assert_eq!(content_bounds(&framed(64, 48, (0, 6, 64, 36)), 64, 48), (0, 6, 64, 36));
        assert_eq!(content_bounds(&framed(64, 36, (8, 0, 48, 36)), 64, 36), (8, 0, 48, 36));
        // Both at once, slightly uneven (encoders round).
        assert_eq!(content_bounds(&framed(64, 48, (8, 5, 48, 37)), 64, 48), (8, 5, 48, 37));
        // A speck in a bar is noise, not picture.
        let mut speck = framed(100, 60, (0, 10, 100, 40));
        speck[3 * 100 + 50] = [255, 255, 255];
        assert_eq!(content_bounds(&speck, 100, 60), (0, 10, 100, 40));
    }

    #[test]
    fn covers_that_are_just_dark_stay_whole() {
        // Dark along one edge only: a design, not bars.
        assert_eq!(content_bounds(&framed(40, 40, (0, 0, 40, 30)), 40, 40), (0, 0, 40, 40));
        // All dark, or almost all bar: leave it.
        assert_eq!(content_bounds(&vec![[0, 0, 0]; 1600], 40, 40), (0, 0, 40, 40));
        assert_eq!(content_bounds(&framed(40, 40, (0, 18, 40, 4)), 40, 40), (0, 0, 40, 40));
        assert_eq!(content_bounds(&[], 0, 0), (0, 0, 0, 0));
    }

    #[test]
    fn bmp_rows_are_padded() {
        let b = bmp(3, 2, &[[1, 2, 3]; 6]);
        assert_eq!(b.len(), 54 + 12 * 2);
        assert_eq!(&b[54..57], &[3, 2, 1]);
    }

    #[test]
    fn greyscale_has_no_accent() {
        assert_eq!(vivid(&vec![[128, 128, 128]; 100]), None);
    }
}
