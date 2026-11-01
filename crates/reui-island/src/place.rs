//! Where the island sits: which display (the `monitor` setting) and the window rectangle
//! at that display's top centre, at its DPI. Re-run after resume, display and DPI changes,
//! Explorer restarts and work-area changes (see `main.rs`), and, in "cursor" mode, whenever
//! the pointer settles on another display ([`should_follow`]), whatever the island is
//! showing: an open card goes along.

use windows::core::BOOL;
use windows::Win32::Foundation::{LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, MONITORINFOF_PRIMARY};

use reui_design::look::Look;
use reui_design::tokens::island;

use reui_design::layout::switcher;

use crate::render::{expanded_surface, COMPACT_MAX_WIDTH};

/// The window timer for following the pointer to another display ("cursor" mode), and
/// how long the pointer stays there first. Outside the pill's own timers (1–30).
pub const TIMER_FOLLOW: usize = 104;
pub const FOLLOW_DWELL_MS: u32 = 150;

/// Posted to the island window to re-place it now (the `monitor` setting changed).
pub const WM_PLACE: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 5;

/// Slack around the island inside the (click-through) window.
const MARGIN: f32 = 24.0;

/// The `monitor` setting.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum MonitorChoice {
    /// The primary display (default).
    #[default]
    Primary,
    /// The display the pointer is on when an activity appears.
    Cursor,
    /// A display by device name (`\\.\DISPLAY2`); the primary one while it's not connected.
    Named(String),
    /// Every display, an island on each (docs/PRD-all-displays.md).
    All,
}

impl MonitorChoice {
    pub fn parse(text: &str) -> Self {
        match text.trim() {
            "" | "primary" => MonitorChoice::Primary,
            "cursor" => MonitorChoice::Cursor,
            "all" => MonitorChoice::All,
            name => MonitorChoice::Named(name.to_string()),
        }
    }
}

/// A display as the choice sees it.
#[derive(Clone, Debug)]
pub struct Screen {
    pub handle: HMONITOR,
    pub name: String,
    pub rect: RECT,
    pub work: RECT,
    pub primary: bool,
    pub dpi: u32,
}

/// Where an island goes: the display, its scale, and the window rectangle.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    pub monitor: HMONITOR,
    /// The display's device name (`\\.\DISPLAY1`): how an island is matched to its display
    /// when displays change (handles don't survive that).
    pub name: String,
    /// The display's scale (its DPI / 96), before the look's size.
    pub scale: f32,
    pub window: RECT,
}

fn contains(r: &RECT, p: POINT) -> bool {
    p.x >= r.left && p.x < r.right && p.y >= r.top && p.y < r.bottom
}

/// Which of `screens` the choice means: the named or pointer's display when there is one,
/// otherwise the primary (or the first, if Windows reports none as primary).
pub fn choose(choice: &MonitorChoice, screens: &[Screen], cursor: POINT) -> Option<usize> {
    let primary = || screens.iter().position(|s| s.primary).or((!screens.is_empty()).then_some(0));
    let picked = match choice {
        MonitorChoice::Primary => None,
        MonitorChoice::Cursor => screens.iter().position(|s| contains(&s.rect, cursor)),
        MonitorChoice::Named(name) => screens.iter().position(|s| s.name.eq_ignore_ascii_case(name)),
        // One island per display is `wanted`'s; asked for one, it's the primary's.
        MonitorChoice::All => None,
    };
    picked.or_else(primary)
}

/// Whether an island on the display `screen` should move to the pointer's: only in
/// "cursor" mode, only when it can move (`Pill::can_move`: not mid-drag, no menu up), and
/// only once the pointer is off that display. Pure.
pub fn should_follow(choice: &MonitorChoice, can_move: bool, screen: &RECT, pointer: POINT) -> bool {
    *choice == MonitorChoice::Cursor && can_move && !contains(screen, pointer)
}

/// A display's whole area.
pub fn monitor_rect(monitor: HMONITOR) -> Option<RECT> {
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool().then_some(info.rcMonitor)
}

