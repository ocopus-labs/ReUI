//! An accent colour from artwork: the most vivid hue in the image, not its (usually muddy)
//! average. Decodes a tiny copy with WIC; needs COM initialised on the calling thread.

use reui_protocol::Rgb;
use windows::core::Result;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};

const SAMPLE: u32 = 24;
const HUE_BUCKETS: usize = 12;

pub fn accent(bytes: &[u8]) -> Option<Rgb> {
    pixels(bytes).ok().and_then(|p| vivid(&p))
}

/// The image as SAMPLE×SAMPLE straight RGB pixels.
fn pixels(bytes: &[u8]) -> Result<Vec<[u8; 3]>> {
    unsafe {
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let stream = wic.CreateStream()?;
        stream.InitializeFromMemory(bytes)?;
        let decoder = wic.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)?;
        let scaler = wic.CreateBitmapScaler()?;
        scaler.Initialize(&decoder.GetFrame(0)?, SAMPLE, SAMPLE, WICBitmapInterpolationModeFant)?;
        let converter = wic.CreateFormatConverter()?;
        converter.Initialize(&scaler, &GUID_WICPixelFormat24bppRGB, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeMedianCut)?;
        let mut buffer = vec![0u8; (SAMPLE * SAMPLE * 3) as usize];
        converter.CopyPixels(std::ptr::null(), SAMPLE * 3, &mut buffer)?;
        Ok(buffer.chunks_exact(3).map(|p| [p[0], p[1], p[2]]).collect())
    }
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

    #[test]
    fn greyscale_has_no_accent() {
        assert_eq!(vivid(&vec![[128, 128, 128]; 100]), None);
    }
}
