//! The islands: one per window, and a window per display in "All displays"
//! (docs/PRD-all-displays.md); one in every other mode. What they share lives here: the hub
//! link (drained once, applied to each), which island is the lead (the one on the pointer's
//! display), and keeping the windows in step with the displays and the `monitor` setting.
//!
//! The lead alone opens cards by itself, takes hotkeys and the open shortcut, and animates
//! bars. When the pointer moves to another display, its island becomes the lead and an open
//! card goes along (`Pill::open_card` / `take_over`).

use std::cell::{Cell, RefCell};
use std::sync::mpsc::Receiver;

use reui_design::look::Look;
use reui_protocol::ClientMsg;
use tokio::sync::mpsc::UnboundedSender;
use windows::core::{w, Result};
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::ipc::Event;
use crate::pill::{self, Pill};
use crate::place::{self, MonitorChoice, Placement};
use crate::settings::Settings;
use crate::{dragdrop, theme};

/// The pill's own timer ids, killed before it's rebuilt.
const PILL_TIMERS: std::ops::RangeInclusive<usize> = 1..=30;

pub const CLASS: windows::core::PCWSTR = w!("ReUI.Island");

thread_local! {
    static PILLS: RefCell<Vec<Pill>> = const { RefCell::new(Vec::new()) };
    /// The hub link: the sender each island gets a copy of, and what arrives.
    static TO_HUB: RefCell<Option<UnboundedSender<ClientMsg>>> = const { RefCell::new(None) };
    static EVENTS: RefCell<Option<Receiver<Event>>> = const { RefCell::new(None) };
    /// The lead island's window.
    static LEAD: Cell<isize> = const { Cell::new(0) };
}

pub fn set_link(to_hub: UnboundedSender<ClientMsg>, events: Receiver<Event>) {
    TO_HUB.with_borrow_mut(|t| *t = Some(to_hub));
    EVENTS.with_borrow_mut(|e| *e = Some(events));
}

/// Run `f` on the island of window `hwnd` unless the islands are already borrowed
/// (re-entrant window messages).
pub fn with_pill(hwnd: HWND, f: impl FnOnce(&mut Pill)) {
    PILLS.with(|cell| {
        if let Ok(mut pills) = cell.try_borrow_mut() {
            if let Some(pill) = pills.iter_mut().find(|p| p.hwnd() == hwnd) {
                f(pill);
            }
        }
    });
}

/// Run `f` on every island (the mouse hook, the foreground app, settings): each decides
/// for itself what it means on its display.
pub fn each_pill(mut f: impl FnMut(&mut Pill)) {
    PILLS.with(|cell| {
        if let Ok(mut pills) = cell.try_borrow_mut() {
            pills.iter_mut().for_each(&mut f);
        }
    });
}

/// The lead island's window (the tray's menu opens from it).
pub fn lead() -> Option<HWND> {
    let hwnd = HWND(LEAD.get() as *mut _);
    (!hwnd.is_invalid()).then_some(hwnd)
}

/// Applies what the hub sent to every island; the lead alone opens what was asked for.
pub fn drain_hub() {
    let Some(batch) = EVENTS.with_borrow(|e| e.as_ref().map(pill::drain)) else { return };
    let leader = LEAD.get();
    each_pill(|p| p.apply_batch(&batch, p.hwnd().0 as isize == leader));
    if batch.toggle_hidden {
        // One setting for all: the lead saves it, the others read it back.
        if let Some(hwnd) = lead() {
            with_pill(hwnd, |p| p.toggle_hidden());
        }
        sync_settings();
    }
}

/// settings.json changed (or the tray changed it): every island applies it, and the
/// windows follow a new `monitor` choice.
pub fn sync_settings() {
    let settings = Settings::load();
    each_pill(|p| p.apply_settings(settings.clone()));
}

/// The look for `settings` as Windows is now (its app mode for Auto, its accent).
pub fn look_for(settings: &Settings) -> Look {
    Look::new(&settings.appearance, theme::apps_light(), theme::accent())
}

/// Windows' colours changed: rebuild only if the look comes out different.
pub fn colors_changed() {
    let mut current = None;
    PILLS.with_borrow(|pills| current = pills.first().map(|p| *p.look().0));
    if current.is_some_and(|look| look != look_for(&Settings::load())) {
        sync(true);
    }
}

