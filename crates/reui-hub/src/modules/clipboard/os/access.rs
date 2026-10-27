//! Opening, reading and writing the Win32 clipboard: the formats we keep, copied out raw
//! under the size caps, and prepared formats set back in one go.

use std::time::Duration;

use windows::core::Result;
use windows::Win32::Foundation::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::{CF_DIB, CF_DIBV5, CF_HDROP, CF_UNICODETEXT};
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

use super::formats::{dib_size, png_end, png_size};
use super::{Formats, Raw};

/// Per clipboard format: text, HTML and RTF bigger than this aren't kept (text over it skips
/// the whole copy rather than storing it cut short).
const MAX_FORMAT_BYTES: usize = 16 << 20;
/// Bigger images are skipped: an 8K screenshot is 33 MP, this is ~1.4 GB of RGBA work.
const MAX_IMAGE_PIXELS: u64 = 50_000_000;

/// Closes the clipboard when dropped.
struct Open;

impl Open {
    /// Another app may still hold the clipboard right after it changes. A couple of very
    /// short tries only: the caller retries later rather than blocking the message loop.
    unsafe fn new(hwnd: HWND) -> Option<Open> {
        for attempt in 0..3 {
            if attempt > 0 {
                std::thread::sleep(Duration::from_millis(5));
            }
            if OpenClipboard(Some(hwnd)).is_ok() {
                return Some(Open);
            }
        }
        None
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        unsafe { _ = CloseClipboard() };
    }
}

pub(super) enum Read {
    /// Another app has the clipboard open.
    Busy,
    /// Nothing we keep, too large, or excluded.
    Skip,
    /// Raw content and the clipboard owner's window.
    Got(Raw, Option<HWND>),
}

/// Copies the raw bytes of the formats we keep and closes the clipboard right away;
/// decoding happens on the worker.
pub(super) unsafe fn read(hwnd: HWND, formats: Formats) -> Read {
    let Some(open) = Open::new(hwnd) else { return Read::Busy };
    if has(formats.exclude) || has(formats.viewer_ignore) {
        return Read::Skip;
    }
    if has(formats.history) {
        // `CanIncludeInClipboardHistory` = 0 means private; unreadable counts as private too.
        let allowed = with_global(formats.history, |b| b.len() >= 4 && u32::from_le_bytes([b[0], b[1], b[2], b[3]]) != 0);
        if allowed != Some(true) {
            return Read::Skip;
        }
    }
    let owner = GetClipboardOwner().ok().filter(|h| !h.is_invalid());
    let raw = read_content(formats);
    drop(open);
    match raw {
        Some(raw) => Read::Got(raw, owner),
        None => Read::Skip,
    }
}

/// Whether a format is offered (doesn't make a delayed-render owner render it).
unsafe fn has(format: u32) -> bool {
    format != 0 && IsClipboardFormatAvailable(format).is_ok()
}

