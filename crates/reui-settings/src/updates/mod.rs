//! Updates from GitHub Releases: checking the latest release, downloading and verifying its
//! installer, and handing over to it. Also what the window knows across runs, in
//! `%LOCALAPPDATA%\ReUI\update.json`:
//!
//! - written here: `checked_ms`, `available` (a newer version, or null), `notes`,
//!   `setup_url`, `setup_size`, `sums_url`, `whats_new_seen`;
//! - written by the hub's updates module: `version` (the last version that ran),
//!   `updated_from` (set when that changed), `snoozed` / `snoozed_until` (the version the
//!   island's card was put off for, and until when).
//!
//! The hub runs `reui-settings --check-updates` (no window) once a day when automatic
//! checks are on, and `--install-update` ([`install`]) when Install is pressed in the
//! island; the About page checks and installs on request. Network only then, only to
//! api.github.com and the release's download links.

pub mod install;
pub mod net;
pub mod release;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{Map, Value};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

use self::release::{Asset, Release};
use crate::model::doc::write_atomic;
use crate::pages::VERSION;
use crate::paths::Paths;

/// The GitHub repository releases come from, as `owner/name`. TODO: set it once the
/// repository is public; until then update checks are skipped and "Report a bug" lets
/// GitHub ask for the repository.
pub const REPOSITORY: &str = "OWNER/REPO";

pub const STATE_FILE: &str = "update.json";
/// The window hears about progress through this message.
pub const WM_UPDATES: u32 = WM_APP + 3;

/// Whether [`REPOSITORY`] has been filled in.
pub fn repository_set() -> bool {
    let mut parts = REPOSITORY.split('/');
    let (owner, name) = (parts.next().unwrap_or_default(), parts.next().unwrap_or_default());
    !owner.is_empty() && !name.is_empty() && parts.next().is_none() && REPOSITORY != "OWNER/REPO"
}

/// `https://github.com/owner/name/`, or GitHub itself while the repository isn't set.
pub fn repository_url() -> String {
    if repository_set() {
        format!("https://github.com/{REPOSITORY}/")
    } else {
        "https://github.com/".into()
    }
}

fn latest_url() -> String {
    format!("https://api.github.com/repos/{REPOSITORY}/releases/latest")
}

/// Where the update stands, as the About page shows it.
#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    /// Bytes so far, of the asset's size.
    Downloading { release: Release, done: u64 },
    /// Downloaded and verified: run it.
    Ready(PathBuf),
    Failed(String),
}

/// Reads and merges into `update.json`.
pub fn read_state(paths: &Paths) -> Map<String, Value> {
    std::fs::read_to_string(paths.local.join(STATE_FILE)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|v| v.as_object().cloned()).unwrap_or_default()
}

pub fn write_state(paths: &Paths, pairs: Vec<(&str, Value)>) -> Result<(), String> {
    let mut map = read_state(paths);
    for (k, v) in pairs {
        map.insert(k.to_string(), v);
    }
    let path = paths.local.join(STATE_FILE);
    write_atomic(&path, &(serde_json::to_string_pretty(&Value::Object(map)).unwrap_or_default() + "\n")).map_err(|e| format!("can't write {}: {e}", path.display()))
}

/// What a check found, as stored: newer than this build, or nothing.
fn state_pairs(release: Option<&Release>) -> Vec<(&'static str, Value)> {
    let newer = release.filter(|r| release::newer(&r.version, VERSION));
    let asset = |a: Option<&Asset>, f: fn(&Asset) -> Value| a.map_or(Value::Null, f);
    vec![
        ("checked_ms", Value::from(reui_protocol::unix_ms())),
        ("available", newer.map_or(Value::Null, |r| Value::from(r.version.clone()))),
        ("notes", newer.map_or(Value::Null, |r| Value::from(r.notes.clone()))),
        ("setup_url", asset(newer.and_then(|r| r.setup.as_ref()), |a| Value::from(a.url.clone()))),
        ("setup_size", asset(newer.and_then(|r| r.setup.as_ref()), |a| Value::from(a.size))),
        ("sums_url", asset(newer.and_then(|r| r.sums.as_ref()), |a| Value::from(a.url.clone()))),
    ]
}

/// The newer release a past check found, if this build is still older than it.
pub fn stored_release(state: &Map<String, Value>) -> Option<Release> {
    let version = state.get("available")?.as_str()?.to_string();
    if !release::newer(&version, VERSION) {
        return None;
    }
    let str_of = |k: &str| state.get(k).and_then(Value::as_str).map(String::from);
    let setup = str_of("setup_url").map(|url| Asset { name: file_name(&url), url, size: state.get("setup_size").and_then(Value::as_u64).unwrap_or(0) });
    let sums = str_of("sums_url").map(|url| Asset { name: file_name(&url), url, size: 0 });
    Some(Release { version, notes: str_of("notes").unwrap_or_default(), setup, sums })
}

fn file_name(url: &str) -> String {
    url.rsplit('/').next().unwrap_or(url).to_string()
}

