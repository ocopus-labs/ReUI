//! Pointer input: hover and peek, clicks, the wheel, dragging the scrubber and sliders,
//! and the context menus of the open card.

use std::time::Instant;

use reui_design::tokens::{component, motion, typography};
use reui_protocol::{
    ActivityId, ClientMsg, ItemKey, Row, ACTION_ALT, ACTION_SWIPE_NEXT, ACTION_SWIPE_PREV,
};
use windows::Win32::Foundation::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::drag_guard;
use crate::features::choices;
use crate::features::clipboard_keys;
use crate::features::music::{self, SeekHold, Throttle};
use crate::render::{self, expanded_surface, Thumb};
use crate::settings::Visibility;

use super::{Drag, MenuEntry, Pill, State};
use super::timers::*;

/// The module whose first-run hint is told about hover intent (no other module gets it).
const ONBOARDING: &str = "onboarding";
const ACTION_HOVERED: &str = "hovered";

/// Whether any mouse button is down now, as drag-guard bits (which one doesn't matter).
fn held_buttons() -> u8 {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON};
    let down = [VK_LBUTTON, VK_RBUTTON, VK_MBUTTON].iter().any(|&vk| unsafe { GetAsyncKeyState(vk.0 as i32) } as u16 & 0x8000 != 0);
    if down {
        drag_guard::LEFT | drag_guard::RIGHT | drag_guard::MIDDLE
    } else {
        0
    }
}

impl Pill {
    /// A button (`drag_guard::LEFT`...) went down anywhere: outside a summoned card, that
    /// dismisses it; away from the island, it may be the start of a drag (see `DragGuard`).
    pub fn on_mouse_down(&mut self, pt: POINT, button: u8) {
        let (x, y) = (pt.x as f32, pt.y as f32);
        let within = |(l, t, r, b): (f32, f32, f32, f32)| x >= l && x < r && y >= t && y < b;
        let interactive = self.shown;
        let on_switcher = self.switcher_zone().is_some_and(within);
        let on_island = interactive && (within(self.shape_rect()) || self.bubble_rect().is_some_and(within) || on_switcher);
        self.drag_guard.press(button, on_island || self.menu_open);
        if self.menu_open {
            return;
        }
        // A press on the switcher is going to another place, not away.
        if !within(self.shape_rect()) && !on_switcher {
            self.dismiss_summoned();
            // Elsewhere, the idle pill's row of places folds away.
            if self.state == State::Places {
                self.set_state(State::Idle);
            }
        }
    }

    /// A button came up: a drag from elsewhere is over, so resting on the island counts again.
    pub fn on_mouse_up(&mut self, pt: POINT, button: u8) {
        let blocked = self.drag_guard.blocks();
        self.drag_guard.release(button);
        if blocked && !self.drag_guard.blocks() && self.drag.is_none() {
            self.on_mouse_move(pt);
        }
    }

