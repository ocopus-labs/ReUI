//! The idle island (no activities; `features::idle`): the time at rest when `idle_clock`
//! is on, the date and time on a peek, and a click opening the row of places, the same
//! destinations and glyphs as the view switcher. A tab opens its place exactly as a
//! switcher tab does (the module's own open command); Esc, clicking elsewhere or leaving
//! closes the row. The clock redraws once a minute only while it's on screen.

use reui_design::layout::switcher::Switcher;
use windows::Win32::Foundation::POINT;

use crate::features::clipboard_keys;
use crate::features::idle::{self, Face};
use crate::render::IdleContent;

use super::{Pill, State};

/// The idle pill's own state.
#[derive(Default)]
pub(super) struct Idle {
    /// The `idle_clock` setting.
    pub clock: bool,
    /// The place under the pointer while the row is open.
    pub hover: Option<usize>,
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

    /// Esc while the row is open: the keyboard hook passes every other key on.
    pub(super) fn places_key(&mut self) {
        if self.state == State::Places {
            self.set_state(State::Compact);
        }
    }

    /// Leaving the row (closing it, or a place opening) forgets the hover.
    pub(super) fn left_places(&mut self) {
        self.idle.hover = None;
        clipboard_keys::escape_only(false);
    }
}
