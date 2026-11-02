//! ReUI's own shortcuts, each pointing at the hub next to Settings:
//! - the **Start menu entry** (Settings → General → Startup): `ReUI.lnk` in the Start menu's
//!   Programs folder. Launching it while ReUI runs opens Settings (the hub's single
//!   instance); without it a copy run from the zip or a build folder is hard to start again
//!   after Quit.
//! - **Send to → ReUI shelf** (Settings → Shelf): `ReUI shelf.lnk` in Explorer's Send to
//!   folder, running the hub with `--shelf` and the files (docs/PRD-send-to-shelf.md).
//!
//! The installer makes both (`reui-setup/src/shortcut.rs`, same names, same AppUserModelID),
//! so an installed ReUI shows them as on; a copy that wasn't installed has none until
//! they're turned on here.

use std::path::{Path, PathBuf};

use windows::core::{Interface, HSTRING, PWSTR};
use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemAlloc, CoTaskMemFree, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, STGM_READ};
use windows::Win32::System::Variant::VT_LPWSTR;
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::{FOLDERID_Programs, FOLDERID_SendTo, IShellLinkW, SHGetKnownFolderPath, ShellLink, KF_FLAG_DEFAULT};

use crate::paths::Paths;
use crate::system::same_path;

/// What the installer ties the entry to, so Windows groups it with ReUI's windows.
const APP_USER_MODEL_ID: &str = "ReUI.Hub";

/// Which of ReUI's shortcuts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    StartMenu,
    SendTo,
}

impl Which {
    /// Under the user's profile, where a sandbox keeps its own.
    fn folder(self) -> &'static str {
        match self {
            Which::StartMenu => r"AppData\Roaming\Microsoft\Windows\Start Menu\Programs",
            Which::SendTo => r"AppData\Roaming\Microsoft\Windows\SendTo",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Which::StartMenu => "ReUI.lnk",
            Which::SendTo => "ReUI shelf.lnk",
        }
    }

    /// The hub's arguments and the shortcut's description, as the installer writes them.
    fn launch(self) -> (&'static str, &'static str) {
        match self {
            Which::StartMenu => ("", "ReUI"),
            Which::SendTo => ("--shelf", "Hold on ReUI's shelf"),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Which::StartMenu => "the Start menu entry",
            Which::SendTo => "Send to → ReUI shelf",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartMenu {
    Off,
    On,
    /// The shortcut starts another `reui-hub.exe` (an installed copy, say).
    Elsewhere(String),
}

/// The shortcut's file; a sandbox keeps its own under its home.
pub fn entry(paths: &Paths, which: Which) -> PathBuf {
    let known = match which {
        Which::StartMenu => &FOLDERID_Programs,
        Which::SendTo => &FOLDERID_SendTo,
    };
    let folder = if paths.sandbox { None } else { known_folder(known) };
    folder.unwrap_or_else(|| paths.home.join(which.folder())).join(which.name())
}

pub fn state(paths: &Paths, which: Which) -> StartMenu {
    let lnk = entry(paths, which);
    if !lnk.exists() {
        return StartMenu::Off;
    }
    match target(&lnk) {
        Some(target) if same_file(&target, &paths.hub_exe()) => StartMenu::On,
        Some(target) => StartMenu::Elsewhere(target),
        // There but unreadable: it's ours to replace or remove all the same.
        None => StartMenu::On,
    }
}

/// Whether the entry's target is this hub. The shell may store a long path where ours is
/// a short (8.3) one, or the other way round: the files are compared, then the text.
fn same_file(target: &str, hub: &Path) -> bool {
    match (std::fs::canonicalize(target), std::fs::canonicalize(hub)) {
        (Ok(a), Ok(b)) => a == b,
        _ => same_path(target, &hub.to_string_lossy()),
    }
}

/// Adds the shortcut for the hub next to Settings (replacing one for another copy), or
/// removes it.
pub fn set(paths: &Paths, which: Which, on: bool) -> Result<(), String> {
    let lnk = entry(paths, which);
    if !on {
        return match std::fs::remove_file(&lnk) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("can't remove {} ({e})", which.label())),
            _ => Ok(()),
        };
    }
    let (args, description) = which.launch();
    create(&lnk, &paths.hub_exe(), args, description).map_err(|e| format!("can't add {} ({})", which.label(), e.message()))
}

fn com() {
    // S_FALSE (already initialised) and RPC_E_CHANGED_MODE are both fine here.
    _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
}

fn known_folder(id: &windows::core::GUID) -> Option<PathBuf> {
    unsafe {
        let raw = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let path = raw.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(raw.0 as *const _));
        path
    }
}

