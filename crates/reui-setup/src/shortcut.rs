//! The Start menu entry: a `.lnk` to the hub with ReUI's AppUserModelID, so Windows groups
//! it with ReUI's windows and launching it again goes through the hub's single instance
//! (which opens Settings).

use std::path::{Path, PathBuf};

use windows::core::{Interface, HSTRING, PWSTR};
use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemAlloc, CoTaskMemFree, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Variant::VT_LPWSTR;
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::{FOLDERID_Programs, IShellLinkW, SHGetKnownFolderPath, ShellLink, KF_FLAG_DEFAULT};

fn com() {
    // S_FALSE (already initialised) and RPC_E_CHANGED_MODE are both fine for our use.
    _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
}

/// The current user's Start menu Programs folder.
pub fn programs_folder() -> Option<PathBuf> {
    unsafe {
        let raw = SHGetKnownFolderPath(&FOLDERID_Programs, KF_FLAG_DEFAULT, None).ok()?;
        let path = raw.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(raw.0 as *const _));
        path
    }
}

/// Creates (or replaces) `lnk` pointing at `target`.
pub fn create(lnk: &Path, target: &Path, description: &str, app_id: &str) -> windows::core::Result<()> {
    com();
    if let Some(dir) = lnk.parent() {
        _ = std::fs::create_dir_all(dir);
    }
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.SetPath(&HSTRING::from(target.as_os_str()))?;
        if let Some(dir) = target.parent() {
            link.SetWorkingDirectory(&HSTRING::from(dir.as_os_str()))?;
        }
        link.SetDescription(&HSTRING::from(description))?;
        link.SetIconLocation(&HSTRING::from(target.as_os_str()), 0)?;

        let store: IPropertyStore = link.cast()?;
        // VT_LPWSTR owns a CoTaskMem string: PROPVARIANT's Drop frees it (PropVariantClear).
        let id: Vec<u16> = app_id.encode_utf16().chain(Some(0)).collect();
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

/// The target of an existing shortcut.
#[cfg(test)]
pub fn target(lnk: &Path) -> windows::core::Result<PathBuf> {
    com();
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.cast::<IPersistFile>()?.Load(&HSTRING::from(lnk.as_os_str()), windows::Win32::System::Com::STGM_READ)?;
        let mut buf = [0u16; 1024];
        let mut find = windows::Win32::Storage::FileSystem::WIN32_FIND_DATAW::default();
        link.GetPath(&mut buf, &mut find, 0)?;
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Ok(PathBuf::from(String::from_utf16_lossy(&buf[..n])))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_round_trips_in_a_temp_folder() {
        let dir = std::env::temp_dir().join(format!("reui-setup-lnk-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&dir);
        let target_exe = dir.join("Programs").join("reui-hub.exe");
        std::fs::create_dir_all(target_exe.parent().unwrap()).unwrap();
        std::fs::write(&target_exe, b"MZ").unwrap();
        let lnk = dir.join("Start").join("ReUI.lnk");

        create(&lnk, &target_exe, "ReUI", "ReUI.Test").unwrap();
        assert!(lnk.is_file());
        assert_eq!(target(&lnk).unwrap().to_string_lossy().to_lowercase(), target_exe.to_string_lossy().to_lowercase());
        // Replacing an existing one works (upgrades).
        create(&lnk, &target_exe, "ReUI", "ReUI.Test").unwrap();
        _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn programs_folder_exists() {
        assert!(programs_folder().is_some_and(|p| p.ends_with(r"Start Menu\Programs")));
    }
}
