//! OLE data objects both ways (docs/PRD-file-shelf.md): reading what a drag dropped on the
//! island offers (`read`, into `features::drop::Offer`), and building what a card carries
//! out (`build`): files through the shell's own data object (Shell IDList Array, so
//! Explorer, mail and upload pages all get what they expect, `CF_HDROP` included), text and
//! links as `CF_UNICODETEXT` and `UniformResourceLocatorW` set on an empty one.

use std::mem::ManuallyDrop;

use reui_protocol::Drag;
use windows::core::{w, Result, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HGLOBAL, POINT, S_OK};
use windows::Win32::System::Com::{IDataObject, IStream, DVASPECT_CONTENT, FORMATETC, STGMEDIUM, STGMEDIUM_0, TYMED, TYMED_HGLOBAL, TYMED_ISTREAM};
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE, GMEM_ZEROINIT};
use windows::Win32::System::Ole::{ReleaseStgMedium, CF_HDROP, CF_UNICODETEXT};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{DragQueryFileW, ILFree, SHCreateDataObject, SHParseDisplayName, DROPFILES, FILEDESCRIPTORW, HDROP};

use crate::features::drop::{Offer, DATA_MAX};

/// `FD_FILESIZE`: a virtual file's descriptor says how big it is.
const FD_FILESIZE: u32 = 0x40;

fn format(name: PCWSTR) -> u16 {
    unsafe { RegisterClipboardFormatW(name) as u16 }
}

/// Marks a drag ReUI started, so the island never takes its own drag back.
pub fn reui_format() -> u16 {
    format(w!("ReUI.Drag"))
}

fn etc(cf: u16, tymed: TYMED, index: i32) -> FORMATETC {
    FORMATETC { cfFormat: cf, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: index, tymed: tymed.0 as u32 }
}

fn has(data: &IDataObject, cf: u16) -> bool {
    // S_OK only: some data objects answer S_FALSE for a format they don't have.
    unsafe { data.QueryGetData(&etc(cf, TYMED_HGLOBAL, -1)) == S_OK }
}

/// Whether the drag offers anything the shelf can hold (formats only: nothing is read).
pub fn holdable(data: &IDataObject) -> bool {
    !has(data, reui_format())
        && [CF_HDROP.0, format(w!("FileGroupDescriptorW")), format(w!("UniformResourceLocatorW")), format(w!("PNG")), CF_UNICODETEXT.0].into_iter().any(|cf| has(data, cf))
}

/// Everything the drag offers, read once, when it's dropped.
pub fn read(data: &IDataObject) -> Offer {
    let mut offer = Offer { files: files(data), ..Default::default() };
    if !offer.files.is_empty() {
        return offer;
    }
    offer.virtual_files = virtual_files(data);
    offer.url = bytes(data, format(w!("UniformResourceLocatorW")), -1, DATA_MAX).map(|b| utf16(&b));
    offer.png = bytes(data, format(w!("PNG")), -1, DATA_MAX);
    offer.text = bytes(data, CF_UNICODETEXT.0, -1, DATA_MAX).map(|b| utf16(&b));
    offer
}

fn files(data: &IDataObject) -> Vec<String> {
    unsafe {
        let Ok(mut medium) = data.GetData(&etc(CF_HDROP.0, TYMED_HGLOBAL, -1)) else { return Vec::new() };
        let mut paths = Vec::new();
        if medium.tymed == TYMED_HGLOBAL.0 as u32 {
            let drop = HDROP(medium.u.hGlobal.0);
            for i in 0..DragQueryFileW(drop, u32::MAX, None) {
                let len = DragQueryFileW(drop, i, None) as usize;
                let mut buf = vec![0u16; len + 1];
                DragQueryFileW(drop, i, Some(&mut buf));
                paths.push(String::from_utf16_lossy(&buf[..len]));
            }
        }
        ReleaseStgMedium(&mut medium);
        paths
    }
}

/// Files with no path: their names from the group descriptor, each one's content read only
/// if it fits.
fn virtual_files(data: &IDataObject) -> Vec<(String, Option<Vec<u8>>)> {
    let Some(group) = bytes(data, format(w!("FileGroupDescriptorW")), -1, 1024 * 1024) else { return Vec::new() };
    let count = group.get(..4).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])) as usize;
    let size = std::mem::size_of::<FILEDESCRIPTORW>();
    let contents = format(w!("FileContents"));
    (0..count)
        .filter_map(|i| {
            let at = 4 + i * size;
            let raw = group.get(at..at + size)?;
            let d: FILEDESCRIPTORW = unsafe { std::ptr::read_unaligned(raw.as_ptr() as *const FILEDESCRIPTORW) };
            let name = { let units = d.cFileName; utf16_units(&units) };
            let known = (d.dwFlags & FD_FILESIZE != 0).then(|| (u64::from(d.nFileSizeHigh) << 32) | u64::from(d.nFileSizeLow));
            let fits = known.is_none_or(|n| n as usize <= DATA_MAX);
            // Memory blocks come rounded up; the descriptor knows the real length.
            let content = if fits { bytes(data, contents, i as i32, DATA_MAX) } else { None }.map(|mut b| {
                if let Some(n) = known {
                    b.truncate(n as usize);
                }
                b
            });
            Some((name, content))
        })
        .collect()
}