/// When the last check ran.
pub fn checked_ms(state: &Map<String, Value>) -> Option<u64> {
    state.get("checked_ms").and_then(Value::as_u64)
}

/// Asks GitHub for the latest release and records what it found.
pub fn check(paths: &Paths) -> Result<Option<Release>, String> {
    if !repository_set() {
        return Err("Update checks start once the GitHub repository is set".into());
    }
    let result = net::get_text(&latest_url(), Some("application/vnd.github+json")).and_then(|json| release::parse(&json));
    match &result {
        Ok(release) => write_state(paths, state_pairs(Some(release)))?,
        // Tried: don't try again before tomorrow either.
        Err(_) => write_state(paths, vec![("checked_ms", Value::from(reui_protocol::unix_ms()))])?,
    }
    result.map(|r| release::newer(&r.version, VERSION).then_some(r))
}

/// `--check-updates`: the hub's daily run, without a window.
pub fn check_quietly(paths: &Paths) {
    if let Err(e) = check(paths) {
        eprintln!("settings: update check: {e}");
    }
}

/// The download folder for one version: fresh, and holding nothing else.
fn download_dir(version: &str) -> PathBuf {
    std::env::temp_dir().join(format!("ReUI-update-{version}"))
}

/// How far [`fetch`] has got.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FetchStep {
    /// Bytes so far, reported about a hundred times over the whole download.
    Downloaded(u64),
    /// All there; checking it against the release.
    Verifying,
}

/// Downloads the release's installer and checks it against the release's size and, when
/// the release lists one, its SHA-256. The About page and `--install-update` both come
/// through here, so the island's Install verifies exactly what Settings' does.
pub fn fetch(release: &Release, mut report: impl FnMut(FetchStep)) -> Result<PathBuf, String> {
    let setup = release.setup.as_ref().ok_or("This release has no installer")?;
    let expected = match &release.sums {
        Some(sums) => Some(release::sha256_for(&net::get_text(&sums.url, None)?, &setup.name).ok_or("The release's checksums don't list its installer")?),
        None => None,
    };
    let dir = download_dir(&release.version);
    _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Can't make {}: {e}", dir.display()))?;
    let path = dir.join(&setup.name);
    let total = setup.size.max(1);
    let mut shown = 0u64;
    let (size, hash) = net::download(&setup.url, &path, |done| {
        if done.saturating_sub(shown) * 100 >= total {
            shown = done;
            report(FetchStep::Downloaded(done));
        }
    })?;
    report(FetchStep::Verifying);
    if let Err(e) = verify(size, &hash, setup.size, expected.as_deref()) {
        _ = std::fs::remove_dir_all(&dir);
        return Err(e);
    }
    Ok(path)
}

/// The downloaded file against what the release says it is.
pub fn verify(size: u64, hash: &str, expected_size: u64, expected_hash: Option<&str>) -> Result<(), String> {
    if expected_size > 0 && size != expected_size {
        return Err(format!("The download is incomplete ({size} of {expected_size} bytes)"));
    }
    if expected_hash.is_some_and(|h| !h.eq_ignore_ascii_case(hash)) {
        return Err("The download doesn't match the release's checksum".into());
    }
    Ok(())
}

/// The update in progress, shared with its worker thread.
#[derive(Clone)]
pub struct Updates {
    phase: Arc<Mutex<Phase>>,
}

impl Updates {
    pub fn new(paths: &Paths) -> Self {
        let phase = stored_release(&read_state(paths)).map_or(Phase::Idle, Phase::Available);
        Self { phase: Arc::new(Mutex::new(phase)) }
    }

    pub fn phase(&self) -> Phase {
        self.phase.lock().map(|p| p.clone()).unwrap_or(Phase::Idle)
    }

    fn set(&self, phase: Phase, hwnd: isize) {
        if let Ok(mut p) = self.phase.lock() {
            *p = phase;
        }
        unsafe { _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_UPDATES, WPARAM(0), LPARAM(0)) };
    }

    pub fn busy(&self) -> bool {
        matches!(self.phase(), Phase::Checking | Phase::Downloading { .. } | Phase::Ready(_))
    }

    /// Checks on a thread; the window is told when it's done.
    pub fn check(&self, paths: &Paths, hwnd: HWND) {
        if self.busy() {
            return;
        }
        let (me, paths, hwnd) = (self.clone(), paths.clone(), hwnd.0 as isize);
        me.set(Phase::Checking, hwnd);
        _ = std::thread::Builder::new().name("update-check".into()).spawn(move || {
            let phase = match check(&paths) {
                Ok(Some(release)) => Phase::Available(release),
                Ok(None) => Phase::UpToDate,
                Err(e) => Phase::Failed(e),
            };
            me.set(phase, hwnd);
        });
    }

    /// Downloads and verifies on a thread; [`Phase::Ready`] means run it.
    pub fn download(&self, hwnd: HWND) {
        let Phase::Available(release) = self.phase() else { return };
        let (me, hwnd) = (self.clone(), hwnd.0 as isize);
        me.set(Phase::Downloading { release: release.clone(), done: 0 }, hwnd);
        _ = std::thread::Builder::new().name("update-download".into()).spawn(move || {
            let result = fetch(&release, |step| {
                if let FetchStep::Downloaded(done) = step {
                    me.set(Phase::Downloading { release: release.clone(), done }, hwnd);
                }
            });
            me.set(
                match result {
                    Ok(path) => Phase::Ready(path),
                    Err(e) => Phase::Failed(e),
                },
                hwnd,
            );
        });
    }

    /// After a failure or a refusal to install: back to what's known.
    pub fn reset(&self, paths: &Paths) {
        let phase = stored_release(&read_state(paths)).map_or(Phase::Idle, Phase::Available);
        if let Ok(mut p) = self.phase.lock() {
            *p = phase;
        }
    }
}