fn create(lnk: &Path, hub: &Path, args: &str, description: &str) -> windows::core::Result<()> {
    com();
    if let Some(dir) = lnk.parent() {
        _ = std::fs::create_dir_all(dir);
    }
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.SetPath(&HSTRING::from(hub.as_os_str()))?;
        if let Some(dir) = hub.parent() {
            link.SetWorkingDirectory(&HSTRING::from(dir.as_os_str()))?;
        }
        if !args.is_empty() {
            link.SetArguments(&HSTRING::from(args))?;
        }
        link.SetDescription(&HSTRING::from(description))?;
        link.SetIconLocation(&HSTRING::from(hub.as_os_str()), 0)?;

        let store: IPropertyStore = link.cast()?;
        // VT_LPWSTR owns a CoTaskMem string: PROPVARIANT's Drop frees it.
        let id: Vec<u16> = APP_USER_MODEL_ID.encode_utf16().chain(Some(0)).collect();
        let text = CoTaskMemAlloc(id.len() * 2) as *mut u16;
        if text.is_null() {
            return Err(windows::Win32::Foundation::E_OUTOFMEMORY.into());
        }
        std::ptr::copy_nonoverlapping(id.as_ptr(), text, id.len());
        let mut value = PROPVARIANT::default();
        (*value.Anonymous.Anonymous).vt = VT_LPWSTR;
        (*value.Anonymous.Anonymous).Anonymous.pwszVal = PWSTR(text);
        store.SetValue(&PKEY_AppUserModel_ID, &value)?;
        store.Commit()?;

        link.cast::<IPersistFile>()?.Save(&HSTRING::from(lnk.as_os_str()), true)
    }
}

fn target(lnk: &Path) -> Option<String> {
    com();
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        link.cast::<IPersistFile>().ok()?.Load(&HSTRING::from(lnk.as_os_str()), STGM_READ).ok()?;
        let mut buf = [0u16; 1024];
        let mut find = windows::Win32::Storage::FileSystem::WIN32_FIND_DATAW::default();
        link.GetPath(&mut buf, &mut find, 0).ok()?;
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..n])).filter(|t| !t.is_empty())
    }
}

/// A shortcut's arguments (`--shelf`), for the tests.
#[cfg(test)]
fn arguments(lnk: &Path) -> Option<String> {
    com();
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        link.cast::<IPersistFile>().ok()?.Load(&HSTRING::from(lnk.as_os_str()), STGM_READ).ok()?;
        let mut buf = [0u16; 1024];
        link.GetArguments(&mut buf).ok()?;
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..n]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_entry_is_added_read_back_and_removed() {
        let dir = std::env::temp_dir().join(format!("reui-shortcuts-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("reui-hub.exe"), b"").unwrap();
        let paths = Paths { config: dir.join("roaming"), local: dir.join("local"), home: dir.join("home"), exe_dir: dir.clone(), sandbox: true };
        let start = Which::StartMenu;
        assert_eq!(state(&paths, start), StartMenu::Off);
        set(&paths, start, true).unwrap();
        assert!(entry(&paths, start).starts_with(&paths.home), "a sandbox never touches the real Start menu");
        assert_eq!(state(&paths, start), StartMenu::On);
        // Another copy's entry reads as such.
        let other = dir.join("other");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("reui-hub.exe"), b"").unwrap();
        create(&entry(&paths, start), &other.join("reui-hub.exe"), "", "ReUI").unwrap();
        assert!(matches!(state(&paths, start), StartMenu::Elsewhere(p) if p.ends_with(r"other\reui-hub.exe")));
        set(&paths, start, false).unwrap();
        assert_eq!(state(&paths, start), StartMenu::Off);
        set(&paths, start, false).unwrap();

        // Send to: its own folder and name, the hub with --shelf.
        let send = Which::SendTo;
        assert_eq!(state(&paths, send), StartMenu::Off);
        set(&paths, send, true).unwrap();
        let lnk = entry(&paths, send);
        assert!(lnk.starts_with(&paths.home) && lnk.ends_with(r"SendTo\ReUI shelf.lnk"), "{}", lnk.display());
        assert_eq!(state(&paths, send), StartMenu::On);
        assert_eq!(state(&paths, start), StartMenu::Off, "one doesn't turn on the other");
        assert_eq!(arguments(&lnk).as_deref(), Some("--shelf"));
        set(&paths, send, false).unwrap();
        assert_eq!(state(&paths, send), StartMenu::Off);
        _ = std::fs::remove_dir_all(&dir);
    }
}