/// The bytes of format `cf` (item `index` of a multi-item format), from memory or a
/// stream; `None` past `max` bytes.
fn bytes(data: &IDataObject, cf: u16, index: i32, max: usize) -> Option<Vec<u8>> {
    unsafe {
        let mut medium = data.GetData(&etc(cf, TYMED(TYMED_HGLOBAL.0 | TYMED_ISTREAM.0), index)).ok()?;
        let out = if medium.tymed == TYMED_HGLOBAL.0 as u32 {
            global_bytes(medium.u.hGlobal, max)
        } else if medium.tymed == TYMED_ISTREAM.0 as u32 {
            medium.u.pstm.as_ref().and_then(|s| stream_bytes(s, max))
        } else {
            None
        };
        ReleaseStgMedium(&mut medium);
        out
    }
}

unsafe fn global_bytes(global: HGLOBAL, max: usize) -> Option<Vec<u8>> {
    let len = GlobalSize(global);
    if len > max {
        return None;
    }
    let at = GlobalLock(global) as *const u8;
    if at.is_null() {
        return None;
    }
    let out = std::slice::from_raw_parts(at, len).to_vec();
    _ = GlobalUnlock(global);
    Some(out)
}

unsafe fn stream_bytes(stream: &IStream, max: usize) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        let mut read = 0u32;
        if stream.Read(chunk.as_mut_ptr() as *mut _, chunk.len() as u32, Some(&mut read)).is_err() {
            return None;
        }
        if read == 0 {
            return Some(out);
        }
        out.extend_from_slice(&chunk[..read as usize]);
        if out.len() > max {
            return None;
        }
    }
}

fn utf16(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    utf16_units(&units)
}

