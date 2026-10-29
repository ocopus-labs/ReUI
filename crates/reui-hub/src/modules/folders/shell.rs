//! The shell calls the file moments share: where the user's Downloads and Screenshots
//! folders are, Open, Show in folder, and a file's icon. [`known`] and [`icon`] block (COM,
//! the shell), so callers run them off the hub thread; Open and Show in folder take a
//! thread of their own.

use std::path::{Path, PathBuf};

use windows::core::{GUID, HSTRING, PCWSTR};
use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::{ILCreateFromPathW, ILFree, SHGetKnownFolderPath, SHOpenFolderAndSelectItems, ShellExecuteW, FOLDERID_Downloads, FOLDERID_Screenshots, KF_FLAG_DEFAULT};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// A known folder the file moments watch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Known {
    Downloads,
    Screenshots,
}

impl Known {
    /// A sandboxed test hub (`REUI_CONFIG_DIR`) may point this at a temp folder, so tests
    /// never watch, or leave files in, the user's own.
    fn override_var(self) -> &'static str {
        match self {
            Known::Downloads => "REUI_DOWNLOADS_DIR",
            Known::Screenshots => "REUI_SCREENSHOTS_DIR",
        }
    }

    fn id(self) -> GUID {
        match self {
            Known::Downloads => FOLDERID_Downloads,
            Known::Screenshots => FOLDERID_Screenshots,
        }
    }
}

/// Where `folder` is (the user may have moved it, to another drive say). Not created when
/// it doesn't exist yet: `Pictures\Screenshots` appears with the first Win+PrtScn.
pub fn known(folder: Known) -> Option<PathBuf> {
    if crate::paths::sandbox().is_some() {
        if let Some(dir) = std::env::var_os(folder.override_var()).filter(|v| !v.is_empty()) {
            return Some(PathBuf::from(dir));
        }
    }
    unsafe {
        let raw = SHGetKnownFolderPath(&folder.id(), KF_FLAG_DEFAULT, None).ok()?;
        let path = raw.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(raw.0 as *const _));
        path
    }
}

/// Opens the file with its app, as a double-click in Explorer would. Returns at once: the
/// shell runs on a thread of its own (see [`shell_thread`]).
pub fn open(path: PathBuf) {
    shell_thread(move || execute(&path));
}

/// Opens the file's folder in Explorer with the file selected (an Explorer window already
/// showing that folder is reused). Returns at once, like [`open`].
pub fn show_in_folder(path: PathBuf) {
    shell_thread(move || unsafe {
        let item = ILCreateFromPathW(&HSTRING::from(path.as_os_str()));
        if item.is_null() {
            // Gone since: the folder at least.
            if let Some(parent) = path.parent() {
                execute(parent);
            }
            return;
        }
        if SHOpenFolderAndSelectItems(item, None, 0).is_err() {
            eprintln!("folders: couldn't show {}", path.display());
        }
        ILFree(Some(item));
    });
}

/// The shell wants a single-threaded apartment, which the blocking pool's threads (COM
/// initialised as they come, for WIC) can't promise; these are rare, user-asked calls, so a
/// short-lived thread each is cheaper than getting that wrong.
fn shell_thread(work: impl FnOnce() + Send + 'static) {
    let spawned = std::thread::Builder::new().name("reui-shell".into()).spawn(move || {
        _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        work();
    });
    if let Err(e) = spawned {
        eprintln!("folders: no thread for the shell: {e}");
    }
}

fn execute(path: &Path) {
    let result = unsafe { ShellExecuteW(None, &HSTRING::from("open"), &HSTRING::from(path.as_os_str()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
    if result.0 as isize <= 32 {
        eprintln!("folders: couldn't open {}", path.display());
    }
}

/// The icon Explorer shows for the file (its type's, or an exe's own), as a PNG.
pub fn icon(path: &Path) -> Option<Vec<u8>> {
    crate::modules::clipboard::exe_icon(&path.to_string_lossy())
}
