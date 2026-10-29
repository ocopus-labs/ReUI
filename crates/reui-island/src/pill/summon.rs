//! A card summoned by its hotkey: bringing it forward, dismissing it, telling modules it
//! opened, and the keyboard and search it captures while open.


use reui_design::tokens::motion;
use reui_protocol::{
    ActivityId, ClientMsg, ItemKey, Row, ACTION_COLLAPSED, ACTION_EXPANDED, ACTION_SEARCH, ACTION_SUBMIT,
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

    /// Closes a summoned card when the user clearly moved on.
    pub(super) fn dismiss_summoned(&mut self) {
        if self.summoned && self.state == State::Expanded {
            self.set_state(State::Compact);
        }
    }

    /// Tells modules when their card opens and closes (`expanded` / `collapsed`), so e.g.
    /// the clipboard keeps its history card while it's being used.
    pub(super) fn sync_open(&mut self) {
        let now_open = if self.state == State::Expanded { self.front().map(|a| a.id.clone()) } else { None };
        if now_open == self.open_id {
            return;
        }
        if let Some(old) = self.open_id.take() {
            _ = self.to_hub.send(ClientMsg::Action { id: old, action: ACTION_COLLAPSED.into() });
        }
        if let Some(new) = &now_open {
            _ = self.to_hub.send(ClientMsg::Action { id: new.clone(), action: ACTION_EXPANDED.into() });
        }
        self.open_id = now_open;
    }

    /// The panel reads the keyboard only while an open, showing card can use it (a search
    /// field, a choice list) and was asked to: opened by its hotkey, or clicked. The hook is
    /// gone the moment that stops being true.
    pub(super) fn update_capture(&mut self) {
        let want = self.state == State::Expanded && self.shown && (self.summoned || self.engaged) && self.front_view().is_some_and(choices::wants_keys);
        if want == clipboard_keys::captured() {
            return;
        }
        if want {
            clipboard_keys::capture(self.hwnd);
            if !clipboard_keys::captured() {
                return;
            }
            self.panel.query = Some(self.panel.module_query().unwrap_or_default().to_string());
            self.panel.select(self.panel.selected);
            if self.choice_cursor.is_none() && self.front_view().is_some_and(|v| v.rows.iter().any(|r| matches!(r, Row::Choices { .. }))) {
                self.choice_cursor = Some(0);
            }
        } else {
            clipboard_keys::release();
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
        if keys.is_empty() || !clipboard_keys::captured() {
            return;
        }
        for key in keys {
            if !clipboard_keys::captured() {
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
        if self.on_answer_key(key) {
            return;
        }
        let Some(query) = self.panel.query.as_mut() else { return };
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
            Key::Type(_) | Key::Backspace | Key::BackspaceWord | Key::Escape => None,
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
