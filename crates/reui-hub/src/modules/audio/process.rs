//! Which process a player is: its AUMID (or a browser's hash) matched against running
//! executables and package families, taking the root of the matching process tree.

use std::collections::{HashMap, HashSet};

use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS};
use windows::Win32::Storage::Packaging::Appx::GetPackageFamilyName;
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

/// A process from the ToolHelp snapshot; `exe` is the lowercase file name without ".exe".
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Proc {
    pid: u32,
    parent: u32,
    exe: String,
}

fn exe_stem(name: &str) -> String {
    let name = name.rsplit(['\\', '/']).next().unwrap_or(name).to_ascii_lowercase();
    name.strip_suffix(".exe").map(str::to_owned).unwrap_or(name)
}

pub(super) fn processes() -> Vec<Proc> {
    let Ok(snap) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else { return Vec::new() };
    let mut out = Vec::new();
    let mut e = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
    let mut ok = unsafe { Process32FirstW(snap, &mut e) }.is_ok();
    while ok {
        let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
        out.push(Proc { pid: e.th32ProcessID, parent: e.th32ParentProcessID, exe: exe_stem(&String::from_utf16_lossy(&e.szExeFile[..len])) });
        ok = unsafe { Process32NextW(snap, &mut e) }.is_ok();
    }
    _ = unsafe { CloseHandle(snap) };
    out
}

/// Gecko browsers register a 16-hex install hash as their AUMID instead of a name.
const GECKO: &[&str] = &["firefox", "librewolf", "waterfox", "zen", "floorp"];

/// The package family of a packaged AUMID ("Family_publisherid!App"), `None` for Win32 ones.
fn package_family(aumid: &str) -> Option<&str> {
    let (family, _) = aumid.split_once('!')?;
    family.contains('_').then_some(family)
}

/// Exe stems a GSMTC AUMID may be running as, most likely first. Win32 AUMIDs are usually
/// the exe ("Spotify.exe", "chrome", "MSEdge"), sometimes a known-folder path
/// ("{GUID}\VideoLAN\VLC\vlc.exe") or a dotted id ("com.squirrel.Discord.Discord",
/// "Chrome._crx_…" for PWAs). For packaged ones the app id after '!' is a last resort.
fn exe_candidates(aumid: &str) -> Vec<String> {
    let aumid = aumid.trim();
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: &str| {
        let s = exe_stem(s);
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
    };
    if aumid.len() == 16 && aumid.bytes().all(|b| b.is_ascii_hexdigit()) {
        GECKO.iter().for_each(|g| push(g));
        return out;
    }
    if let Some((_, app)) = aumid.split_once('!') {
        if !app.eq_ignore_ascii_case("app") {
            push(app);
        }
        return out;
    }
    let stem = exe_stem(aumid);
    push(&stem);
    if let Some((_, last)) = stem.rsplit_once('.') {
        push(last);
    }
    if let Some((first, _)) = stem.split_once('.') {
        push(first);
    }
    out
}

/// The root of the largest tree of matching processes: a match whose parent isn't itself a
/// match (Chrome's browser process, not its renderers). Ties go to the oldest-looking pid.
fn tree_root(procs: &[Proc], is_match: impl Fn(&Proc) -> bool) -> Option<u32> {
    let matching: HashSet<u32> = procs.iter().filter(|p| is_match(p)).map(|p| p.pid).collect();
    matching
        .iter()
        .copied()
        .filter(|&pid| procs.iter().find(|p| p.pid == pid).is_some_and(|p| p.parent == pid || !matching.contains(&p.parent)))
        .map(|root| (descendants(procs, root).iter().filter(|d| matching.contains(d)).count(), root))
        .max_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)))
        .map(|(_, root)| root)
}

/// `root` and every process below it.
pub(super) fn descendants(procs: &[Proc], root: u32) -> HashSet<u32> {
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for p in procs.iter().filter(|p| p.pid != p.parent) {
        children.entry(p.parent).or_default().push(p.pid);
    }
    let mut seen = HashSet::from([root]);
    let mut stack = vec![root];
    while let Some(pid) = stack.pop() {
        for &c in children.get(&pid).into_iter().flatten() {
            if seen.insert(c) {
                stack.push(c);
            }
        }
    }
    seen
}

fn process_family(pid: u32) -> Option<String> {
    let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buf = [0u16; 256];
    let mut len = buf.len() as u32;
    let rc = unsafe { GetPackageFamilyName(h, &mut len, Some(PWSTR(buf.as_mut_ptr()))) };
    _ = unsafe { CloseHandle(h) };
    (rc == ERROR_SUCCESS).then(|| String::from_utf16_lossy(&buf[..(len as usize).saturating_sub(1).min(buf.len())]))
}

