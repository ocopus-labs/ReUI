//! Where a paste goes back to: the foreground window, its focused control and the text
//! caret (UI Automation, then MSAA, then the system caret), plus the owning app's name.

use std::ffi::c_void;
use std::sync::mpsc as std_mpsc;
use std::time::Duration;

use windows::core::{Interface, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::Ole::{SafeArrayAccessData, SafeArrayDestroy, SafeArrayGetLBound, SafeArrayGetUBound, SafeArrayUnaccessData};
use windows::Win32::System::Threading::{GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::System::Variant::{VARIANT, VT_I4};
use windows::Win32::UI::Accessibility::{
    AccessibleObjectFromWindow, CUIAutomation8, IAccessible, IUIAutomation, IUIAutomation2, IUIAutomationTextPattern2, IUIAutomationTextRange, TextUnit_Character, UIA_TextPattern2Id,
};
use windows::Win32::UI::HiDpi::{SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::WindowsAndMessaging::*;

use super::Target;

/// Whole budget for UI Automation and MSAA to find the caret; then the system caret is used.
const CARET_WAIT: Duration = Duration::from_millis(200);
/// UI Automation's own connection and transaction timeouts, so a hung app can't hold us.
const UIA_TIMEOUT_MS: u32 = 150;

/// One of ours: this process or another `reui-*` executable (the island and its window).
pub(super) unsafe fn is_reui(window: HWND) -> bool {
    let mut pid = 0u32;
    GetWindowThreadProcessId(window, Some(&mut pid));
    pid == GetCurrentProcessId() || exe_path(window).is_some_and(|p| file_name_lower(&p).starts_with("reui-"))
}

/// The foreground window, its focused control, and (with a finder) the caret: UI
/// Automation or MSAA within [`CARET_WAIT`], else the classic system caret.
pub(super) unsafe fn capture_target(finder: Option<&CaretFinder>) -> Target {
    let foreground = GetForegroundWindow();
    let thread = GetWindowThreadProcessId(foreground, None);
    let mut info = GUITHREADINFO { cbSize: size_of::<GUITHREADINFO>() as u32, ..Default::default() };
    let gui = (thread != 0 && GetGUIThreadInfo(thread, &mut info).is_ok()).then_some(info);
    let focus = gui.map(|g| g.hwndFocus).filter(|h| !h.is_invalid()).unwrap_or(foreground);
    // Some providers report an empty rectangle at the origin for "no caret".
    let caret = finder.and_then(|finder| finder.find(focus)).filter(|c| *c != [0, 0, 0, 0]);
    let caret = caret.or_else(|| {
        let g = gui.filter(|g| finder.is_some() && !g.hwndCaret.is_invalid() && g.rcCaret.bottom > g.rcCaret.top)?;
        let mut corners = [POINT { x: g.rcCaret.left, y: g.rcCaret.top }, POINT { x: g.rcCaret.right, y: g.rcCaret.bottom }];
        for corner in &mut corners {
            if !ClientToScreen(g.hwndCaret, corner).as_bool() {
                return None;
            }
        }
        Some([corners[0].x, corners[0].y, corners[1].x.max(corners[0].x + 1), corners[1].y])
    });
    Target { hwnd: foreground.0 as isize, focus: focus.0 as isize, caret }
}

type CaretRequest = (isize, std_mpsc::Sender<Option<[i32; 4]>>);

/// Asks UI Automation, then MSAA, where the text caret is, on its own thread: both talk to
/// the target app, and a hung app must cost the hotkey at most [`CARET_WAIT`]. A request
/// still stuck when the next one comes simply makes that one time out too.
pub(super) struct CaretFinder {
    requests: std_mpsc::Sender<CaretRequest>,
}

impl CaretFinder {
    pub(super) fn start() -> Option<CaretFinder> {
        let (requests, queue) = std_mpsc::channel::<CaretRequest>();
        std::thread::Builder::new()
            .name("clipboard-caret".into())
            .spawn(move || unsafe {
                SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
                _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                let uia: Option<IUIAutomation> = CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER).ok();
                if let Some(uia) = uia.as_ref().and_then(|u| u.cast::<IUIAutomation2>().ok()) {
                    _ = uia.SetConnectionTimeout(UIA_TIMEOUT_MS);
                    _ = uia.SetTransactionTimeout(UIA_TIMEOUT_MS);
                }
                for (focus, reply) in queue {
                    let caret = uia.as_ref().and_then(|uia| uia_caret(uia)).or_else(|| msaa_caret(HWND(focus as *mut c_void)));
                    _ = reply.send(caret);
                }
            })
            .ok()?;
        Some(CaretFinder { requests })
    }

    fn find(&self, focus: HWND) -> Option<[i32; 4]> {
        let (reply, answer) = std_mpsc::channel();
        self.requests.send((focus.0 as isize, reply)).ok()?;
        answer.recv_timeout(CARET_WAIT).ok().flatten()
    }
}

/// The focused element's caret range (TextPattern2). An empty range often has no
/// rectangle, so then the character after it is measured and its left edge used.
unsafe fn uia_caret(uia: &IUIAutomation) -> Option<[i32; 4]> {
    let element = uia.GetFocusedElement().ok()?;
    let pattern: IUIAutomationTextPattern2 = element.GetCurrentPatternAs(UIA_TextPattern2Id).ok()?;
    let mut active = windows::core::BOOL::default();
    let range = pattern.GetCaretRange(&mut active).ok()?;
    let rect = match range_rect(&range) {
        Some(rect) => rect,
        None => {
            let around = range.Clone().ok()?;
            around.ExpandToEnclosingUnit(TextUnit_Character).ok()?;
            range_rect(&around)?
        }
    };
    Some([rect[0], rect[1], rect[0] + 1, rect[3]])
}

/// The first bounding rectangle of a text range, as left, top, right, bottom.
unsafe fn range_rect(range: &IUIAutomationTextRange) -> Option<[i32; 4]> {
    let array = range.GetBoundingRectangles().ok()?;
    if array.is_null() {
        return None;
    }
    let (low, high) = (SafeArrayGetLBound(array, 1).unwrap_or(0), SafeArrayGetUBound(array, 1).unwrap_or(-1));
    let mut data: *mut c_void = std::ptr::null_mut();
    let mut rect = None;
    if high - low + 1 >= 4 && SafeArrayAccessData(array, &mut data).is_ok() {
        // Doubles: left, top, width, height, then the next rectangle.
        let v = std::slice::from_raw_parts(data as *const f64, 4);
        if v[3] > 0.0 {
            rect = Some([v[0].round() as i32, v[1].round() as i32, (v[0] + v[2]).round() as i32, (v[1] + v[3]).round() as i32]);
        }
        _ = SafeArrayUnaccessData(array);
    }
    _ = SafeArrayDestroy(array);
    rect
}

/// The MSAA caret object of the focused window: apps that draw their own caret but report
/// it to accessibility tools, and the classic caret's proxy.
unsafe fn msaa_caret(focus: HWND) -> Option<[i32; 4]> {
    if focus.is_invalid() {
        return None;
    }
    let mut raw: *mut c_void = std::ptr::null_mut();
    AccessibleObjectFromWindow(focus, OBJID_CARET.0 as u32, &IAccessible::IID, &mut raw).ok()?;
    if raw.is_null() {
        return None;
    }
    let accessible = IAccessible::from_raw(raw);
    let mut child = VARIANT::default();
    (*child.Anonymous.Anonymous).vt = VT_I4; // CHILDID_SELF (0)
    let (mut l, mut t, mut w, mut h) = (0, 0, 0, 0);
    accessible.accLocation(&mut l, &mut t, &mut w, &mut h, &child).ok()?;
    (h > 0 && (l, t) != (0, 0)).then_some([l, t, l + w.max(1), t + h])
}

/// Path of the process that owns `window`.
pub(super) unsafe fn exe_path(window: HWND) -> Option<String> {
    if window.is_invalid() {
        return None;
    }
    let mut pid = 0u32;
    GetWindowThreadProcessId(window, Some(&mut pid));
    if pid == 0 {
        return None;
    }
    let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
    let mut buffer = [0u16; 512];
    let mut len = buffer.len() as u32;
    let named = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &mut len);
    _ = CloseHandle(process);
    named.ok()?;
    Some(String::from_utf16_lossy(&buffer[..len as usize]))
}

pub(super) fn file_name_lower(path: &str) -> String {
    path.rsplit(['\\', '/']).next().unwrap_or(path).to_lowercase()
}

/// `C:\...\chrome.exe` → "Chrome".
pub(crate) fn app_name(path: &str) -> String {
    let file = path.rsplit(['\\', '/']).next().unwrap_or(path);
    let stem = file.strip_suffix(".exe").or_else(|| file.strip_suffix(".EXE")).unwrap_or(file);
    let known = [
        ("chrome", "Chrome"),
        ("msedge", "Edge"),
        ("firefox", "Firefox"),
        ("code", "VS Code"),
        ("explorer", "File Explorer"),
        ("windowsterminal", "Terminal"),
        ("snippingtool", "Snipping Tool"),
        ("screenclippinghost", "Snipping Tool"),
        ("winword", "Word"),
        ("excel", "Excel"),
        ("powerpnt", "PowerPoint"),
        ("notepad", "Notepad"),
        ("figma", "Figma"),
        ("slack", "Slack"),
        ("discord", "Discord"),
        ("zoom", "Zoom"),
        ("ms-teams", "Teams"),
        ("obs64", "OBS Studio"),
    ];
    let lower = stem.to_ascii_lowercase();
    if let Some((_, name)) = known.iter().find(|(k, _)| *k == lower) {
        return name.to_string();
    }
    let mut chars = stem.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    /// Reads (never changes) the foreground window and its caret; prints what it found.
    #[test]
    #[ignore]
    fn targets_are_captured_quickly() {
        let finder = CaretFinder::start().unwrap();
        for _ in 0..2 {
            let start = Instant::now();
            let target = unsafe { capture_target(Some(&finder)) };
            let took = start.elapsed();
            println!("{target:?} in {took:?}");
            assert!(target.hwnd != 0 && took < CARET_WAIT + Duration::from_millis(100));
        }
        assert_eq!(Target::now().caret, None);
    }

    #[test]
    fn source_apps_read_naturally() {
        assert_eq!(app_name(r"C:\Program Files\Google\Chrome\Application\chrome.exe"), "Chrome");
        assert_eq!(app_name(r"C:\Users\me\AppData\Local\Programs\Microsoft VS Code\Code.exe"), "VS Code");
        assert_eq!(app_name(r"C:\Tools\obsidian.exe"), "Obsidian");
        assert_eq!(file_name_lower(r"C:\Tools\KeePass.EXE"), "keepass.exe");
    }
}