/// Files beat text (Explorer offers both), text beats pictures (Office offers both), but
/// blank text falls through to the picture (Office shapes carry an empty CF_UNICODETEXT).
/// Only the formats we keep are asked for: each `GetClipboardData` can force a delayed render.
unsafe fn read_content(formats: Formats) -> Option<Raw> {
    if has(u32::from(CF_HDROP.0)) {
        if let Some(paths) = read_files() {
            let effect = if has(formats.drop_effect) { with_global(formats.drop_effect, |b| Some(u32::from_le_bytes(b.get(..4)?.try_into().ok()?))).flatten() } else { None };
            return Some(Raw::Files { paths, cut: effect.is_some_and(is_move) });
        }
    }
    if has(u32::from(CF_UNICODETEXT.0)) {
        match read_capped(u32::from(CF_UNICODETEXT.0), 2) {
            Capped::TooLarge => {
                eprintln!("clipboard: copied text is too large (over {} MB); not recorded", MAX_FORMAT_BYTES >> 20);
                return None;
            }
            Capped::Bytes(utf16) if !is_blank_utf16(&utf16) => {
                let rich = |format: u32, name: &str| match if has(format) { read_capped(format, 1) } else { Capped::Missing } {
                    Capped::Bytes(bytes) if !bytes.is_empty() => Some(bytes),
                    Capped::TooLarge => {
                        eprintln!("clipboard: copied {name} is too large; kept as plain text");
                        None
                    }
                    _ => None,
                };
                return Some(Raw::Text { utf16, html: rich(formats.html, "HTML"), rtf: rich(formats.rtf, "RTF") });
            }
            _ => {}
        }
    }
    if has(formats.png) {
        if let Some(png) = read_png(formats.png) {
            return Some(Raw::Png(png));
        }
    }
    // Windows synthesises CF_DIBV5 from CF_DIB (and back), so CF_DIB is the rare fallback.
    for format in [CF_DIBV5, CF_DIB] {
        if has(u32::from(format.0)) {
            if let Some(dib) = read_dib(u32::from(format.0)) {
                return Some(Raw::Dib(dib));
            }
        }
    }
    None
}

/// "Preferred DropEffect" says move (cut in Explorer) rather than copy.
fn is_move(effect: u32) -> bool {
    const DROPEFFECT_COPY: u32 = 1;
    const DROPEFFECT_MOVE: u32 = 2;
    effect & DROPEFFECT_MOVE != 0 && effect & DROPEFFECT_COPY == 0
}

/// Runs `f` on the bytes of a clipboard format while it's locked (the clipboard must be open).
unsafe fn with_global<T>(format: u32, f: impl FnOnce(&[u8]) -> T) -> Option<T> {
    let handle = GetClipboardData(format).ok()?;
    let global = HGLOBAL(handle.0);
    let ptr = GlobalLock(global) as *const u8;
    if ptr.is_null() {
        return None;
    }
    let result = f(std::slice::from_raw_parts(ptr, GlobalSize(global)));
    _ = GlobalUnlock(global);
    Some(result)
}

enum Capped {
    Missing,
    TooLarge,
    Bytes(Vec<u8>),
}

/// A NUL-terminated clipboard string (`unit`: 1 byte, or 2 for UTF-16) without its terminator
/// and the allocation's padding; refused past the byte cap rather than cut short.
unsafe fn read_capped(format: u32, unit: usize) -> Capped {
    with_global(format, |bytes| {
        let bytes = until_nul(bytes, unit);
        if bytes.len() > MAX_FORMAT_BYTES {
            Capped::TooLarge
        } else {
            Capped::Bytes(bytes.to_vec())
        }
    })
    .unwrap_or(Capped::Missing)
}

fn until_nul(bytes: &[u8], unit: usize) -> &[u8] {
    let whole = bytes.len() - bytes.len() % unit;
    let end = bytes.chunks_exact(unit).position(|c| c.iter().all(|&b| b == 0)).map_or(whole, |i| i * unit);
    &bytes[..end]
}

fn is_blank_utf16(bytes: &[u8]) -> bool {
    bytes.chunks_exact(2).all(|c| char::from_u32(u32::from(u16::from_le_bytes([c[0], c[1]]))).is_some_and(char::is_whitespace))
}

/// CF_HTML bytes → the payload as received (header, offsets and all). The offsets count
/// UTF-8 bytes, so it's kept verbatim rather than re-encoded or trimmed to the fragment.
pub(super) fn html_from_clipboard(bytes: &[u8]) -> Option<String> {
    let bytes = until_nul(bytes, 1);
    (!bytes.is_empty()).then(|| String::from_utf8_lossy(bytes).into_owned())
}

pub(super) fn html_to_clipboard(html: &str) -> Vec<u8> {
    html.bytes().chain([0]).collect()
}