    pub fn on_mouse_move(&mut self, pt: POINT) {
        if self.drag.is_some() {
            // The card stays as it is (no leave, no collapse) until the button comes up.
            return self.on_drag(pt);
        }
        if self.drag_guard.blocks() {
            self.drag_guard.sync(held_buttons());
        }
        // Dragging something across the screen: the island neither opens nor takes clicks.
        let dragging = self.drag_guard.blocks();
        // In a hidden mode, pushing the pointer against the top edge above the island reveals it.
        let at_edge = pt.y <= self.window.top + 1 && pt.x >= self.window.left && pt.x < self.window.right;
        if at_edge && !dragging && !self.hidden && self.visibility != Visibility::Always && !self.shown {
            self.revealed = true;
            self.refresh_visibility();
            unsafe { SetTimer(Some(self.hwnd), TIMER_REVEAL, motion::delay::REVEAL_HOLD, None) };
        }
        // The idle pill too: it peeks with the date and time, and opens into the places.
        let interactive = self.shown && !dragging;
        let (x, y) = (pt.x as f32, pt.y as f32);
        let within = |(l, t, r, b): (f32, f32, f32, f32)| x >= l && x < r && y >= t && y < b;
        let on_bubble = interactive && self.bubble_rect().is_some_and(within);
        // The switcher and the gap above it count as the card, so crossing to a tab never
        // starts its collapse.
        let on_switcher = interactive && self.switcher_zone().is_some_and(within);
        let inside = interactive && (within(self.hit_rect()) || on_bubble || on_switcher);
        self.on_bubble = on_bubble;
        // On the visible pill (or bubble) itself, clicks are the island's at once: a quick
        // click there is clearly aimed at it. The rest delay only gates the peek, and the
        // thin band above the pill, where browser tabs actually are.
        if interactive && (within(self.shape_rect()) || on_bubble || on_switcher) {
            self.set_clickthrough(false);
        }
        self.hover_switcher(pt);
        self.hover_places(pt);
        self.update_thumb(pt);
        self.update_hover_card(pt, inside);
        if inside == self.inside {
            return;
        }
        self.inside = inside;
        if inside {
            // Only take over once the pointer rests here (see `hovered`).
            unsafe { SetTimer(Some(self.hwnd), TIMER_HOVER, motion::delay::HOVER_INTENT, None) };
            return;
        }
        unsafe { _ = KillTimer(Some(self.hwnd), TIMER_HOVER) };
        if self.revealed {
            // Hide again a while after the pointer leaves.
            unsafe { SetTimer(Some(self.hwnd), TIMER_REVEAL, motion::delay::REVEAL_HOLD, None) };
        }
        let was_hovered = std::mem::replace(&mut self.hovered, false);
        self.set_clickthrough(true);
        match self.state {
            State::Expanded => unsafe {
                let delay = if self.summoned { motion::delay::PANEL_IDLE } else { motion::delay::COLLAPSE };
                _ = SetTimer(Some(self.hwnd), TIMER_COLLAPSE, delay, None);
            },
            // Like a card opened with a click: it folds a moment after the pointer leaves.
            State::Places => unsafe {
                _ = SetTimer(Some(self.hwnd), TIMER_COLLAPSE, motion::delay::COLLAPSE, None);
            },
            State::Peek if was_hovered => self.set_state(State::Compact),
            _ => {}
        }
    }

    pub(super) fn on_hover_intent(&mut self) {
        if !self.inside || !self.shown || self.drag_guard.blocks() {
            return;
        }
        self.hovered = true;
        // The first-run "Hover me" hint waits for exactly this.
        if let Some(front) = self.front().filter(|a| a.id.module == ONBOARDING) {
            _ = self.to_hub.send(ClientMsg::Action { id: front.id.clone(), action: ACTION_HOVERED.into() });
        }
        unsafe { _ = KillTimer(Some(self.hwnd), TIMER_COLLAPSE) };
        self.set_clickthrough(false);
        // Resting on the bubble makes it clickable without growing the pill. The idle pill
        // peeks too, with the date and time.
        if matches!(self.state, State::Compact | State::Idle) && !self.on_bubble {
            self.set_state(State::Peek);
        }
    }

    /// `x`, `y` are client pixels. Shift+click on a card is its Shift+Enter action; Alt+click
    /// on a moment's button is its second meaning (`ACTION_ALT`: Deny → deny and say why).
    pub fn on_click(&mut self, x: f32, y: f32, shift: bool, alt: bool) {
        if self.click_switcher(x, y) || self.click_idle(x, y) {
            return;
        }
        // A click on the minimal bubble swaps that activity into the pill.
        if let Some((l, t, r, b)) = self.bubble_rect() {
            let (sx, sy) = (x + self.window.left as f32, y + self.window.top as f32);
            if sx >= l && sx < r && sy >= t && sy < b {
                if let Some(second) = self.snapshot.activities.get(1) {
                    self.bring_forward(second.id.clone());
                }
                return;
            }
        }
        let Some(front) = self.front() else { return };
        if self.state != State::Expanded {
            if front.expanded.is_some() {
                self.set_state(State::Expanded);
            } else {
                // No card to open here: the module may have its own window (the clipboard).
                _ = self.to_hub.send(ClientMsg::Action { id: front.id.clone(), action: "open".into() });
            }
            return;
        }
        let (cx, cy) = self.card_point(x, y);
        // A click on a card that asks something gives it the keyboard (PRD agent moments
        // §5.7): only now, never when it arrived by itself.
        if self.front_view().is_some_and(choices::wants_keys) {
            self.engaged = true;
            if let Some(i) = render::hit_at(&self.hits, cx, cy).and_then(|h| h.choice) {
                self.choice_cursor = Some(i);
            }
            self.update_capture();
        }
        if let Some(hit) = render::hit_at(&self.hits, cx, cy) {
            let card = hit.card.is_some();
            let action = match hit.card {
                Some(i) if shift => self.item_action(i, ItemKey::ShiftEnter),
                _ => None,
            };
            let mut action = action.unwrap_or_else(|| hit.action_at(cx));
            let moment = self.front_view().is_some_and(|v| v.context.is_some());
            if alt && moment && !hit.pager && hit.choice.is_none() && !action.is_empty() {
                action = format!("{ACTION_ALT}{action}");
            }
            // A moment's button may bring the agent's terminal forward (Open terminal,
            // Answer in terminal). Windows lets only the process that got the click hand
            // the foreground on, so the hub may set it now; the island itself stays
            // unfocusable.
            if moment && !action.is_empty() {
                unsafe { _ = AllowSetForegroundWindow(ASFW_ANY) };
            }
            // The switcher's middle ("1/2") is only a wheel target.
            if !action.is_empty() {
                self.act(action, card);
            }
        }
    }