/// Starts the installer quietly; it stops ReUI (this window too), installs and starts it
/// again. It gets none of our handles: `--install-update`'s stdout is the hub's pipe, which
/// would otherwise stay open until the setup ends.
pub fn run_installer(path: &Path) -> Result<(), String> {
    use std::process::Stdio;
    std::process::Command::new(path).arg("--quiet").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map(|_| ()).map_err(|e| format!("Can't start the installer: {e}"))
}

/// The CHANGELOG entry for this version, when this run follows an update and it hasn't
/// been shown yet.
pub fn whats_new(state: &Map<String, Value>) -> Option<String> {
    let updated = state.get("updated_from").and_then(Value::as_str).is_some_and(|from| from != VERSION);
    let current = state.get("version").and_then(Value::as_str) == Some(VERSION);
    let seen = state.get("whats_new_seen").and_then(Value::as_str) == Some(VERSION);
    (updated && current && !seen).then(|| release::changelog_entry(CHANGELOG, VERSION)).flatten()
}

pub const CHANGELOG: &str = include_str!("../../../../CHANGELOG.md");

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn paths(dir: &Path) -> Paths {
        Paths { config: dir.join("c"), local: dir.join("l"), home: dir.join("h"), exe_dir: dir.to_path_buf(), sandbox: true }
    }

    fn release(version: &str) -> Release {
        Release {
            version: version.into(),
            notes: "• Better".into(),
            setup: Some(Asset { name: format!("ReUI-Setup-{version}.exe"), url: format!("https://github.com/o/r/releases/download/v{version}/ReUI-Setup-{version}.exe"), size: 42 }),
            sums: None,
        }
    }

    #[test]
    fn the_placeholder_repository_skips_checks() {
        assert!(!repository_set());
        assert_eq!(repository_url(), "https://github.com/");
        let dir = crate::model::doc::tests::temp_dir("updates-skip");
        assert!(check(&paths(&dir)).unwrap_err().contains("repository"));
        assert!(!dir.join("l").join(STATE_FILE).exists(), "nothing recorded, nothing fetched");
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_found_release_is_stored_and_read_back_while_newer() {
        let dir = crate::model::doc::tests::temp_dir("updates-state");
        let paths = paths(&dir);
        write_state(&paths, vec![("version", json!(VERSION)), ("snoozed", json!("9.9.9"))]).unwrap();
        write_state(&paths, state_pairs(Some(&release("99.0.0")))).unwrap();
        let state = read_state(&paths);
        assert_eq!(state["snoozed"], "9.9.9", "the hub's keys are kept");
        assert!(checked_ms(&state).is_some());
        let stored = stored_release(&state).unwrap();
        assert_eq!(stored, release("99.0.0"));
        assert_eq!(Updates::new(&paths).phase(), Phase::Available(release("99.0.0")));

        // An older or equal release isn't "available".
        write_state(&paths, state_pairs(Some(&release(VERSION)))).unwrap();
        assert_eq!(read_state(&paths)["available"], Value::Null);
        assert_eq!(stored_release(&read_state(&paths)), None);
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn downloads_are_checked_against_the_release() {
        let hash = "ab".repeat(32);
        assert!(verify(42, &hash, 42, Some(&hash.to_uppercase())).is_ok());
        assert!(verify(42, &hash, 0, None).is_ok(), "nothing to compare with");
        assert!(verify(41, &hash, 42, None).unwrap_err().contains("incomplete"));
        assert!(verify(42, &hash, 42, Some(&"cd".repeat(32))).unwrap_err().contains("checksum"));
    }

    #[test]
    fn whats_new_shows_once_after_an_update() {
        let entry = release::changelog_entry(CHANGELOG, VERSION);
        assert!(entry.is_some(), "CHANGELOG.md has an entry for {VERSION}");
        let state = |v: Value| v.as_object().unwrap().clone();
        assert_eq!(whats_new(&state(json!({ "version": VERSION }))), None, "a fresh install");
        assert_eq!(whats_new(&state(json!({ "version": VERSION, "updated_from": "0.0.1" }))), entry);
        assert_eq!(whats_new(&state(json!({ "version": VERSION, "updated_from": "0.0.1", "whats_new_seen": VERSION }))), None);
        assert_eq!(whats_new(&state(json!({ "version": "0.0.2", "updated_from": "0.0.1" }))), None, "the hub hasn't caught up");
    }
}
