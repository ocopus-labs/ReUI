//! The idle island (no activities; `features::idle`): the time at rest when `idle_clock`
//! is on, the date and time on a peek, and a click opening the row of places, the same
//! destinations and glyphs as the view switcher. A tab opens its place exactly as a
//! switcher tab does (the module's own open command); Esc, clicking elsewhere or leaving
//! closes the row. Opened with the island's shortcut instead, the arrows walk it and Enter
//! opens a place (docs/PRD-keyboard-island.md). The clock redraws once a minute only while
//! it's on screen.

use reui_design::layout::switcher::Switcher;
use windows::Win32::Foundation::POINT;

use crate::features::clipboard_keys;
use crate::features::keys::Key;
use crate::features::idle::{self, Face};
use crate::render::IdleContent;

use super::{Pill, State};

/// The idle pill's own state.
#[derive(Default)]
pub(super) struct Idle {
    /// The `idle_clock` setting.
    pub clock: bool,
    /// The place under the pointer (or the keyboard's cursor) while the row is open.
    pub hover: Option<usize>,
    /// The row was opened with the island's shortcut: the arrows and Enter are read too.
    pub keyboard: bool,
}

impl Pill {
    /// What the idle pill shows now; `None` while something is happening.
    fn idle_face(&self) -> Option<Face> {
        if !self.idle() {
            return None;
        }
        Some(idle::face(self.state == State::Peek, self.state == State::Places, self.idle.clock))
    }

    /// Draws the idle pill for its state, and re-springs it when its width changed (the
    /// date and time are wider than the resting pill, the row is as wide as its places).
    pub(super) fn draw_idle(&mut self) {
        let Some(face) = self.idle_face() else { return };
        let (date, time, _) = if face.shows_time() { idle::now() } else { Default::default() };
        let places: Vec<char> = self.tabs.places.iter().take(reui_design::layout::switcher::MAX_TABS).map(|p| p.glyph).collect();
        let content = IdleContent { date: &date, time: &time, places: &places, hover: self.idle.hover };
        let rest = super::setup::idle_size(&self.look).0;
        match self.renderer.draw_idle(&self.compact_surface, &face, &content, rest) {
            Ok(width) if width != self.compact_width => {
                self.compact_width = width;
                self.retarget();
            }
            Ok(_) => {}
            Err(e) => eprintln!("island: idle redraw failed: {e}"),
        }
        self.update_clock_tick();
    }

    /// Milliseconds until the idle clock on screen next reads differently.
    pub(super) fn idle_clock_due(&self) -> Option<u64> {
        self.idle_face().filter(Face::shows_time).map(|_| u64::from(idle::now().2))
    }

    /// The `idle_clock` setting.
    pub(super) fn set_idle_clock(&mut self, on: bool) {
        if on != self.idle.clock {
            self.idle.clock = on;
            self.draw_idle();
        }
    }

    /// A click on the idle pill (`true` when it was one): resting or peeking, it opens the
    /// row of places; open, a tab goes to its place.
    pub(super) fn click_idle(&mut self, x: f32, y: f32) -> bool {
        if !self.idle() {
            return false;
        }
        match self.state {
            State::Places => {
                let pt = POINT { x: (x + self.window.left as f32) as i32, y: (y + self.window.top as f32) as i32 };
                if let Some(place) = self.places_tab(pt).and_then(|i| self.tabs.places.get(i)).cloned() {
                    // The module publishes its card and asks for focus; the first snapshot
                    // with it folds the row away (`render`), and the card opens summoned.
                    self.command(&place.module, &place.open);
                }
            }
            _ if !self.tabs.places.is_empty() => self.set_state(State::Places),
            _ => {}
        }
        true
    }

    /// The place under screen point `pt` while the row is open.
    fn places_tab(&self, pt: POINT) -> Option<usize> {
        if self.state != State::Places {
            return None;
        }
        let layout = Switcher::new(&self.look, self.tabs.places.len());
        let s = self.scale;
        let win_w = (self.window.right - self.window.left) as f32;
        let left = self.window.left as f32 + (win_w - layout.width * s) / 2.0;
        let top = self.window.top as f32 + self.look.top() * s;
        layout.tab_at((pt.x as f32 - left) / s, (pt.y as f32 - top) / s)
    }

