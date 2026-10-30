//! The island's menus, opened from the window procedure: the island's own menu (a
//! right-click on the pill or the tray icon), led by a right-clicked moment's quiet entries
//! (`quiet.rs`), and an open card's menus (a card's actions, a slider's output devices).
//! Each runs outside the pill's borrow so hooks and timers keep working meanwhile.

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::DestroyWindow;

use crate::pill::MenuEntry;
use crate::{tray, with_pill};

/// The island's menu, led by `moment`'s entries when it was opened on a moment that can be
/// quieted (`quiet.rs`).
pub fn open_menu(hwnd: HWND, moment: Vec<MenuEntry>) {
    let (mut paused, mut hidden) = (false, false);
    with_pill(|pill| (paused, hidden) = (pill.paused(), pill.hidden_by_user()));
    let labels: Vec<&str> = moment.iter().map(|(label, ..)| label.as_str()).collect();
    with_pill(|pill| pill.menu_opened());
    let (picked, previous) = tray::menu(hwnd, paused, hidden, &labels);
    with_pill(|pill| pill.menu_closed(previous));
    match picked {
        Some(tray::Command::Moment(i)) => with_pill(|pill| pill.quiet_pick(&moment[i].1)),
        Some(tray::Command::Settings) => tray::open_settings(),
        Some(tray::Command::TogglePause) => with_pill(|pill| pill.set_paused(!paused)),
        Some(tray::Command::ToggleHidden) => with_pill(|pill| pill.toggle_hidden()),
        Some(tray::Command::Module(module, action)) => with_pill(|pill| pill.command(module, &action)),
        Some(tray::Command::Quit) => unsafe { _ = DestroyWindow(hwnd) },
        None => {}
    }
}

/// Right-click: a card's own menu over a card that has one, the island's menu elsewhere
/// (led by the moment's quiet entries when it has them).
pub fn right_click(hwnd: HWND, x: f32, y: f32) {
    let (mut entries, mut moment) = (None, Vec::new());
    with_pill(|pill| {
        entries = pill.card_menu(x, y);
        if entries.is_none() {
            moment = pill.moment_menu(x, y);
        }
    });
    match entries {
        Some(entries) => card_menu(hwnd, entries),
        None => open_menu(hwnd, moment),
    }
}

/// A menu from the open card (a card's actions, a slider's output devices).
pub fn card_menu(hwnd: HWND, entries: Vec<MenuEntry>) {
    if entries.is_empty() {
        return;
    }
    with_pill(|pill| pill.menu_opened());
    let items: Vec<(&str, bool)> = entries.iter().map(|(label, _, _, checked)| (label.as_str(), *checked)).collect();
    let (picked, previous) = tray::item_menu(hwnd, &items);
    with_pill(|pill| {
        pill.menu_closed(previous);
        if let Some(i) = picked {
            let (_, action, closes, _) = &entries[i];
            pill.act(action.clone(), *closes);
        }
    });
}