/// Up to the first NUL.
fn utf16_units(units: &[u16]) -> String {
    let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

/// The data object a card carries out (not yet marked as ReUI's; see `mark`).
pub fn build(drag: &Drag) -> Result<IDataObject> {
    unsafe {
        match drag {
            Drag::Files(paths) => {
                let pidls: Vec<*mut ITEMIDLIST> = paths.iter().filter_map(|p| pidl(p)).collect();
                let items: Vec<*const ITEMIDLIST> = pidls.iter().map(|p| *p as *const _).collect();
                // No parent folder: each item is a full path from the desktop, so files
                // from several folders travel together.
                let data = SHCreateDataObject::<_, IDataObject>(None, (!items.is_empty()).then_some(items.as_slice()), None);
                for p in pidls {
                    ILFree(Some(p));
                }
                let data = data?;
                if !has(&data, CF_HDROP.0) {
                    set(&data, CF_HDROP.0, &hdrop(paths))?;
                }
                Ok(data)
            }
            Drag::Text(text) => {
                let data: IDataObject = SHCreateDataObject(None, None, None)?;
                set(&data, CF_UNICODETEXT.0, &wide(text))?;
                Ok(data)
            }
            Drag::Link(url) => {
                let data: IDataObject = SHCreateDataObject(None, None, None)?;
                set(&data, CF_UNICODETEXT.0, &wide(url))?;
                set(&data, format(w!("UniformResourceLocatorW")), &wide(url))?;
                Ok(data)
            }
        }
    }
}

/// Marks `data` as ReUI's own drag.
pub fn mark(data: &IDataObject) -> Result<()> {
    set(data, reui_format(), &[1, 0])
}

unsafe fn pidl(path: &str) -> Option<*mut ITEMIDLIST> {
    let mut out: *mut ITEMIDLIST = std::ptr::null_mut();
    SHParseDisplayName(&HSTRING::from(path), None, &mut out, 0, None).ok()?;
    (!out.is_null()).then_some(out)
}

fn wide(text: &str) -> Vec<u8> {
    text.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect()
}

/// A `CF_HDROP` block: a `DROPFILES` header, then the paths, each NUL-ended, and one more NUL.
fn hdrop(paths: &[String]) -> Vec<u8> {
    let header = DROPFILES { pFiles: std::mem::size_of::<DROPFILES>() as u32, pt: POINT::default(), fNC: false.into(), fWide: true.into() };
    let mut out = unsafe { std::slice::from_raw_parts(&header as *const DROPFILES as *const u8, std::mem::size_of::<DROPFILES>()) }.to_vec();
    for path in paths {
        out.extend(wide(path));
    }
    out.extend([0, 0]);
    out
}

/// Sets format `cf` on `data` to `bytes`; the data object owns the memory afterwards.
fn set(data: &IDataObject, cf: u16, bytes: &[u8]) -> Result<()> {
    unsafe {
        let global = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, bytes.len().max(1))?;
        let at = GlobalLock(global) as *mut u8;
        if !at.is_null() {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), at, bytes.len());
            _ = GlobalUnlock(global);
        }
        let medium = STGMEDIUM { tymed: TYMED_HGLOBAL.0 as u32, u: STGMEDIUM_0 { hGlobal: global }, pUnkForRelease: ManuallyDrop::new(None) };
        data.SetData(&etc(cf, TYMED_HGLOBAL, -1), &medium, true)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use windows::Win32::System::Ole::OleInitialize;

    pub fn com() {
        unsafe { _ = OleInitialize(None) };
    }

    pub fn temp_files(name: &str, files: &[&str]) -> (std::path::PathBuf, Vec<String>) {
        let dir = std::env::temp_dir().join(format!("reui-dragdrop-{name}-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("other")).unwrap();
        let paths = files
            .iter()
            .map(|f| {
                let p = dir.join(f);
                std::fs::write(&p, b"x").unwrap();
                p.to_string_lossy().into_owned()
            })
            .collect();
        (dir, paths)
    }

    #[test]
    fn files_from_several_folders_go_out_as_cf_hdrop() {
        com();
        let (dir, paths) = temp_files("out", &["a.txt", r"other\b.txt"]);
        let data = build(&Drag::Files(paths.clone())).unwrap();
        assert!(holdable(&data));
        assert_eq!(read(&data).files, paths, "the paths read back from CF_HDROP");
        mark(&data).unwrap();
        assert!(!holdable(&data), "ReUI's own drag is never taken back");
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn text_and_links_go_out_as_text() {
        com();
        let text = build(&Drag::Text("héllo\nworld".into())).unwrap();
        assert_eq!(read(&text).text.as_deref(), Some("héllo\nworld"));
        let link = read(&build(&Drag::Link("https://example.com/a".into())).unwrap());
        assert_eq!((link.url.as_deref(), link.text.as_deref()), (Some("https://example.com/a"), Some("https://example.com/a")));
    }

    #[test]
    fn a_hand_made_hdrop_reads_back() {
        com();
        let data: IDataObject = unsafe { SHCreateDataObject(None, None, None) }.unwrap();
        let paths = vec![r"C:\one.txt".to_string(), r"\\server\share\two.pdf".to_string()];
        set(&data, CF_HDROP.0, &hdrop(&paths)).unwrap();
        assert_eq!(files(&data), paths);
    }

    #[test]
    fn virtual_files_are_read_by_name() {
        com();
        let data: IDataObject = unsafe { SHCreateDataObject(None, None, None) }.unwrap();
        let mut d = FILEDESCRIPTORW { dwFlags: FD_FILESIZE, nFileSizeLow: 3, ..Default::default() };
        for (i, u) in "cat.png".encode_utf16().enumerate() {
            d.cFileName[i] = u;
        }
        let mut group = 1u32.to_le_bytes().to_vec();
        group.extend_from_slice(unsafe { std::slice::from_raw_parts(&d as *const FILEDESCRIPTORW as *const u8, std::mem::size_of::<FILEDESCRIPTORW>()) });
        set(&data, format(w!("FileGroupDescriptorW")), &group).unwrap();
        unsafe {
            let global = GlobalAlloc(GMEM_MOVEABLE, 3).unwrap();
            std::ptr::copy_nonoverlapping([7u8, 8, 9].as_ptr(), GlobalLock(global) as *mut u8, 3);
            _ = GlobalUnlock(global);
            let medium = STGMEDIUM { tymed: TYMED_HGLOBAL.0 as u32, u: STGMEDIUM_0 { hGlobal: global }, pUnkForRelease: ManuallyDrop::new(None) };
            data.SetData(&etc(format(w!("FileContents")), TYMED_HGLOBAL, 0), &medium, true).unwrap();
        }
        assert!(holdable(&data));
        let offer = read(&data);
        assert_eq!(offer.virtual_files.len(), 1);
        assert_eq!(offer.virtual_files[0].0, "cat.png");
        // The shell's generic data object keeps one medium per format, whatever its index,
        // so only the name is certain here; real sources answer per index.
        assert!(offer.virtual_files[0].1.as_ref().is_some_and(|b| b.starts_with(&[7, 8, 9])));
    }
}