    /// Mouse wheel at screen point `pt`. In an open card: over the source switcher it
    /// switches source, over a slider it moves it, and a row of cards scrolls (also
    /// sideways, for trackpads). A sideways swipe on the compact pill skips track. Otherwise
    /// it cycles through activities in the hub's order.
    pub fn on_wheel(&mut self, delta: i16, horizontal: bool, pt: POINT) {
        if !horizontal && self.wheel_switcher(delta as i32, pt) {
            return;
        }
        if self.state == State::Expanded && !horizontal {
            let (cx, cy) = self.card_point((pt.x - self.window.left) as f32, (pt.y - self.window.top) as f32);
            let hit = render::hit_at(&self.hits, cx, cy).map(|h| (h.pager, h.slider.is_some(), h.action.clone(), h.block));
            match hit {
                Some((true, ..)) => return self.wheel_pager(delta as i32),
                // A slider in a list that scrolls (the mixer) gives the wheel to the list.
                Some((_, true, action, false)) => return self.wheel_slider(delta as i32, action),
                Some((.., true)) => return self.wheel_block(delta as i32),
                _ => {}
            }
        }
        if horizontal && matches!(self.state, State::Compact | State::Peek) && !self.on_bubble {
            match self.swipe.feed(delta as i32, Instant::now()) {
                Some(true) => self.send_action(ACTION_SWIPE_NEXT.into()),
                Some(false) => self.send_action(ACTION_SWIPE_PREV.into()),
                None => {}
            }
            return;
        }
        if self.state == State::Expanded && self.panel.row().is_some() {
            // Tilting right (positive) moves on, like turning the wheel down.
            self.panel.wheel(if horizontal { -(delta as i32) } else { delta as i32 });
            self.update_scroll();
            return;
        }
        if horizontal {
            return;
        }
        let ids: Vec<ActivityId> = self.raw.activities.iter().map(|a| a.id.clone()).collect();
        let Some(current) = self.front().map(|a| a.id.clone()) else { return };
        if ids.len() < 2 {
            return;
        }
        let at = ids.iter().position(|id| *id == current).unwrap_or(0);
        let next = if delta < 0 { (at + 1) % ids.len() } else { (at + ids.len() - 1) % ids.len() };
        self.bring_forward(ids[next].clone());
    }

    /// Wheel over "‹ 1/2 ›": a notch down is the next source.
    fn wheel_pager(&mut self, delta: i32) {
        let Some(pager) = self.front_view().and_then(|v| v.pager.clone()) else { return };
        self.pager_travel += delta;
        while self.pager_travel.abs() >= music::WHEEL_NOTCH {
            let down = self.pager_travel < 0;
            self.pager_travel -= music::WHEEL_NOTCH * self.pager_travel.signum();
            self.send_action(if down { pager.next.clone() } else { pager.prev.clone() });
        }
    }

