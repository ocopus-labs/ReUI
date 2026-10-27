//! Image codecs (WIC): copied PNGs and DIBs to PNG plus thumbnail, app icons, link
//! thumbnails, and PNG back to pixels for a paste-back.

use std::ffi::c_void;

use windows::core::{Interface, Result, GUID, PCWSTR};
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, IStream, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, STATFLAG_NONAME, STATSTG, STREAM_SEEK_SET};
use windows::Win32::UI::Shell::SHCreateMemStream;
use windows::Win32::UI::WindowsAndMessaging::*;

use super::access::within_cap;
use super::formats::{dib_pixels, png_has_alpha};
use super::Content;

const THUMB_PX: u32 = 320;

unsafe fn decode(wic: &IWICImagingFactory, bytes: &[u8]) -> Result<IWICBitmapFrameDecode> {
    let stream = wic.CreateStream()?;
    stream.InitializeFromMemory(bytes)?;
    wic.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnLoad)?.GetFrame(0)
}

unsafe fn source_size(source: &IWICBitmapSource) -> Result<(u32, u32)> {
    let (mut w, mut h) = (0u32, 0u32);
    source.GetSize(&mut w, &mut h)?;
    Ok((w, h))
}

/// A copied PNG is kept byte for byte once it's known to decode; only the thumbnail is new.
pub(super) unsafe fn png_image(wic: &IWICImagingFactory, png: Vec<u8>) -> Result<(Content, u32, u32)> {
    let frame = decode(wic, &png)?;
    let (w, h) = source_size(&frame)?;
    if !within_cap(u64::from(w), u64::from(h)) {
        return Err(windows::core::Error::empty());
    }
    let thumb = scaled_png(wic, &frame, THUMB_PX, png_has_alpha(&png))?;
    drop(frame);
    Ok((Content::Image { png, thumb }, w, h))
}

/// CF_DIBV5 / CF_DIB → (PNG and thumbnail, width, height). 32 bpp keeps its alpha (taken as
/// straight, which is what browsers and Office write); other depths are opaque.
pub(super) unsafe fn dib_image(wic: &IWICImagingFactory, dib: &[u8]) -> Result<(Content, u32, u32)> {
    let Some(px) = dib_pixels(dib) else { return bmp_image(wic, dib) };
    if !within_cap(u64::from(px.w), u64::from(px.h)) {
        return Err(windows::core::Error::empty());
    }
    let source: IWICBitmapSource = wic.CreateBitmapFromMemory(px.w, px.h, &GUID_WICPixelFormat32bppBGRA, px.w * 4, &px.bgra)?.cast()?;
    let format = if px.alpha { GUID_WICPixelFormat32bppBGRA } else { GUID_WICPixelFormat24bppBGR };
    let png = encode_png(wic, &convert(wic, &source, &format)?, format)?;
    let thumb = scaled_png(wic, &source, THUMB_PX, px.alpha)?;
    Ok((Content::Image { png, thumb }, px.w, px.h))
}

/// Any other DIB (palettes, 16/24 bpp, RLE) through WIC's BMP decoder: a DIB is a BMP
/// without its 14-byte file header.
unsafe fn bmp_image(wic: &IWICImagingFactory, dib: &[u8]) -> Result<(Content, u32, u32)> {
    if dib.len() < 40 {
        return Err(windows::core::Error::empty());
    }
    let header_size = u32::from_le_bytes(dib[0..4].try_into().unwrap());
    let bit_count = u16::from_le_bytes(dib[14..16].try_into().unwrap());
    let compression = u32::from_le_bytes(dib[16..20].try_into().unwrap());
    let colors_used = u32::from_le_bytes(dib[32..36].try_into().unwrap());
    let palette = if bit_count <= 8 { (if colors_used > 0 { colors_used } else { 1 << bit_count }) * 4 } else { colors_used * 4 };
    let masks = match (compression, header_size) {
        (3, 40) => 12,
        (6, 40) => 16,
        _ => 0,
    };
    let mut bmp = Vec::with_capacity(14 + dib.len());
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&((14 + dib.len()) as u32).to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&(14 + header_size + palette + masks).to_le_bytes());
    bmp.extend_from_slice(dib);

    let frame = decode(wic, &bmp)?;
    let (w, h) = source_size(&frame)?;
    if !within_cap(u64::from(w), u64::from(h)) {
        return Err(windows::core::Error::empty());
    }
    let source = convert(wic, &frame, &GUID_WICPixelFormat24bppBGR)?;
    let png = encode_png(wic, &source, GUID_WICPixelFormat24bppBGR)?;
    let thumb = scaled_png(wic, &source, THUMB_PX, false)?;
    Ok((Content::Image { png, thumb }, w, h))
}

