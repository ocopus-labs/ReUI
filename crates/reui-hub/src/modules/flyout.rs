//! Hiding Windows 11's own volume / brightness flyout while ReUI shows those changes
//! (opt-in, `replace_windows_flyout`). FluentFlyout's method: find the
//! `XamlExplorerHostIslandWindow` whose `DesktopWindowXamlSource` content bridge has an input
//! site, make it layered and click-through, move it far off-screen and minimise it without
//! activating. It's all or nothing: the same host shows the media and airplane-mode flyouts.
//!
//! Always restorable: the original extended style is kept in memory and in
//! `%LOCALAPPDATA%\ReUI\flyout-hidden.json` while hidden, so it comes back when the setting is
//! turned off, the module ends, the process panics (release builds abort, so a panic hook
//! does it), or, after a hard crash, at the next start ([`recover`]). Every call here may
//! talk to Explorer's windows, so they all run off the hub thread.

use std::path::PathBuf;
use std::sync::{Mutex, Once};

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ};
use windows::Win32::UI::Shell::{SHQueryUserNotificationState, QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, GetClassNameW, GetWindowLongPtrW, GetWindowRect, InternalGetWindowText, IsWindow, SetWindowLongPtrW, SetWindowPos, ShowWindowAsync, GWL_EXSTYLE, SWP_ASYNCWINDOWPOS,
    SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SW_SHOWMINNOACTIVE, SW_SHOWNOACTIVATE, WS_EX_LAYERED, WS_EX_TRANSPARENT,
};

pub const HOST_CLASS: &str = "XamlExplorerHostIslandWindow";
pub const BRIDGE_CLASS: &str = "Windows.UI.Composition.DesktopWindowContentBridge";
pub const BRIDGE_TITLE: &str = "DesktopWindowXamlSource";
pub const INPUT_SITE_CLASS: &str = "Windows.UI.Input.InputSite.WindowClass";
/// Other Explorer surfaces built the same way that must never be touched.
const NOT_THE_FLYOUT: &[&str] = &["Task Switching", "Task View"];
const OFFSCREEN: i32 = -99_999;

/// A window, for matching: what Explorer's window tree looks like to us.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Node {
    pub class: String,
    pub title: String,
    pub children: Vec<Node>,
}

/// The flyout host: a host island window holding (at any depth) the XAML content bridge,
/// which has an input site child.
pub fn is_flyout_host(top: &Node) -> bool {
    fn has_bridge(node: &Node) -> bool {
        node.children.iter().any(|c| (c.class == BRIDGE_CLASS && c.title == BRIDGE_TITLE && c.children.iter().any(|i| i.class == INPUT_SITE_CLASS)) || has_bridge(c))
    }
    top.class == HOST_CLASS && !NOT_THE_FLYOUT.contains(&top.title.as_str()) && has_bridge(top)
}

/// The first top-level window that is the flyout host.
pub fn find_host(tops: &[Node]) -> Option<usize> {
    tops.iter().position(is_flyout_host)
}

// ---- the real window tree ----------------------------------------------------------------

fn class_of(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) }.max(0) as usize;
    String::from_utf16_lossy(&buf[..n])
}

/// The title without sending the window a message (Explorer may be busy).
fn title_of(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { InternalGetWindowText(hwnd, &mut buf) }.max(0) as usize;
    String::from_utf16_lossy(&buf[..n])
}

fn children(parent: Option<HWND>, class: Option<&str>) -> Vec<HWND> {
    let class = class.map(HSTRING::from);
    let class = class.as_ref().map_or(PCWSTR::null(), |c| PCWSTR(c.as_ptr()));
    let mut out = Vec::new();
    let mut after = None;
    while let Ok(next) = unsafe { FindWindowExW(parent, after, class, PCWSTR::null()) } {
        out.push(next);
        after = Some(next);
        if out.len() >= 256 {
            break;
        }
    }
    out
}