/// In "cursor" mode, the island moves to the pointer's display before what the hub sent
/// shows up on it (a new activity, a card a hotkey opens).
pub fn follow_cursor() {
    let mut follow = false;
    PILLS.with(|cell| {
        if let Ok(pills) = cell.try_borrow() {
            follow = pills.len() == 1 && *pills[0].monitor_choice() == MonitorChoice::Cursor && pills[0].can_move();
        }
    });
    if follow {
        sync(false);
    }
}

/// Puts an island where the `monitor` setting says on each display it wants, at that
/// display's DPI: moves the ones whose display is still wanted (rebuilding those at another
/// scale, or all of them for a new look: `relook`), adds islands for new displays, removes
/// those whose display went. Every mode but All displays keeps exactly one island.
pub fn sync(relook: bool) {
    let settings = Settings::load();
    let mut look = look_for(&settings);
    if !relook {
        PILLS.with_borrow(|pills| {
            if let Some(p) = pills.first() {
                look = *p.look().0;
            }
        });
    }
    let wanted = place::placements(&settings.monitor, &look);
    if wanted.is_empty() {
        return;
    }
    let current: Vec<(HWND, String)> = PILLS.with_borrow(|pills| pills.iter().map(|p| (p.hwnd(), p.display().to_string())).collect());
    let single = settings.monitor != MonitorChoice::All;
    let mut placed: Vec<String> = Vec::new();
    for (i, (hwnd, display)) in current.iter().enumerate() {
        let target = match single {
            // One island: the first one goes wherever the setting says.
            true => (i == 0).then(|| wanted[0].clone()),
            false => wanted.iter().find(|w| w.name == *display).cloned(),
        };
        match target {
            Some(placement) => {
                placed.push(placement.name.clone());
                place_one(*hwnd, &placement, relook);
            }
            None => remove(*hwnd),
        }
    }
    for placement in wanted.iter().filter(|w| !placed.contains(&w.name)).take(if single { 1 - placed.len().min(1) } else { usize::MAX }) {
        if let Err(e) = create(placement, &settings) {
            eprintln!("island: can't add an island on {}: {e}", placement.name);
        }
    }
    let mut pointer = POINT::default();
    unsafe { _ = GetCursorPos(&mut pointer) };
    update_lead(pointer);
}

/// Moves `hwnd`'s island to `placement`, or rebuilds it there (another scale, a new look).
fn place_one(hwnd: HWND, placement: &Placement, relook: bool) {
    let mut moved = !relook;
    if moved {
        with_pill(hwnd, |pill| moved = pill.move_to(placement));
    }
    if !moved {
        rebuild(hwnd, placement);
    }
    with_pill(hwnd, |pill| pill.set_display(&placement.name));
}

/// The island on the pointer's display becomes the lead; a card open on the old lead
/// opens on the new one, then folds where it was.
pub fn update_lead(pointer: POINT) {
    PILLS.with(|cell| {
        let Ok(mut pills) = cell.try_borrow_mut() else { return };
        if pills.is_empty() {
            return;
        }
        let rects: Vec<RECT> = pills.iter().map(|p| p.screen()).collect();
        let current = pills.iter().position(|p| p.hwnd().0 as isize == LEAD.get());
        let new = match pills.len() {
            1 => 0,
            _ => place::holding(&rects, pointer).or(current).unwrap_or_default(),
        };
        if current == Some(new) {
            return;
        }
        LEAD.set(pills[new].hwnd().0 as isize);
        let card = current.and_then(|old| pills[old].open_card());
        for (i, pill) in pills.iter_mut().enumerate() {
            pill.set_lead(i == new);
        }
        if let (Some(old), Some(card)) = (current, card) {
            pills[new].take_over(card);
            pills[old].fold_after_hand_off();
        }
    });
}