/// A PNG no larger than `max_px` on its long side; with `alpha`, scaled premultiplied (no
/// dark fringes) and written as 32 bpp BGRA, else 24 bpp.
unsafe fn scaled_png(wic: &IWICImagingFactory, source: &IWICBitmapSource, max_px: u32, alpha: bool) -> Result<Vec<u8>> {
    let (w, h) = source_size(source)?;
    let k = (max_px as f32 / w.max(h).max(1) as f32).min(1.0);
    let working = if alpha { GUID_WICPixelFormat32bppPBGRA } else { GUID_WICPixelFormat24bppBGR };
    let scaler = wic.CreateBitmapScaler()?;
    scaler.Initialize(&convert(wic, source, &working)?, ((w as f32 * k) as u32).max(1), ((h as f32 * k) as u32).max(1), WICBitmapInterpolationModeFant)?;
    let format = if alpha { GUID_WICPixelFormat32bppBGRA } else { GUID_WICPixelFormat24bppBGR };
    encode_png(wic, &convert(wic, &scaler.cast()?, &format)?, format)
}

/// Any encoded image (a page's og:image) → small PNG thumbnail. Initialises COM on the
/// calling thread, so it can run from a blocking-pool thread.
pub fn thumbnail(bytes: &[u8], max_px: u32) -> Option<Vec<u8>> {
    unsafe {
        _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let frame = decode(&wic, bytes).ok()?;
        scaled_png(&wic, &frame, max_px, false).ok()
    }
}

/// The app's own icon as a 32×32 PNG with transparency.
pub(super) fn app_icon(wic: &IWICImagingFactory, exe: &str) -> Option<Vec<u8>> {
    unsafe {
        let path: Vec<u16> = exe.encode_utf16().chain([0]).collect();
        let mut info = windows::Win32::UI::Shell::SHFILEINFOW::default();
        let found = windows::Win32::UI::Shell::SHGetFileInfoW(
            PCWSTR(path.as_ptr()),
            windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut info),
            size_of::<windows::Win32::UI::Shell::SHFILEINFOW>() as u32,
            windows::Win32::UI::Shell::SHGFI_ICON | windows::Win32::UI::Shell::SHGFI_LARGEICON,
        );
        if found == 0 || info.hIcon.is_invalid() {
            return None;
        }
        let bitmap = wic.CreateBitmapFromHICON(info.hIcon);
        _ = DestroyIcon(info.hIcon);
        let source: IWICBitmapSource = bitmap.ok()?.cast().ok()?;
        let source = convert(wic, &source, &GUID_WICPixelFormat32bppBGRA).ok()?;
        encode_png(wic, &source, GUID_WICPixelFormat32bppBGRA).ok()
    }
}

unsafe fn convert(wic: &IWICImagingFactory, source: &IWICBitmapSource, format: &GUID) -> Result<IWICBitmapSource> {
    let converter = wic.CreateFormatConverter()?;
    converter.Initialize(source, format, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeMedianCut)?;
    converter.cast()
}

unsafe fn encode_png(wic: &IWICImagingFactory, source: &IWICBitmapSource, pixel_format: GUID) -> Result<Vec<u8>> {
    let stream: IStream = SHCreateMemStream(None).ok_or_else(windows::core::Error::empty)?;
    let encoder = wic.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
    encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
    let mut frame = None;
    encoder.CreateNewFrame(&mut frame, std::ptr::null_mut())?;
    let frame = frame.ok_or_else(windows::core::Error::empty)?;
    frame.Initialize(None)?;
    let (w, h) = source_size(source)?;
    frame.SetSize(w, h)?;
    let mut format = pixel_format;
    frame.SetPixelFormat(&mut format)?;
    frame.WriteSource(source, std::ptr::null())?;
    frame.Commit()?;
    encoder.Commit()?;

    let mut stat = STATSTG::default();
    stream.Stat(&mut stat, STATFLAG_NONAME)?;
    stream.Seek(0, STREAM_SEEK_SET, None)?;
    let mut bytes = vec![0u8; stat.cbSize as usize];
    let mut read = 0u32;
    stream.Read(bytes.as_mut_ptr() as *mut c_void, bytes.len() as u32, Some(&mut read)).ok()?;
    bytes.truncate(read as usize);
    Ok(bytes)
}

/// PNG → (width, height, top-down straight BGRA), for the DIB formats of a paste-back.
pub(super) unsafe fn png_to_bgra(wic: &IWICImagingFactory, png: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    let frame = decode(wic, png)?;
    let source = convert(wic, &frame, &GUID_WICPixelFormat32bppBGRA)?;
    let (w, h) = source_size(&source)?;
    let mut bgra = vec![0u8; w as usize * h as usize * 4];
    source.CopyPixels(std::ptr::null(), w * 4, &mut bgra)?;
    Ok((w, h, bgra))
}
