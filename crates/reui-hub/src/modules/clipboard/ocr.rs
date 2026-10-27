//! Text inside copied images, with Windows' own OCR (`Windows.Media.Ocr`): local, no
//! network, in the user's profile languages. The result is stored with the item and makes
//! screenshots findable by the words in them.
//!
//! Recognition is blocking WinRT work, so it runs on a blocking-pool thread, one image at
//! a time, and the caller gives up after [`TIME_BOX`]. Big screenshots are scaled down to
//! the engine's limit while decoding. Nothing runs on battery saver.

use std::time::Duration;

use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapDecoder, BitmapInterpolationMode, BitmapPixelFormat, BitmapTransform, ColorManagementMode, ExifOrientationMode};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::IRandomAccessStream;
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::WinRT::{CreateRandomAccessStreamOverStream, RoInitialize, BSOS_DEFAULT, RO_INIT_MULTITHREADED};
use windows::Win32::UI::Shell::SHCreateMemStream;

/// Longest a single image may take (PRD W-8).
pub const TIME_BOX: Duration = Duration::from_secs(2);

/// The recognised text of a PNG, lines joined with newlines ("" when it has none);
/// `None` when OCR isn't available (no language pack, battery saver), timed out, or
/// failed, in which case the image is tried again another time.
pub async fn recognize(png: Vec<u8>) -> Option<String> {
    if battery_saver() {
        return None;
    }
    let work = tokio::task::spawn_blocking(move || {
        recognize_blocking(&png).map_err(|e| eprintln!("clipboard: OCR failed: {e}")).ok()
    });
    tokio::time::timeout(TIME_BOX, work).await.ok()?.ok()?
}

fn recognize_blocking(png: &[u8]) -> windows::core::Result<String> {
    unsafe {
        // Pool threads may not be in a COM apartment yet; "already" is fine too.
        _ = RoInitialize(RO_INIT_MULTITHREADED);
    }
    let engine = OcrEngine::TryCreateFromUserProfileLanguages()?;
    let max = OcrEngine::MaxImageDimension()?.max(1);
    let memory = unsafe { SHCreateMemStream(Some(png)) }.ok_or_else(windows::core::Error::empty)?;
    let stream: IRandomAccessStream = unsafe { CreateRandomAccessStreamOverStream(&memory, BSOS_DEFAULT)? };
    let decoder = BitmapDecoder::CreateAsync(&stream)?.join()?;
    let (w, h) = (decoder.PixelWidth()?, decoder.PixelHeight()?);
    let transform = BitmapTransform::new()?;
    let k = (f64::from(max) / f64::from(w.max(h).max(1))).min(1.0);
    if k < 1.0 {
        transform.SetScaledWidth(((f64::from(w) * k) as u32).max(1))?;
        transform.SetScaledHeight(((f64::from(h) * k) as u32).max(1))?;
        transform.SetInterpolationMode(BitmapInterpolationMode::Fant)?;
    }
    let bitmap = decoder
        .GetSoftwareBitmapTransformedAsync(BitmapPixelFormat::Bgra8, BitmapAlphaMode::Premultiplied, &transform, ExifOrientationMode::IgnoreExifOrientation, ColorManagementMode::DoNotColorManage)?
        .join()?;
    let result = engine.RecognizeAsync(&bitmap)?.join()?;
    let lines = result.Lines()?;
    let mut text = Vec::new();
    for i in 0..lines.Size()? {
        let line = lines.GetAt(i)?.Text()?.to_string();
        if !line.trim().is_empty() {
            text.push(line);
        }
    }
    Ok(text.join("\n"))
}

/// Windows' battery saver is on (W-8: OCR waits for it to end).
fn battery_saver() -> bool {
    let mut status = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut status) }.is_ok() && status.SystemStatusFlag == 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::core::w;
    use windows::Win32::Graphics::Gdi::*;

    /// Black text on white, drawn with GDI, as a PNG (via a BMP and the WIC thumbnailer).
    fn rendered(text: &str) -> Vec<u8> {
        let (w, h) = (900i32, 120i32);
        unsafe {
            let dc = CreateCompatibleDC(None);
            let info = BITMAPINFO { bmiHeader: BITMAPINFOHEADER { biSize: size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, ..Default::default() }, ..Default::default() };
            let mut bits = std::ptr::null_mut();
            let bitmap = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0).unwrap();
            let old = SelectObject(dc, bitmap.into());
            std::ptr::write_bytes(bits as *mut u8, 255, (w * h * 4) as usize);
            let font = CreateFontW(72, 0, 0, 0, 400, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, ANTIALIASED_QUALITY, 0, w!("Segoe UI"));
            SelectObject(dc, font.into());
            SetBkMode(dc, TRANSPARENT);
            let wide: Vec<u16> = text.encode_utf16().collect();
            _ = TextOutW(dc, 20, 20, &wide);
            _ = GdiFlush();
            let pixels = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize).to_vec();
            SelectObject(dc, old);
            _ = DeleteObject(bitmap.into());
            _ = DeleteObject(font.into());
            _ = DeleteDC(dc);
            // A top-down 32 bpp BMP file.
            let mut bmp = b"BM".to_vec();
            bmp.extend_from_slice(&(54 + pixels.len() as u32).to_le_bytes());
            bmp.extend_from_slice(&[0; 4]);
            bmp.extend_from_slice(&54u32.to_le_bytes());
            for v in [40u32, w as u32, (-h) as u32, 1 | (32 << 16), 0, pixels.len() as u32, 0, 0, 0, 0] {
                bmp.extend_from_slice(&v.to_le_bytes());
            }
            bmp.extend_from_slice(&pixels);
            super::super::os::thumbnail(&bmp, 4096).unwrap()
        }
    }

    /// Needs Windows' OCR language pack for the user's language (English here).
    #[tokio::test]
    #[ignore]
    async fn ocr_reads_rendered_text() {
        let text = recognize(rendered("Quarterly revenue 2026")).await.expect("OCR available");
        assert!(text.to_lowercase().contains("revenue"), "{text:?}");
    }
}