/// The island's window on a display: wide enough for the expanded pill, or the widest
/// compact pill plus its minimal bubble, whichever is wider; centred on the display, with
/// its top at the work area's top (below a taskbar docked at the top), and tall enough for
/// the tallest card with the view switcher under it. Sized for the look: its size, padding
/// and gap from the top.
pub fn window_rect(screen: &RECT, work: &RECT, dpi: f32, look: &Look) -> RECT {
    let scale = dpi * look.size;
    let surface = expanded_surface(look);
    let half = (surface.0 / 2.0).max(COMPACT_MAX_WIDTH / 2.0 + island::minimal::GAP + island::minimal::SIZE);
    let width = ((half + MARGIN) * 2.0 * scale).ceil() as i32;
    let height = ((look.top() + surface.1 + switcher::ROOM + MARGIN) * scale).ceil() as i32;
    let left = screen.left + (screen.right - screen.left - width) / 2;
    let top = work.top.max(screen.top);
    RECT { left, top, right: left + width, bottom: top + height }
}

/// Every display, with its name, area and DPI.
pub fn screens() -> Vec<Screen> {
    unsafe extern "system" fn each(monitor: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        let list = &mut *(data.0 as *mut Vec<HMONITOR>);
        list.push(monitor);
        BOOL(1)
    }
    let mut handles: Vec<HMONITOR> = Vec::new();
    unsafe { _ = EnumDisplayMonitors(None, None, Some(each), LPARAM(&mut handles as *mut _ as isize)) };
    handles
        .into_iter()
        .filter_map(|handle| unsafe {
            let mut info = MONITORINFOEXW::default();
            info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
            if !GetMonitorInfoW(handle, &mut info as *mut MONITORINFOEXW as *mut MONITORINFO).as_bool() {
                return None;
            }
            let (mut dpi, mut _dpi_y) = (96u32, 96u32);
            if GetDpiForMonitor(handle, MDT_EFFECTIVE_DPI, &mut dpi, &mut _dpi_y).is_err() {
                dpi = 96;
            }
            let len = info.szDevice.iter().position(|&c| c == 0).unwrap_or(info.szDevice.len());
            Some(Screen {
                handle,
                name: String::from_utf16_lossy(&info.szDevice[..len]),
                rect: info.monitorInfo.rcMonitor,
                work: info.monitorInfo.rcWork,
                primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
                dpi,
            })
        })
        .collect()
}

fn placement(screen: &Screen, look: &Look) -> Placement {
    let scale = screen.dpi.max(1) as f32 / 96.0;
    Placement { monitor: screen.handle, name: screen.name.clone(), scale, window: window_rect(&screen.rect, &screen.work, scale, look) }
}

/// Which of `screens` get an island: every one for All displays, else the one the choice
/// means. Pure.
pub fn wanted(choice: &MonitorChoice, screens: &[Screen], cursor: POINT) -> Vec<usize> {
    match choice {
        MonitorChoice::All => (0..screens.len()).collect(),
        _ => choose(choice, screens, cursor).into_iter().collect(),
    }
}

/// Where each island goes now, primary display first (it holds the hub link's first
/// window); empty only if Windows reports no display.
pub fn placements(choice: &MonitorChoice, look: &Look) -> Vec<Placement> {
    let screens = screens();
    let mut cursor = POINT::default();
    unsafe { _ = GetCursorPos(&mut cursor) };
    let mut picked = wanted(choice, &screens, cursor);
    picked.sort_by_key(|&i| !screens[i].primary);
    picked.into_iter().map(|i| placement(&screens[i], look)).collect()
}

