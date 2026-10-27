//! Native drag-out from the clipboard window: an OLE data object with the clip as text, as
//! an image (DIB + PNG + a temporary .png file, since browsers and chat apps only take
//! image drops as files), or as files. `DoDragDrop` runs its own modal loop on this thread
//! until the drop.

use std::path::{Path, PathBuf};

use windows::core::{implement, w, Error, Result, BOOL, HRESULT};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{BITMAPINFOHEADER, BI_RGB};
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::*;
use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::Shell::{SHCreateStdEnumFmtEtc, DROPFILES};

/// What the hub's `export` answer offers.
#[derive(Default, Debug)]
pub struct Export {
    pub text: Option<String>,
    pub png: Option<(u64, Vec<u8>)>,
    pub files: Vec<PathBuf>,
}

/// Runs the drag; returns once it ends (dropped or cancelled).
pub fn run(export: Export) -> Result<()> {
    let formats = formats(export)?;
    if formats.is_empty() {
        return Ok(());
    }
    unsafe {
        // Already STA (RoInitialize); this adds OLE's drag-and-drop on top.
        _ = OleInitialize(None);
        let data: IDataObject = DataObject { formats }.into();
        let source: IDropSource = DropSource.into();
        let mut effect = DROPEFFECT::default();
        _ = DoDragDrop(&data, &source, DROPEFFECT_COPY, &mut effect);
    }
    Ok(())
}

fn formats(export: Export) -> Result<Vec<(u16, Vec<u8>)>> {
    let mut out = Vec::new();
    if let Some(text) = &export.text {
        out.push((CF_UNICODETEXT.0, utf16z(text)));
    }
    let mut files = export.files;
    if let Some((id, png)) = &export.png {
        if let Ok(dib) = dib_from_image(png) {
            out.push((CF_DIB.0, dib));
        }
        let png_format = unsafe { RegisterClipboardFormatW(w!("PNG")) } as u16;
        if png_format != 0 {
            out.push((png_format, png.clone()));
        }
        if files.is_empty() {
            if let Ok(path) = temp_png(*id, png) {
                files.push(path);
            }
        }
    }
    if !files.is_empty() {
        out.push((CF_HDROP.0, hdrop(&files)));
    }
    Ok(out)
}

fn utf16z(text: &str) -> Vec<u8> {
    text.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect()
}

/// `DROPFILES` followed by the wide paths, each NUL-terminated, plus a final NUL.
fn hdrop(files: &[PathBuf]) -> Vec<u8> {
    let header = DROPFILES { pFiles: size_of::<DROPFILES>() as u32, fWide: true.into(), ..Default::default() };
    let mut out = unsafe { std::slice::from_raw_parts(&header as *const DROPFILES as *const u8, size_of::<DROPFILES>()) }.to_vec();
    for file in files {
        out.extend(file.as_os_str().to_string_lossy().encode_utf16().chain([0]).flat_map(u16::to_le_bytes));
    }
    out.extend([0, 0]);
    out
}

fn temp_png(id: u64, png: &[u8]) -> std::io::Result<PathBuf> {
    let dir = std::env::temp_dir().join("ReUI").join("drag");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("clip-{id}.png"));
    write_if_changed(&path, png)?;
    Ok(path)
}

fn write_if_changed(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if std::fs::metadata(path).is_ok_and(|m| m.len() == bytes.len() as u64) {
        return Ok(());
    }
    std::fs::write(path, bytes)
}

/// A packed bottom-up 32-bit DIB (header + pixels) decoded from PNG/JPEG bytes with WIC.
fn dib_from_image(bytes: &[u8]) -> Result<Vec<u8>> {
    unsafe {
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let stream = wic.CreateStream()?;
        stream.InitializeFromMemory(bytes)?;
        let decoder = wic.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)?;
        let frame = WICConvertBitmapSource(&GUID_WICPixelFormat32bppBGRA, &decoder.GetFrame(0)?)?;
        let (mut w, mut h) = (0u32, 0u32);
        frame.GetSize(&mut w, &mut h)?;
        let stride = w as usize * 4;
        let mut pixels = vec![0u8; stride * h as usize];
        frame.CopyPixels(std::ptr::null(), stride as u32, &mut pixels)?;
        let header = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            biHeight: h as i32,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            biSizeImage: pixels.len() as u32,
            ..Default::default()
        };
        let mut out = std::slice::from_raw_parts(&header as *const BITMAPINFOHEADER as *const u8, size_of::<BITMAPINFOHEADER>()).to_vec();
        for row in pixels.chunks_exact(stride).rev() {
            out.extend_from_slice(row);
        }
        Ok(out)
    }
}