fn tree(hwnd: HWND, depth: u32) -> Node {
    let kids = if depth == 0 { Vec::new() } else { children(Some(hwnd), None).into_iter().map(|c| tree(c, depth - 1)).collect() };
    Node { class: class_of(hwnd), title: title_of(hwnd), children: kids }
}

/// Finds Windows' flyout host now.
fn locate() -> Option<HWND> {
    let hosts = children(None, Some(HOST_CLASS));
    let trees: Vec<Node> = hosts.iter().map(|&h| tree(h, 3)).collect();
    find_host(&trees).map(|i| hosts[i])
}

// ---- hide / restore ----------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct Hidden {
    hwnd: isize,
    exstyle: isize,
}

static HIDDEN: Mutex<Option<Hidden>> = Mutex::new(None);

fn marker() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("ReUI")).unwrap_or_else(|| PathBuf::from(".")).join("flyout-hidden.json")
}

fn alive(hwnd: isize) -> bool {
    let hwnd = HWND(hwnd as *mut _);
    unsafe { IsWindow(Some(hwnd)) }.as_bool() && class_of(hwnd) == HOST_CLASS
}

fn offscreen(hwnd: isize) -> bool {
    let mut r = RECT::default();
    unsafe { GetWindowRect(HWND(hwnd as *mut _), &mut r) }.is_ok() && r.left <= OFFSCREEN / 2
}

fn apply_hide(hwnd: isize, exstyle: isize) {
    let h = HWND(hwnd as *mut _);
    unsafe {
        SetWindowLongPtrW(h, GWL_EXSTYLE, exstyle | (WS_EX_LAYERED.0 | WS_EX_TRANSPARENT.0) as isize);
        _ = SetWindowPos(h, None, OFFSCREEN, OFFSCREEN, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS);
        _ = ShowWindowAsync(h, SW_SHOWMINNOACTIVE);
    }
}

fn apply_restore(hwnd: isize, exstyle: isize) {
    let h = HWND(hwnd as *mut _);
    unsafe {
        SetWindowLongPtrW(h, GWL_EXSTYLE, exstyle);
        _ = SetWindowPos(h, None, 0, 0, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS);
        _ = ShowWindowAsync(h, SW_SHOWNOACTIVATE);
    }
}

/// How hiding went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hide {
    Hidden,
    /// No host (a new Windows build, or Explorer hasn't created it yet): both flyouts show.
    NotFound,
}

/// Hides Windows' flyout, or checks it's still hidden (Explorer may have recreated it).
pub fn hide() -> Hide {
    let mut hidden = HIDDEN.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(h) = *hidden {
        if alive(h.hwnd) {
            if !offscreen(h.hwnd) {
                apply_hide(h.hwnd, h.exstyle);
            }
            return Hide::Hidden;
        }
        *hidden = None; // Explorer restarted: find the new one
    }
    let Some(hwnd) = locate() else { return Hide::NotFound };
    let raw = hwnd.0 as isize;
    let exstyle = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
    // Written first, so a crash in between still leaves a way back.
    let path = marker();
    if let Some(dir) = path.parent() {
        _ = std::fs::create_dir_all(dir);
    }
    _ = std::fs::write(&path, serde_json::json!({ "exstyle": exstyle }).to_string());
    install_panic_restore();
    apply_hide(raw, exstyle);
    *hidden = Some(Hidden { hwnd: raw, exstyle });
    Hide::Hidden
}

/// Puts Windows' flyout back if we hid it.
pub fn restore() {
    let mut hidden = HIDDEN.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(h) = hidden.take() {
        if alive(h.hwnd) {
            apply_restore(h.hwnd, h.exstyle);
        }
        _ = std::fs::remove_file(marker());
    }
}

