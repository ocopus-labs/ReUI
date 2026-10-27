//! Install, upgrade and uninstall end to end against a temp folder and a throwaway
//! `HKCU\Software\ReUI-test-*` key, with a recording [`System`] (nothing is stopped,
//! launched or run for real).

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use crate::install::{install, Options};
use crate::layout::{quoted, Layout, SETUP_EXE};
use crate::payload::File;
use crate::registry::{self, tests::TestKey};
use crate::system::System;
use crate::uninstall::{self, Agent, Step};

#[derive(Default)]
struct Recorder(Vec<String>);

impl System for Recorder {
    fn stop_reui(&mut self, dir: &Path) {
        self.0.push(format!("stop {}", dir.display()));
    }
    fn run(&mut self, exe: &Path, args: &[&str]) -> Result<(), String> {
        self.0.push(format!("run {} {}", exe.file_name().unwrap().to_string_lossy(), args.join(" ")));
        Ok(())
    }
    fn launch(&mut self, exe: &Path) -> Result<(), String> {
        self.0.push(format!("launch {}", exe.file_name().unwrap().to_string_lossy()));
        Ok(())
    }
    fn delete_after_exit(&mut self, dir: &Path) {
        self.0.push("delete later".into());
        _ = std::fs::remove_dir_all(dir);
    }
}

fn files(tag: &'static [u8]) -> Vec<File> {
    crate::payload_gen::PROGRAMS.iter().map(|&name| File { name, bytes: Cow::Borrowed(tag) }).collect()
}

fn root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("reui-setup-flow-{tag}-{}", std::process::id()));
    _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn install_upgrade_uninstall() {
    let key = TestKey::new("flow");
    let root = root("full");
    let layout = Layout::under(&root, &key.0);
    let download = root.join("Downloads").join("ReUI-Setup-0.1.0.exe");
    std::fs::create_dir_all(download.parent().unwrap()).unwrap();
    std::fs::write(&download, b"setup v1").unwrap();

    // Fresh install.
    let mut sys = Recorder::default();
    let outcome = install(&layout, &files(b"v1"), &download, Options::default(), &mut sys).unwrap();
    assert!(!outcome.upgrade && outcome.autostart);
    assert_eq!(sys.0, [format!("stop {}", layout.install_dir.display()), "launch reui-hub.exe".into()]);
    assert_eq!(std::fs::read(layout.exe("reui-hub.exe")).unwrap(), b"v1");
    assert_eq!(std::fs::read(layout.exe(SETUP_EXE)).unwrap(), b"setup v1");
    assert_eq!(crate::shortcut::target(&layout.shortcut()).unwrap().to_string_lossy().to_lowercase(), layout.exe("reui-hub.exe").to_string_lossy().to_lowercase());
    assert!(registry::has_uninstall_entry(&layout));
    assert_eq!(registry::autostart(&layout), Some(quoted(&layout.exe("reui-hub.exe"))));

    // User data, and the user turning start-with-Windows off; an upgrade keeps both.
    std::fs::create_dir_all(&layout.roaming_data).unwrap();
    std::fs::write(layout.roaming_data.join("clipboard.json"), b"{}").unwrap();
    registry::remove_autostart(&layout).unwrap();
    let mut sys = Recorder::default();
    let outcome = install(&layout, &files(b"v2"), &download, Options { no_launch: true, ..Default::default() }, &mut sys).unwrap();
    assert!(outcome.upgrade && !outcome.autostart);
    assert_eq!(sys.0.len(), 1, "stopped, not launched: {:?}", sys.0);
    assert_eq!(std::fs::read(layout.exe("reui-island.exe")).unwrap(), b"v2");
    assert_eq!(registry::autostart(&layout), None);
    assert!(layout.roaming_data.join("clipboard.json").is_file());
    let mut leftovers: Vec<_> = std::fs::read_dir(&layout.install_dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    leftovers.sort();
    assert_eq!(leftovers, ["reui-bridge.exe", "reui-hub.exe", "reui-island.exe", "reui-settings.exe", "reui-setup.exe"]);

    // Uninstall from the installed copy (as Apps & features does), keeping data.
    std::fs::create_dir_all(layout.bridge_copy_dir()).unwrap();
    std::fs::write(layout.bridge_copy_dir().join("reui-bridge.exe"), b"copy").unwrap();
    let steps = uninstall::plan(&[Agent::Claude], false);
    let mut sys = Recorder::default();
    let failures = uninstall::run(&layout, &steps, &layout.exe(SETUP_EXE), &mut sys);
    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(sys.0[1..], ["run reui-hub.exe --restore-flyout", "run reui-bridge.exe uninstall claude", "delete later"]);
    assert!(!layout.install_dir.exists());
    assert!(!layout.shortcut().exists());
    assert!(!registry::has_uninstall_entry(&layout));
    assert!(!layout.bridge_copy_dir().exists());
    assert!(layout.roaming_data.join("clipboard.json").is_file(), "data kept");

    // Uninstalling again (from a downloaded setup), now deleting data: nothing left.
    let failures = uninstall::run(&layout, &uninstall::plan(&[], true), &download, &mut Recorder::default());
    assert!(failures.is_empty(), "{failures:?}");
    assert!(!layout.roaming_data.exists() && !layout.local_data.exists());
    _ = std::fs::remove_dir_all(root);
    drop(key);
}

#[test]
fn no_autostart_on_a_fresh_install() {
    let key = TestKey::new("noauto");
    let root = root("noauto");
    let layout = Layout::under(&root, &key.0);
    let setup = root.join("setup.exe");
    std::fs::write(&setup, b"s").unwrap();
    let outcome = install(&layout, &files(b"v1"), &setup, Options { no_autostart: true, no_launch: true }, &mut Recorder::default()).unwrap();
    assert!(!outcome.autostart);
    assert_eq!(registry::autostart(&layout), None);
    // Clean up through the real steps (no hooks in this fake home).
    assert!(uninstall::hooked_agents(&layout.home).is_empty());
    let steps = uninstall::plan(&[], true);
    assert!(!steps.contains(&Step::RemoveHooks(Agent::Claude)));
    assert!(uninstall::run(&layout, &steps, &setup, &mut Recorder::default()).is_empty());
    assert!(!layout.install_dir.exists());
    _ = std::fs::remove_dir_all(root);
    drop(key);
}