/// The root process of a player from its GSMTC `SourceAppUserModelId` ("Spotify.exe",
/// "Chrome", "SpotifyAB.SpotifyMusic_…!Spotify"). For browsers, the root browser process,
/// so its audio child is included in process-tree capture.
pub fn process_for(aumid: &str) -> Option<u32> {
    let procs = processes();
    if let Some(family) = package_family(aumid) {
        // Only packaged processes answer; everything else (and protected processes) fails fast.
        let families: HashMap<u32, String> = procs.iter().filter(|p| p.pid > 4).filter_map(|p| Some((p.pid, process_family(p.pid)?))).collect();
        let hit = tree_root(&procs, |p| families.get(&p.pid).is_some_and(|f| f.eq_ignore_ascii_case(family)));
        if hit.is_some() {
            return hit;
        }
    }
    exe_candidates(aumid).iter().find_map(|exe| tree_root(&procs, |p| &p.exe == exe))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn procs(list: &[(u32, u32, &str)]) -> Vec<Proc> {
        list.iter().map(|&(pid, parent, exe)| Proc { pid, parent, exe: exe_stem(exe) }).collect()
    }

    #[test]
    fn audio_exe_candidates() {
        assert_eq!(exe_candidates("Spotify.exe"), ["spotify"]);
        assert_eq!(exe_candidates("Chrome"), ["chrome"]);
        assert_eq!(exe_candidates("MSEdge"), ["msedge"]);
        assert_eq!(exe_candidates(r"{6D809377-6AF0-444B-8957-A3773F02200E}\VideoLAN\VLC\vlc.exe"), ["vlc"]);
        assert_eq!(exe_candidates("com.squirrel.Discord.Discord"), ["com.squirrel.discord.discord", "discord", "com"]);
        assert_eq!(exe_candidates("Chrome._crx_abcdef"), ["chrome._crx_abcdef", "_crx_abcdef", "chrome"]);
        assert_eq!(exe_candidates("308046B0AF4A39CB")[0], "firefox");
        assert_eq!(exe_candidates("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"), ["spotify"]);
        assert!(exe_candidates("Microsoft.ZuneMusic_8wekyb3d8bbwe!App").is_empty());
        assert!(exe_candidates("").is_empty());
    }

    #[test]
    fn audio_package_family() {
        assert_eq!(package_family("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"), Some("SpotifyAB.SpotifyMusic_zpdnekdrzrea0"));
        assert_eq!(package_family("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"), Some("Microsoft.ZuneMusic_8wekyb3d8bbwe"));
        assert_eq!(package_family("Spotify.exe"), None);
        assert_eq!(package_family("308046B0AF4A39CB"), None);
    }

    #[test]
    fn audio_tree_root() {
        let p = procs(&[
            (4, 0, "System"),
            (100, 4, "explorer.exe"),
            (200, 100, "chrome.exe"), // browser
            (201, 200, "chrome.exe"), // audio service
            (202, 200, "chrome.exe"),
            (300, 100, "chrome.exe"), // a second, smaller instance
            (400, 999, "Spotify.exe"),
            (401, 400, "Spotify.exe"),
        ]);
        assert_eq!(tree_root(&p, |x| x.exe == "chrome"), Some(200));
        assert_eq!(tree_root(&p, |x| x.exe == "spotify"), Some(400));
        assert_eq!(tree_root(&p, |x| x.exe == "firefox"), None);
        // A lone child whose parent exited still counts as a root.
        assert_eq!(tree_root(&procs(&[(7, 3, "vlc.exe")]), |x| x.exe == "vlc"), Some(7));
    }

    #[test]
    fn audio_descendants() {
        let p = procs(&[(1, 0, "a"), (2, 1, "b"), (3, 2, "c"), (4, 9, "d"), (0, 0, "idle")]);
        assert_eq!(descendants(&p, 1), HashSet::from([1, 2, 3]));
        assert_eq!(descendants(&p, 4), HashSet::from([4]));
    }

    #[test]
    fn audio_process_for_doesnt_panic() {
        // Read-only: a process snapshot, no devices.
        _ = process_for("explorer.exe");
        _ = process_for("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic");
        _ = process_for("308046B0AF4A39CB");
        assert_eq!(process_for("reui-no-such-player-xyz.exe"), None);
    }
}