    /// Wheel over a text block: a few lines a notch (trackpads proportionally), drawn at
    /// once with no easing, so nothing keeps running afterwards.
    fn wheel_block(&mut self, delta: i32) {
        let step = component::block::WHEEL_LINES * typography::BLOCK_CODE.line_height;
        let scroll = (self.block_scroll - delta as f32 / music::WHEEL_NOTCH as f32 * step).clamp(0.0, self.block_max);
        if scroll != self.block_scroll {
            self.block_scroll = scroll;
            self.redraw_expanded();
        }
    }

    /// Wheel over a slider: 5 % a notch, shown at once.
    fn wheel_slider(&mut self, delta: i32, action: String) {
        let held = self.slider_hold.as_ref().filter(|(a, _)| *a == action).map(|(_, v)| *v);
        let Some(current) = held.or_else(|| self.slider_value(&action)) else { return };
        let value = music::wheel_value(current, delta);
        self.send_action(music::slider_action(&action, value));
        self.slider_hold = Some((action, value));
        self.redraw_expanded();
    }

    /// The module's value of the slider with `action` in the open card (a card may have
    /// several, one per display).
    fn slider_value(&self, action: &str) -> Option<f32> {
        self.front_view()?.rows.iter().find_map(|r| match r {
            Row::Slider { value, action: a, .. } if a == action => Some(*value),
            Row::Mixer(channels) => channels.iter().find(|c| c.action == action).map(|c| c.value),
            _ => None,
        })
    }

    /// A press in the open card at client point (`x`, `y`): on a seek bar or a slider it
    /// starts a drag (the caller captures the mouse). A plain click is a drag that ends
    /// where it began.
    pub fn on_press(&mut self, x: f32, y: f32) -> bool {
        if self.state != State::Expanded {
            return false;
        }
        let (cx, cy) = self.card_point(x, y);
        let Some(hit) = render::hit_at(&self.hits, cx, cy) else { return false };
        self.drag = match (hit.seek, hit.slider) {
            (Some((left, right, length)), _) => Some(Drag::Seek { left, right, length, ms: music::scrub_ms(cx, left, right, length) }),
            (_, Some((left, right))) => Some(Drag::Slider { left, right, action: hit.action.clone(), value: music::fraction_at(cx, left, right), throttle: Throttle::default() }),
            _ => None,
        };
        if self.drag.is_none() {
            return false;
        }
        self.send_drag();
        self.redraw_expanded();
        true
    }

    /// The pointer moved (screen point) during a drag.
    fn on_drag(&mut self, pt: POINT) {
        let (cx, _) = self.card_point((pt.x - self.window.left) as f32, (pt.y - self.window.top) as f32);
        match &mut self.drag {
            Some(Drag::Seek { left, right, length, ms }) => *ms = music::scrub_ms(cx, *left, *right, *length),
            Some(Drag::Slider { left, right, value, .. }) => *value = music::fraction_at(cx, *left, *right),
            None => return,
        }
        self.send_drag();
        let frame = std::time::Duration::from_secs_f32(1.0 / motion::scroll::FPS);
        if self.drag_drawn.is_none_or(|t| t.elapsed() >= frame) {
            self.drag_drawn = Some(Instant::now());
            self.redraw_expanded();
        }
    }

    /// A slider being dragged tells its module, at most 20 times a second.
    fn send_drag(&mut self) {
        let Some(Drag::Slider { action, value, throttle, .. }) = &mut self.drag else { return };
        if throttle.ready(Instant::now()) {
            let action = music::slider_action(action, *value);
            self.send_action(action);
        }
    }

    /// The button came up (client point): ends a drag, sending where it ended. Returns
    /// whether there was one (the caller then releases the mouse).
    pub fn on_release(&mut self, x: f32, y: f32) -> bool {
        if self.drag.is_none() {
            return false;
        }
        let (sx, sy) = (x as i32 + self.window.left, y as i32 + self.window.top);
        self.on_drag(POINT { x: sx, y: sy });
        self.finish_drag();
        true
    }

    /// The mouse capture went elsewhere mid-drag: keep what was set.
    pub fn on_capture_lost(&mut self) {
        self.finish_drag();
    }

