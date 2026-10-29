//! The Windows side of the shelf's actions and of what its cards show: opening a file or
//! a link, Explorer with files selected, the clipboard, and what the disk says (missing
//! files, a picture of an image, a file type's icon). Everything here blocks (the shell,
//! COM, the disk, maybe a network share), so the module calls it from blocking threads.

use std::collections::HashMap;
use std::path::Path;

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{ILFree, SHOpenFolderAndSelectItems, SHParseDisplayName, ShellExecuteW};
use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, HWND_MESSAGE, SW_SHOWNORMAL, WINDOW_EX_STYLE, WINDOW_STYLE};

use super::store::{Held, What};
use crate::modules::clipboard::{exe_icon, thumbnail};

/// Pictures of image files are made only for files up to this size.
const THUMB_MAX_FILE: u64 = 64 * 1024 * 1024;
const THUMB_PX: u32 = 360;
const IMAGE_TYPES: &[&str] = &["png", "jpg", "jpeg", "gif", "bmp", "webp", "tif", "tiff", "heic"];

/// Opens a file, folder or link the way a double-click in Explorer would.
pub fn open(target: &str) {
    unsafe {
        _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let result = ShellExecuteW(None, w!("open"), &HSTRING::from(target), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
        if result.0 as isize <= 32 {
            eprintln!("shelf: can't open {target} ({})", result.0 as isize);
        }
    }
}

/// Explorer, in the first file's folder, with those of `paths` that are in it selected.
pub fn show_in_folder(paths: &[String]) {
    let Some(first) = paths.first() else { return };
    let Some(folder) = Path::new(first).parent().map(|p| p.to_string_lossy().into_owned()) else { return };
    unsafe {
        _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let Some(parent) = pidl(&folder) else { return open(&folder) };
        let children: Vec<*mut ITEMIDLIST> = paths.iter().filter(|p| Path::new(p).parent().map(|d| d.to_string_lossy() == folder).unwrap_or(false)).filter_map(|p| pidl(p)).collect();
        let selected: Vec<*const ITEMIDLIST> = children.iter().map(|p| *p as *const _).collect();
        if let Err(e) = SHOpenFolderAndSelectItems(parent, Some(&selected), 0) {
            eprintln!("shelf: can't show {first} in its folder: {e}");
        }
        for child in children {
            ILFree(Some(child));
        }
        ILFree(Some(parent));
    }
}

unsafe fn pidl(path: &str) -> Option<*mut ITEMIDLIST> {
    let mut out = std::ptr::null_mut();
    SHParseDisplayName(&HSTRING::from(path), None, &mut out, 0, None).ok()?;
    (!out.is_null()).then_some(out)
}

/// Puts `text` on the clipboard. The clipboard wants an owner window for that, so a
/// message-only one exists for the moment it takes.
pub fn copy_text(text: &str) {
    unsafe {
        let Ok(owner) = CreateWindowExW(WINDOW_EX_STYLE(0), w!("STATIC"), w!("ReUI shelf"), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, None, None) else { return };
        if put_text(owner, text).is_none() {
            eprintln!("shelf: can't copy to the clipboard");
        }
        _ = DestroyWindow(owner);
    }
}

unsafe fn put_text(owner: HWND, text: &str) -> Option<()> {
    let units: Vec<u16> = text.encode_utf16().chain([0]).collect();
    OpenClipboard(Some(owner)).ok()?;
    let done = (|| {
        EmptyClipboard().ok()?;
        let global: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, units.len() * 2).ok()?;
        let at = GlobalLock(global) as *mut u16;
        if at.is_null() {
            return None;
        }
        std::ptr::copy_nonoverlapping(units.as_ptr(), at, units.len());
        _ = GlobalUnlock(global);
        // The clipboard owns the memory from here.
        SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(global.0))).ok().map(|_| ())
    })();
    _ = CloseClipboard();
    done
}

/// What the disk says about held things: kept between lookups, so a picture or an icon is
/// made once per file (or file type), while whether a file is there is asked every time.
#[derive(Default)]
pub struct Looks {
    thumbs: HashMap<String, Option<Vec<u8>>>,
    icons: HashMap<String, Option<Vec<u8>>>,
}

/// Bytes the module registers as images.
#[derive(Default)]
pub struct Seen {
    pub missing: Vec<String>,
    pub thumbs: Vec<(u64, Vec<u8>)>,
    pub icons: Vec<(u64, Vec<u8>)>,
}

impl Looks {
    pub fn look(&mut self, items: &[Held]) -> Seen {
        let mut seen = Seen::default();
        for held in items {
            let What::Files { paths, .. } = &held.what else { continue };
            for path in paths {
                if std::fs::metadata(path).is_err() {
                    seen.missing.push(path.clone());
                }
            }
            let Some(first) = paths.first().filter(|p| !seen.missing.contains(p)) else { continue };
            let ext = Path::new(first).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            if paths.len() == 1 && IMAGE_TYPES.contains(&ext.as_str()) {
                let thumb = self.thumbs.entry(first.clone()).or_insert_with(|| picture(first));
                if let Some(png) = thumb {
                    seen.thumbs.push((held.id, png.clone()));
                }
            }
            // Folders and programs have their own icons; other files share their type's.
            let key = if ext.is_empty() || ext == "exe" || ext == "lnk" { first.clone() } else { ext };
            if let Some(png) = self.icons.entry(key).or_insert_with(|| exe_icon(first)) {
                seen.icons.push((held.id, png.clone()));
            }
        }
        seen
    }
}

fn picture(path: &str) -> Option<Vec<u8>> {
    let size = std::fs::metadata(path).ok()?.len();
    if size > THUMB_MAX_FILE {
        return None;
    }
    thumbnail(&std::fs::read(path).ok()?, THUMB_PX)
}