    /// The tab under the pointer shows it; redrawn only when that changes.
    pub(super) fn hover_places(&mut self, pt: POINT) {
        let want = if self.shown { self.places_tab(pt) } else { None };
        if want != self.idle.hover {
            self.idle.hover = want;
            self.draw_idle();
        }
    }

    /// The row of places from the island's shortcut, the first place under the cursor;
    /// in a hidden visibility mode the island shows for it, as for a summoned card.
    pub(super) fn open_places_from_keyboard(&mut self) {
        if self.tabs.places.is_empty() {
            return;
        }
        if self.visibility != crate::settings::Visibility::Always {
            self.revealed = true;
            self.refresh_visibility();
            unsafe { windows::Win32::UI::WindowsAndMessaging::SetTimer(Some(self.hwnd), super::timers::TIMER_REVEAL, reui_design::tokens::motion::delay::REVEAL_HOLD, None) };
        }
        self.idle.keyboard = true;
        self.idle.hover = Some(0);
        self.set_state(State::Places);
        self.update_capture();
        self.draw_idle();
    }

    /// A key while the row is open: Esc closes it; opened from the keyboard, the arrows
    /// move along it and Enter opens the place under the cursor. The keyboard hook passes
    /// every other key on.
    pub(super) fn places_key(&mut self, key: Key) {
        if self.state != State::Places {
            return;
        }
        let count = self.tabs.places.len().min(reui_design::layout::switcher::MAX_TABS);
        if !self.idle.keyboard || count == 0 {
            if key == Key::Escape {
                self.set_state(State::Compact);
            }
            return;
        }
        match places_step(key, self.idle.hover, count) {
            PlacesStep::Close => self.set_state(State::Compact),
            PlacesStep::Cursor(at) => {
                self.idle.hover = Some(at);
                self.draw_idle();
            }
            PlacesStep::Open(at) => {
                if let Some(place) = self.tabs.places.get(at).cloned() {
                    // As a click on the tab: the module opens its card summoned.
                    self.command(&place.module, &place.open);
                }
            }
            PlacesStep::Nothing => {}
        }
    }

    /// Leaving the row (closing it, or a place opening) forgets the hover.
    pub(super) fn left_places(&mut self) {
        self.idle.hover = None;
        self.idle.keyboard = false;
        if clipboard_keys::held_by(self.hwnd) {
            clipboard_keys::limit(clipboard_keys::Limit::All);
        }
    }
}

/// What a key does in the row of places opened from the keyboard. Pure.
#[derive(Debug, PartialEq, Eq)]
enum PlacesStep {
    Close,
    Cursor(usize),
    Open(usize),
    Nothing,
}

fn places_step(key: Key, cursor: Option<usize>, count: usize) -> PlacesStep {
    let last = count.saturating_sub(1);
    match key {
        Key::Escape => PlacesStep::Close,
        Key::Move(d) => PlacesStep::Cursor(match cursor {
            Some(at) => at.saturating_add_signed(d as isize).min(last),
            None if d < 0 => last,
            None => 0,
        }),
        Key::First => PlacesStep::Cursor(0),
        Key::Last => PlacesStep::Cursor(last),
        Key::Activate => PlacesStep::Open(cursor.unwrap_or(0).min(last)),
        _ => PlacesStep::Nothing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keyboard_walks_the_row_and_stops_at_its_ends() {
        assert_eq!(places_step(Key::Move(1), Some(0), 3), PlacesStep::Cursor(1));
        assert_eq!(places_step(Key::Move(1), Some(2), 3), PlacesStep::Cursor(2));
        assert_eq!(places_step(Key::Move(-1), Some(0), 3), PlacesStep::Cursor(0));
        assert_eq!(places_step(Key::Move(-1), None, 3), PlacesStep::Cursor(2));
        assert_eq!((places_step(Key::First, Some(2), 3), places_step(Key::Last, None, 3)), (PlacesStep::Cursor(0), PlacesStep::Cursor(2)));
        assert_eq!(places_step(Key::Activate, Some(1), 3), PlacesStep::Open(1));
        assert_eq!(places_step(Key::Activate, None, 3), PlacesStep::Open(0));
        assert_eq!(places_step(Key::Escape, Some(1), 3), PlacesStep::Close);
        assert_eq!(places_step(Key::Type('a'), Some(1), 3), PlacesStep::Nothing);
    }
}