    fn finish_drag(&mut self) {
        match self.drag.take() {
            Some(Drag::Seek { ms, .. }) => {
                self.send_action(format!("seek:{ms}"));
                self.seek_hold = Some(SeekHold { position_ms: ms, at_unix_ms: reui_protocol::unix_ms() });
                // Redraw when the hold runs out, in case nothing else does (paused).
                unsafe { SetTimer(Some(self.hwnd), TIMER_HOLD, music::SEEK_HOLD_MS as u32 + 20, None) };
            }
            Some(Drag::Slider { action, value, .. }) => {
                self.send_action(music::slider_action(&action, value));
                self.slider_hold = Some((action, value));
            }
            None => return,
        }
        self.redraw_expanded();
        self.update_tick();
    }

    /// The thumb of the bar under the pointer (screen point), redrawn only when it changes.
    fn update_thumb(&mut self, pt: POINT) {
        let want = if self.state == State::Expanded && self.shown {
            let (cx, cy) = self.card_point((pt.x - self.window.left) as f32, (pt.y - self.window.top) as f32);
            render::hit_at(&self.hits, cx, cy).and_then(|h| match (h.seek, h.slider) {
                (Some(_), _) => Some(Thumb::Seek),
                (_, Some(_)) => Some(Thumb::Slider),
                _ => None,
            })
        } else {
            None
        };
        if want != self.thumb {
            self.thumb = want;
            self.redraw_expanded();
        }
    }

    /// The card under the pointer (screen point) shows its pin and delete actions; redrawn
    /// only when that changes.
    fn update_hover_card(&mut self, pt: POINT, inside: bool) {
        let want = if inside && self.state == State::Expanded && self.shown && self.drag.is_none() {
            let (cx, cy) = self.card_point((pt.x - self.window.left) as f32, (pt.y - self.window.top) as f32);
            render::card_at(&self.hits, cx, cy)
        } else {
            None
        };
        if want != self.hover_card {
            self.hover_card = want;
            self.redraw_expanded();
        }
    }

    /// The context menu entries (label, action) of the card under client point (`x`, `y`),
    /// or `None` when it isn't over a card.
    /// Each entry says whether picking it closes the panel (see [`Self::act`]).
    pub fn card_menu(&self, x: f32, y: f32) -> Option<Vec<MenuEntry>> {
        if self.state != State::Expanded {
            return None;
        }
        let (cx, cy) = self.card_point(x, y);
        let i = render::card_at(&self.hits, cx, cy)?;
        let closes = |a: &reui_protocol::ItemAction| matches!(a.key, Some(ItemKey::ShiftEnter | ItemKey::CtrlEnter));
        Some(self.items().get(i)?.actions.iter().map(|a| (a.label.clone(), a.action.clone(), closes(a), false)).collect())
    }

    /// The choices of the menu button under client point (`x`, `y`) in the open card (a
    /// slider's output device), the current one checked; `None` when it isn't over one.
    pub fn click_menu(&self, x: f32, y: f32) -> Option<Vec<MenuEntry>> {
        if self.state != State::Expanded {
            return None;
        }
        let (cx, cy) = self.card_point(x, y);
        if !render::hit_at(&self.hits, cx, cy)?.menu {
            return None;
        }
        let menu = self.front_view()?.rows.iter().find_map(|r| if let Row::Slider { trailing: Some(menu), .. } = r { Some(menu) } else { None })?;
        Some(menu.options.iter().map(|o| (o.label.clone(), o.action.clone(), false, o.selected)).collect())
    }

    /// A card's context menu is up (keys go to it, clicks on it don't dismiss the panel)...
    pub fn menu_opened(&mut self) {
        self.menu_open = true;
        clipboard_keys::pause(true);
    }

    /// ...and gone, the foreground handed back to `previous`.
    pub fn menu_closed(&mut self, previous: HWND) {
        self.menu_open = false;
        clipboard_keys::pause(false);
        self.ignore_foreground = Some(previous);
    }

    /// Client pixels to expanded-surface DIPs.
    fn card_point(&self, x: f32, y: f32) -> (f32, f32) {
        let s = self.scale;
        let win_w = (self.window.right - self.window.left) as f32;
        ((x - (win_w - expanded_surface(&self.look).0 * s) / 2.0) / s, (y - self.look.top() * s) / s)
    }
}