fn global(bytes: &[u8]) -> Result<HGLOBAL> {
    unsafe {
        let handle = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1))?;
        let ptr = GlobalLock(handle) as *mut u8;
        if ptr.is_null() {
            return Err(Error::from_thread());
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
        _ = GlobalUnlock(handle);
        Ok(handle)
    }
}

#[implement(IDataObject)]
struct DataObject {
    /// Clipboard format and its HGLOBAL contents; each `GetData` hands out a fresh copy.
    formats: Vec<(u16, Vec<u8>)>,
}

impl DataObject {
    fn find(&self, format: *const FORMATETC) -> Option<&[u8]> {
        let format = unsafe { format.as_ref()? };
        if format.tymed & TYMED_HGLOBAL.0 as u32 == 0 || format.dwAspect != DVASPECT_CONTENT.0 {
            return None;
        }
        self.formats.iter().find(|(cf, _)| *cf == format.cfFormat).map(|(_, bytes)| bytes.as_slice())
    }
}

impl IDataObject_Impl for DataObject_Impl {
    fn GetData(&self, format: *const FORMATETC) -> Result<STGMEDIUM> {
        let bytes = self.find(format).ok_or(Error::from(DV_E_FORMATETC))?;
        Ok(STGMEDIUM { tymed: TYMED_HGLOBAL.0 as u32, u: STGMEDIUM_0 { hGlobal: global(bytes)? }, pUnkForRelease: Default::default() })
    }

    fn GetDataHere(&self, _: *const FORMATETC, _: *mut STGMEDIUM) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn QueryGetData(&self, format: *const FORMATETC) -> HRESULT {
        if self.find(format).is_some() {
            S_OK
        } else {
            DV_E_FORMATETC
        }
    }

    fn GetCanonicalFormatEtc(&self, _: *const FORMATETC, out: *mut FORMATETC) -> HRESULT {
        if let Some(out) = unsafe { out.as_mut() } {
            out.ptd = std::ptr::null_mut();
        }
        E_NOTIMPL
    }

    /// Drop targets may store their own formats (drop descriptions); we don't keep them.
    fn SetData(&self, _: *const FORMATETC, _: *const STGMEDIUM, _: BOOL) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn EnumFormatEtc(&self, direction: u32) -> Result<IEnumFORMATETC> {
        if direction != DATADIR_GET.0 as u32 {
            return Err(E_NOTIMPL.into());
        }
        let formats: Vec<FORMATETC> = self
            .formats
            .iter()
            .map(|(cf, _)| FORMATETC { cfFormat: *cf, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 })
            .collect();
        unsafe { SHCreateStdEnumFmtEtc(&formats) }
    }

    fn DAdvise(&self, _: *const FORMATETC, _: u32, _: windows::core::Ref<IAdviseSink>) -> Result<u32> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }

    fn DUnadvise(&self, _: u32) -> Result<()> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }

    fn EnumDAdvise(&self) -> Result<IEnumSTATDATA> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }
}

#[implement(IDropSource)]
struct DropSource;

impl IDropSource_Impl for DropSource_Impl {
    fn QueryContinueDrag(&self, escape: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
        if escape.as_bool() {
            DRAGDROP_S_CANCEL
        } else if keys.0 & MK_LBUTTON.0 == 0 {
            DRAGDROP_S_DROP
        } else {
            S_OK
        }
    }

    fn GiveFeedback(&self, _: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hdrop_is_wide_and_double_terminated() {
        let bytes = hdrop(&[PathBuf::from("C:\\a.txt"), PathBuf::from("C:\\b")]);
        let header = size_of::<DROPFILES>();
        assert_eq!(u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize, header);
        let wide: Vec<u16> = bytes[header..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        assert_eq!(String::from_utf16_lossy(&wide), "C:\\a.txt\0C:\\b\0\0");
    }

    #[test]
    fn text_is_nul_terminated_utf16() {
        assert_eq!(utf16z("hé"), vec![b'h', 0, 0xE9, 0, 0, 0]);
    }
}
