//! A card summoned by its hotkey: bringing it forward, dismissing it, telling modules it
//! opened, and the keyboard and search it captures while open.


use reui_design::tokens::motion;
use reui_protocol::{
    ActivityId, ClientMsg, ItemKey, Priority, Row, ACTION_COLLAPSED, ACTION_EXPANDED, ACTION_SEARCH, ACTION_SUBMIT,
    ACTION_UNDO,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::features::choices;
use crate::features::keys::{self, Key};
use crate::features::clipboard_keys;
use crate::settings::Visibility;

use super::{Pill, State};
use super::timers::*;

/// Typing is sent to the module once it pauses this long; the field shows it at once.
const SEARCH_DEBOUNCE_MS: u32 = 60;

/// Enter right after typing waits for the results of what was typed, at most this long.
const AWAIT_RESULTS_MS: u32 = 400;

impl Pill {
    /// The hub asked to open this activity (the user pressed its hotkey): bring it forward
    /// over anything (showing the island even in a hidden mode) and expand it until
    /// dismissed. Asking again while it's open closes it.
    pub(super) fn focus(&mut self, id: ActivityId) {
        if self.hidden || !self.raw.activities.iter().any(|a| a.id == id) {
            return;
        }
        if self.summoned && self.state == State::Expanded && self.pinned.as_ref() == Some(&id) {
            self.set_state(State::Compact);
            return;
        }
        self.summoned = true;
        self.bring_forward(id);
        if self.visibility != Visibility::Always {
            self.revealed = true;
            self.refresh_visibility();
            unsafe { SetTimer(Some(self.hwnd), TIMER_REVEAL, motion::delay::REVEAL_HOLD, None) };
        }
        self.set_state(State::Expanded);
        if !self.hovered {
            unsafe { SetTimer(Some(self.hwnd), TIMER_COLLAPSE, motion::delay::PANEL_IDLE, None) };
        }
    }

    /// The island's open shortcut (docs/PRD-keyboard-island.md): what's waiting opens with
    /// the keyboard on it. A decision first (as if clicked: Enter answers, Esc gives the
    /// keys back); else the card in front, summoned as its own hotkey would; with nothing
    /// in front, the row of places, walked with the arrows. Pressed again while the island
    /// has the keyboard, it lets go as Esc would.
    pub(super) fn open_from_shortcut(&mut self) {
        if self.hidden {
            return;
        }
        if clipboard_keys::held_by(self.hwnd) && (self.engaged || self.summoned || self.state == State::Places) {
            return self.let_go();
        }
        let decision = self.snapshot.activities.iter().find(|a| a.priority == Priority::Decision && a.expanded.as_ref().is_some_and(choices::wants_keys)).map(|a| a.id.clone());
        if let Some(id) = decision {
            if self.front().map(|a| &a.id) != Some(&id) {
                // As the switcher's waiting dot does: the place gives way to it.
                self.summoned = false;
                self.bring_forward(id);
            }
            self.set_state(State::Expanded);
            // After the state: bringing it forward forgets an older card's keyboard.
            self.engaged = true;
            self.update_capture();
            self.redraw_expanded();
            return;
        }
        match self.front().filter(|a| a.expanded.is_some()).map(|a| a.id.clone()) {
            Some(id) => self.focus(id),
            None if self.idle() && !self.paused && !self.hidden_by_user => self.open_places_from_keyboard(),
            None => {}
        }
    }

    /// The open shortcut pressed again: as Esc.
    fn let_go(&mut self) {
        match self.state {
            State::Places => self.set_state(State::Compact),
            _ if self.summoned => self.set_state(State::Compact),
            _ => {
                self.engaged = false;
                self.update_capture();
                self.redraw_expanded();
            }
        }
    }

    /// Another island took the keyboard (`clipboard_keys::WM_KEYS_TAKEN`): this one lets go
    /// of it, as Esc would, so it doesn't take the keys back on its next redraw.
    pub fn keys_taken(&mut self) {
        match self.state {
            State::Places => self.set_state(State::Compact),
            State::Expanded if self.summoned => self.set_state(State::Compact),
            _ => {
                self.engaged = false;
                self.update_capture();
                if self.state == State::Expanded {
                    self.redraw_expanded();
                }
            }
        }
    }

    /// Closes a summoned card when the user clearly moved on.
    pub(super) fn dismiss_summoned(&mut self) {
        if self.summoned && self.state == State::Expanded {
            self.set_state(State::Compact);
        }
    }

    /// This island goes away with its card open. Rebuilt for another scale (`quietly`), its
    /// successor opens the card again, so the module hears nothing; its display gone, the
    /// module hears it closed if no other island shows it.
    pub fn give_back_open(&mut self, quietly: bool) {
        if let Some(old) = self.open_id.take() {
            if crate::open_cards::closed(&old) && !quietly {
                _ = self.to_hub.send(ClientMsg::Action { id: old, action: ACTION_COLLAPSED.into() });
            }
        }
    }

    /// Tells modules when their card opens and closes (`expanded` / `collapsed`), so e.g.
    /// the clipboard keeps its history card while it's being used.
    pub(super) fn sync_open(&mut self) {
        let now_open = if self.state == State::Expanded { self.front().map(|a| a.id.clone()) } else { None };
        if now_open == self.open_id {
            return;
        }
        // Counted across the islands (`open_cards`): the module hears the first open and
        // the last close.
        if let Some(old) = self.open_id.take() {
            if crate::open_cards::closed(&old) {
                _ = self.to_hub.send(ClientMsg::Action { id: old, action: ACTION_COLLAPSED.into() });
            }
        }
        if let Some(new) = &now_open {
            if crate::open_cards::opened(new) {
                _ = self.to_hub.send(ClientMsg::Action { id: new.clone(), action: ACTION_EXPANDED.into() });
            }
        }
        self.open_id = now_open;
    }

    /// The panel reads the keyboard only while an open, showing card can use it (a search
    /// field, a choice list) and was asked to: opened by its hotkey, or clicked. The hook is
    /// gone the moment that stops being true.
    pub(super) fn update_capture(&mut self) {
        // The idle pill's row of places reads Esc alone to close it, and the arrows and
        // Enter too when the island's shortcut opened it. A summoned card with nothing to
        // type into or pick still reads Esc, so Esc always closes what a shortcut opened.
        let places = self.state == State::Places && self.shown;
        let open = self.state == State::Expanded && self.shown && (self.summoned || self.engaged);
        let keys = open && self.front_view().is_some_and(choices::wants_keys);
        let escape = places || open && self.summoned && !keys;
        let want = escape || keys;
        if want {
            // The hook's limit is shared by the islands: only the one holding the keys sets it.
            clipboard_keys::limit(match () {
                _ if places && self.idle.keyboard => clipboard_keys::Limit::Navigation,
                _ if escape => clipboard_keys::Limit::Escape,
                _ => clipboard_keys::Limit::All,
            });
        }
        if want == clipboard_keys::held_by(self.hwnd) {
            return;
        }
        if want {
            clipboard_keys::capture(self.hwnd);
            if !clipboard_keys::held_by(self.hwnd) || !keys {
                return;
            }
            self.panel.query = Some(self.panel.module_query().unwrap_or_default().to_string());
            self.panel.select(self.panel.selected);
            if self.choice_cursor.is_none() && self.front_view().is_some_and(|v| v.rows.iter().any(|r| matches!(r, Row::Choices { .. }))) {
                self.choice_cursor = Some(0);
            }
        } else {
            clipboard_keys::release(self.hwnd);
            self.panel.query = None;
            self.search_dirty = false;
            self.awaiting = None;
            unsafe {
                _ = KillTimer(Some(self.hwnd), TIMER_SEARCH);
                _ = KillTimer(Some(self.hwnd), TIMER_AWAIT);
            }
        }
        // Caret and ring come and go; a closing card keeps its last frame.
        if self.state == State::Expanded {
            self.redraw_expanded();
            self.update_scroll();
        }
    }

    /// Keys the hook queued (see `clipboard_keys`).
    pub fn on_keys(&mut self) {
        let keys = clipboard_keys::drain();
        if keys.is_empty() || !clipboard_keys::held_by(self.hwnd) {
            return;
        }
        for key in keys {
            if !clipboard_keys::held_by(self.hwnd) {
                break;
            }
            self.on_key(key);
        }
        // Typing counts as using the panel.
        if self.summoned && self.state == State::Expanded && !self.hovered {
            unsafe { SetTimer(Some(self.hwnd), TIMER_COLLAPSE, motion::delay::PANEL_IDLE, None) };
        }
        if self.state == State::Expanded {
            self.redraw_expanded();
            self.update_scroll();
        }
    }

    fn on_key(&mut self, key: Key) {
        if self.state == State::Places {
            return self.places_key(key);
        }
        if let Key::View(step) = key {
            return self.step_place(step);
        }
        if self.on_answer_key(key) {
            return;
        }
        let Some(query) = self.panel.query.as_mut() else {
            // A summoned card with no field (stats, music): Esc closes it.
            if key == Key::Escape && self.summoned {
                self.set_state(State::Compact);
            }
            return;
        };
        let edited = match key {
            Key::Type(c) => {
                query.push(c);
                true
            }
            Key::Backspace => query.pop().is_some(),
            Key::BackspaceWord => {
                let before = query.len();
                keys::delete_word(query);
                query.len() != before
            }
            Key::Escape if !query.is_empty() => {
                query.clear();
                self.search_dirty = true;
                self.flush_search();
                return;
            }
            Key::Escape => {
                self.set_state(State::Compact);
                return;
            }
            _ => false,
        };
        if edited {
            self.search_dirty = true;
            unsafe { SetTimer(Some(self.hwnd), TIMER_SEARCH, SEARCH_DEBOUNCE_MS, None) };
            return;
        }
        if matches!(key, Key::Activate | Key::Item(_) | Key::Nth(_)) && self.search_pending() {
            // Act on the results of what was typed, not the list still showing.
            self.flush_search();
            self.awaiting = Some(key);
            unsafe { SetTimer(Some(self.hwnd), TIMER_AWAIT, AWAIT_RESULTS_MS, None) };
            return;
        }
        let selected = self.panel.selected;
        let count = self.items().len();
        let action = match key {
            Key::Move(d) => {
                self.panel.select(selected.saturating_add_signed(d as isize));
                None
            }
            Key::First => {
                self.panel.select(0);
                None
            }
            Key::Last => {
                self.panel.select(usize::MAX);
                None
            }
            Key::Activate if count == 0 => Some(ACTION_SUBMIT.to_string()),
            Key::Activate => self.items().get(selected).map(|i| i.action.clone()),
            Key::Item(k) => self.item_action(selected, k),
            Key::Nth(n) => {
                let i = self.panel.row().map_or(0, |r| r.first_shown(self.panel.scroll)) + n;
                (i < count).then(|| self.items()[i].action.clone())
            }
            Key::Tab(d) => self.front_view().and_then(|v| {
                let tabs = v.rows.iter().find_map(|r| if let Row::Tabs(t) = r { Some(t) } else { None })?;
                let at = tabs.iter().position(|t| t.selected).unwrap_or(0) as i32;
                let next = (at + d).rem_euclid(tabs.len().max(1) as i32) as usize;
                tabs.get(next).filter(|t| !t.selected).map(|t| t.action.clone())
            }),
            Key::Undo => Some(ACTION_UNDO.to_string()),
            Key::Type(_) | Key::Backspace | Key::BackspaceWord | Key::Escape | Key::View(_) => None,
        };
        if let Some(action) = action {
            let pastes = matches!(key, Key::Activate | Key::Nth(_) | Key::Item(ItemKey::ShiftEnter | ItemKey::CtrlEnter));
            self.act(action, pastes);
        }
    }

    /// Keys for a card that asks something (see `features::choices`); `false` leaves the
    /// key to the panel (the field, a summoned card's Esc).
    fn on_answer_key(&mut self, key: Key) -> bool {
        let Some(view) = self.front_view() else { return false };
        let asks = view.rows.iter().any(|r| matches!(r, Row::Choices { .. }));
        if !asks && !(self.engaged && !self.summoned) {
            return false;
        }
        let typing = self.panel.query.as_deref().is_some_and(|q| !q.is_empty());
        match choices::on_key(key, view, self.choice_cursor, typing, self.summoned) {
            choices::Effect::Pass => return false,
            choices::Effect::Ignore => {}
            choices::Effect::Cursor(i) => self.set_choice_cursor(i),
            choices::Effect::Send { action, at } => {
                if let Some(i) = at {
                    self.set_choice_cursor(i);
                }
                self.send_action(action);
            }
            choices::Effect::Release => {
                self.engaged = false;
                self.update_capture();
            }
        }
        true
    }

    /// Typed text is still on its way to the module, or its results haven't come back.
    pub(super) fn search_pending(&self) -> bool {
        self.search_dirty || self.panel.query.as_deref().is_some_and(|q| Some(q) != self.panel.module_query())
    }

    pub(super) fn flush_search(&mut self) {
        unsafe { _ = KillTimer(Some(self.hwnd), TIMER_SEARCH) };
        if std::mem::take(&mut self.search_dirty) {
            let query = self.panel.query.clone().unwrap_or_default();
            self.send_action(format!("{ACTION_SEARCH}{query}"));
        }
    }

    pub(super) fn run_awaiting(&mut self) {
        unsafe { _ = KillTimer(Some(self.hwnd), TIMER_AWAIT) };
        if let Some(key) = self.awaiting.take() {
            self.on_key(key);
        }
    }
}