/// At start: a previous run that died with the flyout hidden left a marker; put it back.
pub fn recover() {
    let path = marker();
    let Ok(text) = std::fs::read_to_string(&path) else { return };
    let exstyle = serde_json::from_str::<serde_json::Value>(&text).ok().and_then(|v| v["exstyle"].as_i64());
    if let (Some(exstyle), Some(hwnd)) = (exstyle, locate()) {
        if offscreen(hwnd.0 as isize) {
            apply_restore(hwnd.0 as isize, exstyle as isize);
            eprintln!("osd: restored Windows' flyout hidden by an earlier run");
        }
    }
    _ = std::fs::remove_file(path);
}

/// Release builds abort on panic, skipping every `Drop`: restore first.
fn install_panic_restore() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if let Ok(mut hidden) = HIDDEN.try_lock() {
                if let Some(h) = hidden.take() {
                    apply_restore(h.hwnd, h.exstyle);
                    _ = std::fs::remove_file(marker());
                }
            }
            previous(info);
        }));
    });
}

/// A fullscreen app, game or presentation is in front (Windows' own flyout should show
/// there, ReUI's doesn't).
pub fn fullscreen() -> bool {
    matches!(unsafe { SHQueryUserNotificationState() }, Ok(QUNS_BUSY | QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE))
}

/// "26200.7019", for the not-found log line.
pub fn windows_build() -> String {
    let key = w!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion");
    let mut buf = [0u16; 32];
    let mut len = (buf.len() * 2) as u32;
    let build = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, key, w!("CurrentBuild"), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut len)) }.is_ok().then(|| {
        let n = (len as usize / 2).saturating_sub(1).min(buf.len());
        String::from_utf16_lossy(&buf[..n])
    });
    let mut ubr = 0u32;
    let mut size = 4u32;
    let ubr = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, key, w!("UBR"), RRF_RT_REG_DWORD, None, Some((&mut ubr as *mut u32).cast()), Some(&mut size)) }.is_ok().then_some(ubr);
    match (build, ubr) {
        (Some(b), Some(u)) => format!("{b}.{u}"),
        (Some(b), None) => b,
        _ => "unknown".into(),
    }
}

/// Keeps Windows' flyout hidden while alive and wanted; restores it on drop.
#[derive(Default)]
pub struct Guard;

impl Drop for Guard {
    fn drop(&mut self) {
        restore();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(class: &str, title: &str, children: Vec<Node>) -> Node {
        Node { class: class.into(), title: title.into(), children }
    }

    fn osd_host(title: &str) -> Node {
        node(HOST_CLASS, title, vec![node(BRIDGE_CLASS, BRIDGE_TITLE, vec![node(INPUT_SITE_CLASS, "", vec![])])])
    }

    #[test]
    fn flyout_host_matching() {
        assert!(is_flyout_host(&osd_host("")));
        // The bridge may sit a level deeper.
        let deeper = node(HOST_CLASS, "", vec![node("Intermediate", "", vec![osd_host("").children[0].clone()])]);
        assert!(is_flyout_host(&deeper));
        // No input site, wrong bridge title, wrong class, or Alt+Tab's host: not it.
        assert!(!is_flyout_host(&node(HOST_CLASS, "", vec![node(BRIDGE_CLASS, BRIDGE_TITLE, vec![])])));
        assert!(!is_flyout_host(&node(HOST_CLASS, "", vec![node(BRIDGE_CLASS, "Other", vec![node(INPUT_SITE_CLASS, "", vec![])])])));
        assert!(!is_flyout_host(&node("Shell_TrayWnd", "", osd_host("").children)));
        assert!(!is_flyout_host(&osd_host("Task Switching")));
    }

    #[test]
    fn flyout_find_host_picks_the_first_match() {
        let tops = vec![node(HOST_CLASS, "", vec![]), osd_host("Task Switching"), osd_host(""), osd_host("")];
        assert_eq!(find_host(&tops), Some(2));
        assert_eq!(find_host(&tops[..2]), None);
        assert_eq!(find_host(&[]), None);
    }
}