/// Which of `screens` (by area) holds `point`; the island there is the lead (§5.4). Pure.
pub fn holding(rects: &[RECT], point: POINT) -> Option<usize> {
    rects.iter().position(|r| contains(r, point))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(name: &str, left: i32, primary: bool) -> Screen {
        let rect = RECT { left, top: 0, right: left + 1920, bottom: 1080 };
        Screen { handle: HMONITOR(left as isize as *mut _), name: name.into(), rect, work: rect, primary, dpi: 96 }
    }

    #[test]
    fn the_island_follows_the_pointer_off_its_display_unless_held() {
        let left = RECT { left: -1920, top: 0, right: 0, bottom: 1080 };
        let on_right = POINT { x: 500, y: 300 };
        let on_left = POINT { x: -500, y: 300 };
        assert!(should_follow(&MonitorChoice::Cursor, true, &left, on_right));
        assert!(!should_follow(&MonitorChoice::Cursor, true, &left, on_left), "already there");
        assert!(!should_follow(&MonitorChoice::Cursor, false, &left, on_right), "a bar being dragged, a menu: it stays");
        assert!(!should_follow(&MonitorChoice::Primary, true, &left, on_right));
        assert!(!should_follow(&MonitorChoice::Named(r"\\.\DISPLAY2".into()), true, &left, on_right));
    }

    #[test]
    fn all_displays_wants_an_island_on_each() {
        let screens = [screen(r"\\.\DISPLAY2", -1920, false), screen(r"\\.\DISPLAY1", 0, true)];
        let pointer = POINT { x: -100, y: 10 };
        assert_eq!(MonitorChoice::parse("all"), MonitorChoice::All);
        assert_eq!(wanted(&MonitorChoice::All, &screens, pointer), [0, 1]);
        assert_eq!(wanted(&MonitorChoice::Cursor, &screens, pointer), [0]);
        assert_eq!(wanted(&MonitorChoice::Primary, &screens, pointer), [1]);
        assert_eq!(wanted(&MonitorChoice::All, &[], pointer), Vec::<usize>::new());
        let rects: Vec<RECT> = screens.iter().map(|s| s.rect).collect();
        assert_eq!(holding(&rects, pointer), Some(0));
        assert_eq!(holding(&rects, POINT { x: 500, y: 10 }), Some(1));
        assert_eq!(holding(&rects, POINT { x: 5000, y: 10 }), None);
    }

    #[test]
    fn choices_parse_and_fall_back_to_the_primary_display() {
        assert_eq!(MonitorChoice::parse("primary"), MonitorChoice::Primary);
        assert_eq!(MonitorChoice::parse(""), MonitorChoice::Primary);
        assert_eq!(MonitorChoice::parse("cursor"), MonitorChoice::Cursor);
        assert_eq!(MonitorChoice::parse(r"\\.\DISPLAY2"), MonitorChoice::Named(r"\\.\DISPLAY2".into()));

        let screens = [screen(r"\\.\DISPLAY2", -1920, false), screen(r"\\.\DISPLAY1", 0, true)];
        let on_left = POINT { x: -100, y: 500 };
        assert_eq!(choose(&MonitorChoice::Primary, &screens, on_left), Some(1));
        assert_eq!(choose(&MonitorChoice::Cursor, &screens, on_left), Some(0));
        assert_eq!(choose(&MonitorChoice::Cursor, &screens, POINT { x: 99_999, y: 0 }), Some(1));
        assert_eq!(choose(&MonitorChoice::Named(r"\\.\display2".into()), &screens, POINT::default()), Some(0));
        assert_eq!(choose(&MonitorChoice::Named(r"\\.\DISPLAY9".into()), &screens, POINT::default()), Some(1), "unplugged: primary");
        assert_eq!(choose(&MonitorChoice::Primary, &[], POINT::default()), None);
        assert_eq!(choose(&MonitorChoice::Primary, &[screen("a", 0, false)], POINT::default()), Some(0));
    }

    #[test]
    fn the_window_is_centred_at_the_top_and_scales_with_dpi() {
        let monitor = RECT { left: -1920, top: 0, right: 0, bottom: 1080 };
        let look = Look::default();
        let one = window_rect(&monitor, &monitor, 1.0, &look);
        let two = window_rect(&monitor, &monitor, 2.0, &look);
        assert_eq!(one.left - monitor.left, monitor.right - one.right, "centred");
        assert!((two.right - two.left) >= 2 * (one.right - one.left) - 1);
        assert_eq!(one.top, 0);
        let room = (look.top() + expanded_surface(&look).1 + switcher::ROOM) as i32;
        assert!(one.bottom - one.top >= room, "the switcher fits under the tallest card");
        // A taskbar docked at the top: the island sits below it.
        let work = RECT { top: 48, ..monitor };
        assert_eq!(window_rect(&monitor, &work, 1.0, &look).top, 48);
        // A bigger look is a bigger window.
        let big = Look { size: 1.25, ..look };
        assert!(window_rect(&monitor, &monitor, 1.0, &big).right - window_rect(&monitor, &monitor, 1.0, &big).left > one.right - one.left);
    }
}