/// A new window and island on a display.
fn create(placement: &Placement, settings: &Settings) -> Result<()> {
    let Some(to_hub) = TO_HUB.with_borrow(|t| t.clone()) else { return Ok(()) };
    let w = placement.window;
    let hwnd = unsafe {
        let instance = GetModuleHandleW(None)?;
        let ex = WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_NOREDIRECTIONBITMAP | WS_EX_LAYERED | WS_EX_TRANSPARENT;
        let hwnd = CreateWindowExW(ex, CLASS, w!("ReUI"), WS_POPUP, w.left, w.top, w.right - w.left, w.bottom - w.top, None, None, Some(instance.into()), None)?;
        SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA)?;
        hwnd
    };
    let look = look_for(settings);
    let mut pill = match Pill::new(hwnd, placement.scale, w, placement.monitor, !crate::animations_enabled(), settings.visibility, settings.appearance, look, to_hub, None) {
        Ok(pill) => pill,
        Err(e) => {
            unsafe { _ = DestroyWindow(hwnd) };
            return Err(e);
        }
    };
    pill.set_display(&placement.name);
    pill.set_lead(false);
    pill.apply_settings(settings.clone());
    // What the others show (an island added later starts where they are).
    let shown = PILLS.with_borrow(|pills| pills.first().map(|p| (p.received().clone(), p.places())));
    if let Some((snapshot, places)) = shown {
        pill.restore(snapshot, places);
    }
    PILLS.with_borrow_mut(|pills| pills.push(pill));
    // Things dragged onto an island go to the file shelf (its drop target answers only
    // near the island, `pill/drop.rs`).
    dragdrop::register(hwnd, Box::new(move |event| {
        let mut taken = false;
        with_pill(hwnd, |pill| taken = pill.on_drop_event(event));
        taken
    }));
    unsafe { _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
    eprintln!("island: on {}", placement.name);
    Ok(())
}

/// Takes an island away with its window (its display is gone, or one island is wanted).
fn remove(hwnd: HWND) {
    let gone = PILLS.with(|cell| {
        let mut pills = cell.try_borrow_mut().ok()?;
        let at = pills.iter().position(|p| p.hwnd() == hwnd)?;
        Some(pills.remove(at))
    });
    let Some(mut pill) = gone else { return };
    pill.give_back_open(false);
    eprintln!("island: off {}", pill.display());
    drop(pill);
    if LEAD.get() == hwnd.0 as isize {
        LEAD.set(0);
    }
    unsafe { _ = DestroyWindow(hwnd) };
}

/// A new pill for another scale (per-monitor DPI) or look: the old one hands over its
/// snapshot and GPU objects (so a style switch takes milliseconds, not the ~140 ms a new
/// device and fonts cost), the window is resized, and the new one picks up where it left
/// off, its summoned card open again.
fn rebuild(hwnd: HWND, placement: &Placement) {
    let taken = PILLS.with(|cell| {
        let mut pills = cell.try_borrow_mut().ok()?;
        let at = pills.iter().position(|p| p.hwnd() == hwnd)?;
        Some((at, pills.remove(at)))
    });
    let Some((at, mut old)) = taken else { return };
    let Some(to_hub) = TO_HUB.with_borrow(|t| t.clone()) else { return };
    eprintln!("island: rebuilding for scale {}", placement.scale);
    let reopen = old.reopen_after_move();
    let lead = LEAD.get() == hwnd.0 as isize;
    old.give_back_open(true);
    let snapshot = old.take_snapshot();
    let places = old.places();
    let gpu = old.into_gpu();
    unsafe {
        for id in PILL_TIMERS {
            _ = KillTimer(Some(hwnd), id);
        }
        let w = placement.window;
        _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), w.left, w.top, w.right - w.left, w.bottom - w.top, SWP_NOACTIVATE | SWP_NOOWNERZORDER);
    }
    let settings = Settings::load();
    let look = look_for(&settings);
    match Pill::new(hwnd, placement.scale, placement.window, placement.monitor, !crate::animations_enabled(), settings.visibility, settings.appearance, look, to_hub, Some(gpu)) {
        Ok(mut pill) => {
            pill.set_display(&placement.name);
            pill.set_lead(lead);
            pill.apply_settings(settings);
            pill.restore(snapshot, places);
            pill.recheck_fullscreen();
            if let Some(id) = reopen {
                pill.reopen(id);
            }
            PILLS.with_borrow_mut(|pills| {
                let at = at.min(pills.len());
                pills.insert(at, pill);
            });
        }
        Err(e) => {
            // Without a pill there's nothing to show here; the hub starts a fresh island.
            eprintln!("island: can't rebuild the pill: {e}");
            unsafe { PostQuitMessage(1) };
        }
    }
}

/// Every island goes (the process is quitting), then what they shared.
pub fn clear() {
    PILLS.with_borrow_mut(|pills| pills.clear());
    TO_HUB.with_borrow_mut(|t| *t = None);
    EVENTS.with_borrow_mut(|e| *e = None);
    pill::release_shared();
}
