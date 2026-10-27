//! The HKCU entries: Apps & features (`Uninstall\ReUI`) and start with Windows (`Run`).
//! Key paths come from the [`Layout`](crate::layout::Layout), so tests use a throwaway key.

use std::path::Path;

use windows_registry::CURRENT_USER;

use crate::layout::{quoted, Layout, APP, PUBLISHER, SETUP_EXE, VERSION};

/// The Apps & features entry.
pub fn write_uninstall_entry(layout: &Layout, size_bytes: u64, icon: &Path) -> windows_registry::Result<()> {
    let key = CURRENT_USER.create(layout.uninstall_key())?;
    let setup = quoted(&layout.exe(SETUP_EXE));
    key.set_string("DisplayName", APP)?;
    key.set_string("DisplayVersion", VERSION)?;
    key.set_string("Publisher", PUBLISHER)?;
    key.set_string("DisplayIcon", format!("{},0", quoted(icon)))?;
    key.set_string("InstallLocation", layout.install_dir.to_string_lossy())?;
    key.set_string("UninstallString", format!("{setup} --uninstall"))?;
    key.set_string("QuietUninstallString", format!("{setup} --uninstall --quiet"))?;
    key.set_u32("NoModify", 1)?;
    key.set_u32("NoRepair", 1)?;
    key.set_u32("EstimatedSize", size_bytes.div_ceil(1024).min(u32::MAX as u64) as u32)?;
    Ok(())
}

pub fn has_uninstall_entry(layout: &Layout) -> bool {
    CURRENT_USER.open(layout.uninstall_key()).is_ok()
}

pub fn remove_uninstall_entry(layout: &Layout) -> windows_registry::Result<()> {
    ignore_missing(CURRENT_USER.remove_tree(layout.uninstall_key()))
}

/// The `Run` value, if any.
pub fn autostart(layout: &Layout) -> Option<String> {
    CURRENT_USER.open(layout.run_key()).ok()?.get_string(APP).ok()
}

pub fn set_autostart(layout: &Layout, hub: &Path) -> windows_registry::Result<()> {
    CURRENT_USER.create(layout.run_key())?.set_string(APP, quoted(hub))
}

pub fn remove_autostart(layout: &Layout) -> windows_registry::Result<()> {
    match CURRENT_USER.options().read().write().open(layout.run_key()) {
        Ok(key) => ignore_missing(key.remove_value(APP)),
        Err(e) => ignore_missing(Err(e)),
    }
}

/// Should `Run\ReUI` exist after this install? A fresh install starts with Windows unless
/// `--no-autostart`; an upgrade keeps whatever the user chose (Settings may have turned it
/// off), and `--no-autostart` turns it off either way.
pub fn want_autostart(upgrade: bool, currently_on: bool, no_autostart: bool) -> bool {
    !no_autostart && (!upgrade || currently_on)
}

fn ignore_missing(result: windows_registry::Result<()>) -> windows_registry::Result<()> {
    const FILE_NOT_FOUND: windows::core::HRESULT = windows::core::HRESULT(0x8007_0002u32 as i32);
    match result {
        Err(e) if e.code() == FILE_NOT_FOUND => Ok(()),
        other => other,
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A unique `HKCU\Software\ReUI-test-*` base, deleted on drop.
    pub struct TestKey(pub String);

    impl TestKey {
        pub fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos();
            Self(format!(r"Software\ReUI-test-{tag}-{}-{nanos}", std::process::id()))
        }
    }

    impl Drop for TestKey {
        fn drop(&mut self) {
            _ = CURRENT_USER.remove_tree(&self.0);
        }
    }

    #[test]
    fn autostart_choice() {
        assert!(want_autostart(false, false, false), "fresh install: on");
        assert!(!want_autostart(false, false, true), "fresh with --no-autostart");
        assert!(want_autostart(true, true, false), "upgrade keeps on");
        assert!(!want_autostart(true, false, false), "upgrade keeps the user's off");
        assert!(!want_autostart(true, true, true), "--no-autostart wins");
    }

    #[test]
    fn entries_round_trip_under_a_test_key() {
        let key = TestKey::new("registry");
        let root = std::env::temp_dir().join("reui-setup-registry");
        let layout = Layout::under(&root, &key.0);

        assert!(!has_uninstall_entry(&layout));
        assert_eq!(autostart(&layout), None);
        remove_autostart(&layout).unwrap(); // nothing there: fine
        remove_uninstall_entry(&layout).unwrap();

        write_uninstall_entry(&layout, 5 * 1024 * 1024 + 1, &layout.exe("reui-hub.exe")).unwrap();
        assert!(has_uninstall_entry(&layout));
        let entry = CURRENT_USER.open(layout.uninstall_key()).unwrap();
        assert_eq!(entry.get_string("DisplayName").unwrap(), "ReUI");
        assert_eq!(entry.get_string("Publisher").unwrap(), "Rohit Kushwaha");
        assert_eq!(entry.get_string("DisplayVersion").unwrap(), VERSION);
        assert_eq!(entry.get_u32("EstimatedSize").unwrap(), 5 * 1024 + 1);
        assert_eq!(entry.get_u32("NoModify").unwrap(), 1);
        let uninstall = entry.get_string("UninstallString").unwrap();
        assert!(uninstall.starts_with('"') && uninstall.ends_with(r#"\reui-setup.exe" --uninstall"#), "{uninstall}");
        assert!(entry.get_string("QuietUninstallString").unwrap().ends_with("--uninstall --quiet"));
        assert_eq!(entry.get_string("InstallLocation").unwrap(), layout.install_dir.to_string_lossy());

        set_autostart(&layout, &layout.exe("reui-hub.exe")).unwrap();
        assert_eq!(autostart(&layout), Some(quoted(&layout.exe("reui-hub.exe"))));
        remove_autostart(&layout).unwrap();
        assert_eq!(autostart(&layout), None);

        remove_uninstall_entry(&layout).unwrap();
        assert!(!has_uninstall_entry(&layout));
        drop(key);
    }
}
