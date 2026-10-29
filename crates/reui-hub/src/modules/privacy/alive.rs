//! Which consent-store uses still have a process behind them. Pure.
//!
//! The store is written by the app's own session with the capability broker. An app that
//! crashes, or is killed, can leave its stop at 0 for good (seen on this machine: a Chrome
//! screen capture "in use" for days), and a process that exits writes nothing at all, so
//! the registry alone would keep a dot on forever. A use counts only while a process of
//! that app is running that already existed when the use started; a newer instance of the
//! same app (Chrome restarted after the crash) doesn't keep a dead session alive.

use super::consent::{App, Use};

/// Registry and process clocks differ by a little (the store is written just after the
/// process asked for the device).
const SLACK: u64 = 20_000_000; // 2 s in FILETIME units

/// A running process, as far as it could be read. Elevated and protected processes may
/// refuse `PROCESS_QUERY_LIMITED_INFORMATION`: then only the snapshot's file name is known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proc {
    pub pid: u32,
    /// Lowercased file name ("chrome.exe").
    pub name: String,
    pub path: Option<String>,
    /// Creation time (FILETIME).
    pub created: Option<u64>,
}

/// A use with the processes that may hold it: waited on, so a process exit is noticed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Live {
    pub using: Use,
    pub pids: Vec<u32>,
    /// A full exe path to take the icon from (the packaged app's own exe for packaged apps).
    pub exe: Option<String>,
}

/// `Microsoft.WindowsCamera_8wekyb3d8bbwe` matches an exe under
/// `…\WindowsApps\Microsoft.WindowsCamera_2025.2510.1.0_x64__8wekyb3d8bbwe\…`: the package
/// folder is `Name_Version_Arch_Resource_PublisherId`, the family `Name_PublisherId`.
fn in_package(family: &str, path: &str) -> bool {
    let Some((name, publisher)) = family.rsplit_once('_') else { return false };
    let (name, publisher) = (name.to_lowercase(), publisher.to_lowercase());
    path.to_lowercase().split('\\').any(|part| part.starts_with(&format!("{name}_")) && part.ends_with(&format!("_{publisher}")))
}

/// Whether the process is this app; `None` when it can't be told (no path).
fn is_app(app: &App, p: &Proc) -> Option<bool> {
    match (app, &p.path) {
        (App::Exe(exe), Some(path)) => Some(exe.eq_ignore_ascii_case(path)),
        (App::Exe(_), None) => Some(app.file_name().as_deref() == Some(p.name.as_str())),
        (App::Packaged(family), Some(path)) => Some(in_package(family, path)),
        (App::Packaged(_), None) => None,
    }
}

/// The uses a running process accounts for. A matching process whose start time can't be
/// read (elevated) is trusted: there's no way to tell, and hiding a live camera is worse
/// than a stale dot.
pub fn alive(uses: &[Use], procs: &[Proc]) -> Vec<Live> {
    uses.iter()
        .filter_map(|u| {
            let holders: Vec<&Proc> = procs.iter().filter(|p| is_app(&u.app, p) == Some(true) && p.created.is_none_or(|c| c <= u.since + SLACK)).collect();
            if holders.is_empty() {
                return None;
            }
            let exe = match &u.app {
                App::Exe(path) => Some(path.clone()),
                App::Packaged(_) => holders.iter().find_map(|p| p.path.clone()),
            };
            Some(Live { using: u.clone(), pids: holders.iter().map(|p| p.pid).collect(), exe })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::consent::Kind;
    use super::*;

    const CHROME: &str = r"C:\Program Files\Google\Chrome\Application\chrome.exe";
    const CAMERA: &str = r"C:\Program Files\WindowsApps\Microsoft.WindowsCamera_2025.2510.1.0_x64__8wekyb3d8bbwe\WindowsCamera.exe";

    fn proc(pid: u32, path: &str, created: u64) -> Proc {
        Proc { pid, name: path.rsplit('\\').next().unwrap().to_lowercase(), path: Some(path.into()), created: Some(created) }
    }

    fn using(kind: Kind, app: App, since: u64) -> Use {
        Use { kind, app, since }
    }

    #[test]
    fn privacy_a_use_needs_a_process_that_was_there_when_it_started() {
        let u = using(Kind::Screen, App::Exe(CHROME.into()), 1_000_000_000);
        // Chrome died with its stop unwritten, then was started again.
        assert!(alive(std::slice::from_ref(&u), &[proc(7, CHROME, 2_000_000_000)]).is_empty());
        let live = alive(std::slice::from_ref(&u), &[proc(7, CHROME, 2_000_000_000), proc(3, CHROME, 900_000_000), proc(4, r"C:\other\chrome.exe", 1)]);
        assert_eq!((live[0].pids.clone(), live[0].exe.as_deref()), (vec![3], Some(CHROME)));
        assert!(alive(&[u], &[]).is_empty(), "nothing running: the stop was never written");
    }

    #[test]
    fn privacy_packaged_apps_match_their_package_folder() {
        let u = using(Kind::Camera, App::Packaged("Microsoft.WindowsCamera_8wekyb3d8bbwe".into()), 1_000_000_000);
        let live = alive(std::slice::from_ref(&u), &[proc(9, CAMERA, 10), proc(10, CHROME, 10)]);
        assert_eq!((live[0].pids.clone(), live[0].exe.as_deref()), (vec![9], Some(CAMERA)));
        let other = using(Kind::Camera, App::Packaged("Microsoft.WindowsCameraPro_8wekyb3d8bbwe".into()), 1_000_000_000);
        assert!(alive(&[other], &[proc(9, CAMERA, 10)]).is_empty());
    }

    #[test]
    fn privacy_unreadable_processes_are_trusted_by_name() {
        let u = using(Kind::Microphone, App::Exe(r"C:\Tools\Recorder.exe".into()), 1_000);
        let elevated = Proc { pid: 5, name: "recorder.exe".into(), path: None, created: None };
        assert_eq!(alive(std::slice::from_ref(&u), std::slice::from_ref(&elevated))[0].pids, vec![5]);
        let p = using(Kind::Microphone, App::Packaged("A.B_123".into()), 1_000);
        assert!(alive(&[p], &[elevated]).is_empty(), "a packaged app is known only by its path");
    }
}