unsafe fn read_files() -> Option<Vec<String>> {
    let handle = GetClipboardData(u32::from(CF_HDROP.0)).ok()?;
    let drop = HDROP(handle.0);
    let count = DragQueryFileW(drop, u32::MAX, None);
    let files: Vec<String> = (0..count)
        .filter_map(|i| {
            let len = DragQueryFileW(drop, i, None) as usize;
            let mut buffer = vec![0u16; len + 1];
            (DragQueryFileW(drop, i, Some(&mut buffer)) > 0).then(|| String::from_utf16_lossy(&buffer[..len]))
        })
        .collect();
    (!files.is_empty()).then_some(files)
}

pub(super) fn within_cap(w: u64, h: u64) -> bool {
    let ok = w * h <= MAX_IMAGE_PIXELS;
    if !ok {
        eprintln!("clipboard: skipped a {w}×{h} image (over {} MP)", MAX_IMAGE_PIXELS / 1_000_000);
    }
    ok
}

/// The registered "PNG" bytes (alpha intact), up to the end of the file; `None` if it isn't
/// a PNG or is over the size cap (checked from the header, before copying).
unsafe fn read_png(format: u32) -> Option<Vec<u8>> {
    with_global(format, |png| {
        let (w, h) = png_size(png)?;
        within_cap(u64::from(w), u64::from(h)).then(|| png[..png_end(png).unwrap_or(png.len())].to_vec())
    })
    .flatten()
}

/// The DIB bytes, unless the header says the image is over the size cap (checked before
/// copying, so a huge image costs nothing).
unsafe fn read_dib(format: u32) -> Option<Vec<u8>> {
    with_global(format, |dib| {
        let (w, h) = dib_size(dib)?;
        within_cap(w, h).then(|| dib.to_vec())
    })
    .flatten()
}

/// Sets every prepared format in one short open/empty/set/close.
pub(super) unsafe fn write(hwnd: HWND, data: &[(u32, Vec<u8>)]) -> Result<()> {
    let _open = Open::new(hwnd).ok_or_else(windows::core::Error::empty)?;
    EmptyClipboard()?;
    for (format, bytes) in data {
        set_global(*format, bytes)?;
    }
    Ok(())
}

unsafe fn set_global(format: u32, bytes: &[u8]) -> Result<()> {
    let global = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1))?;
    let ptr = GlobalLock(global) as *mut u8;
    if ptr.is_null() {
        _ = GlobalFree(Some(global));
        return Err(windows::core::Error::from_thread());
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
    _ = GlobalUnlock(global);
    // On success the clipboard owns the memory; on failure it's still ours.
    if let Err(e) = SetClipboardData(format, Some(HANDLE(global.0))) {
        _ = GlobalFree(Some(global));
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cf_html_round_trips() {
        let html = "Version:0.9\r\nStartHTML:0000000105\r\nEndHTML:0000000199\r\nStartFragment:0000000141\r\nEndFragment:0000000163\r\n<html><body>\r\n<!--StartFragment--><b>héllo</b><!--EndFragment-->\r\n</body></html>";
        let mut clipboard = html_to_clipboard(html);
        assert_eq!(clipboard.last(), Some(&0));
        clipboard.extend_from_slice(&[0; 7]); // allocation padding
        assert_eq!(html_from_clipboard(&clipboard).as_deref(), Some(html));
        assert_eq!(html_from_clipboard(&[0, 0]), None);
    }

    #[test]
    fn text_helpers() {
        let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<u8>>();
        let mut padded = utf16("hi");
        padded.extend_from_slice(&[0, 0, 7, 7]);
        assert_eq!(until_nul(&padded, 2), utf16("hi").as_slice());
        assert!(is_blank_utf16(&utf16(" \r\n\t\u{3000}")));
        assert!(!is_blank_utf16(&utf16("  😀 ")));
        assert!(is_move(2) && !is_move(5) && !is_move(1) && !is_move(3));
    }
}
