//! The Start menu entry (Settings → General → Startup): `ReUI.lnk` in the user's Start menu
//! Programs folder, pointing at the hub next to Settings. The installer makes the same
//! entry (`reui-setup/src/shortcut.rs`, same name, same AppUserModelID), so an installed
//! ReUI shows it as on; a copy run from the zip or a build folder has none until it's
//! turned on here, and is otherwise impossible to start again after Quit without finding
//! the exe. Launching the entry while ReUI runs opens Settings (the hub's single instance).

use std::path::{Path, PathBuf};

use windows::core::{Interface, HSTRING, PWSTR};
use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemAlloc, CoTaskMemFree, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, STGM_READ};
use windows::Win32::System::Variant::VT_LPWSTR;
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::{FOLDERID_Programs, IShellLinkW, SHGetKnownFolderPath, ShellLink, KF_FLAG_DEFAULT};

use crate::paths::Paths;
use crate::system::same_path;

/// What the installer ties the entry to, so Windows groups it with ReUI's windows.
const APP_USER_MODEL_ID: &str = "ReUI.Hub";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartMenu {
    Off,
    On,
    /// The entry starts another `reui-hub.exe` (an installed copy, say).
    Elsewhere(String),
}

/// `ReUI.lnk` in the Start menu; a sandbox keeps its own under its home.
pub fn entry(paths: &Paths) -> PathBuf {
    let programs = if paths.sandbox {
        paths.home.join(r"AppData\Roaming\Microsoft\Windows\Start Menu\Programs")
    } else {
        programs_folder().unwrap_or_else(|| paths.home.join(r"AppData\Roaming\Microsoft\Windows\Start Menu\Programs"))
    };
    programs.join("ReUI.lnk")
}

pub fn state(paths: &Paths) -> StartMenu {
    let lnk = entry(paths);
    if !lnk.exists() {
        return StartMenu::Off;
    }
    match target(&lnk) {
        Some(target) if same_path(&target, &paths.hub_exe().to_string_lossy()) => StartMenu::On,
        Some(target) => StartMenu::Elsewhere(target),
        // There but unreadable: it's ours to replace or remove all the same.
        None => StartMenu::On,
    }
}

/// Adds the entry for the hub next to Settings (replacing one for another copy), or
/// removes it.
pub fn set(paths: &Paths, on: bool) -> Result<(), String> {
    let lnk = entry(paths);
    if !on {
        return match std::fs::remove_file(&lnk) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("can't remove the Start menu entry ({e})")),
            _ => Ok(()),
        };
    }
    create(&lnk, &paths.hub_exe()).map_err(|e| format!("can't add the Start menu entry ({})", e.message()))
}

fn com() {
    // S_FALSE (already initialised) and RPC_E_CHANGED_MODE are both fine here.
    _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
}

fn programs_folder() -> Option<PathBuf> {
    unsafe {
        let raw = SHGetKnownFolderPath(&FOLDERID_Programs, KF_FLAG_DEFAULT, None).ok()?;
        let path = raw.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(raw.0 as *const _));
        path
    }
}

fn create(lnk: &Path, hub: &Path) -> windows::core::Result<()> {
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
        link.SetDescription(&HSTRING::from("ReUI"))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_entry_is_added_read_back_and_removed() {
        let dir = std::env::temp_dir().join(format!("reui-start-menu-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("reui-hub.exe"), b"").unwrap();
        let paths = Paths { config: dir.join("roaming"), local: dir.join("local"), home: dir.join("home"), exe_dir: dir.clone(), sandbox: true };
        assert_eq!(state(&paths), StartMenu::Off);
        set(&paths, true).unwrap();
        assert!(entry(&paths).starts_with(&paths.home), "a sandbox never touches the real Start menu");
        assert_eq!(state(&paths), StartMenu::On);
        // Another copy's entry reads as such.
        let other = dir.join("other");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("reui-hub.exe"), b"").unwrap();
        create(&entry(&paths), &other.join("reui-hub.exe")).unwrap();
        assert!(matches!(state(&paths), StartMenu::Elsewhere(p) if p.ends_with(r"other\reui-hub.exe")));
        set(&paths, false).unwrap();
        assert_eq!(state(&paths), StartMenu::Off);
        set(&paths, false).unwrap();
        _ = std::fs::remove_dir_all(&dir);
    }
}
