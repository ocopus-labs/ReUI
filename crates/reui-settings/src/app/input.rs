//! Pointer and keyboard. Everything works from the keyboard: Tab and Shift+Tab walk the
//! search field, the nav and every control; Ctrl+F searches; arrows switch pages in the
//! nav, move segments and sliders and walk menus; Space and Enter press; Alt+Left / Right
//! (and the mouse's back / forward buttons) go back and forward; Esc backs out.
//! Coordinates are window DIPs.

use reui_design::tokens::prefs as s;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, SC_MINIMIZE, WM_CLOSE, WM_SYSCOMMAND};

use super::{App, Drag};
use crate::render::menu;
use crate::ui::layout::{self, Rect};
use crate::ui::{chrome, Act, Focus, Hover, Part};

const VK_BACK: u32 = 0x08;
const VK_TAB: u32 = 0x09;
const VK_RETURN: u32 = 0x0D;
const VK_ESCAPE: u32 = 0x1B;
const VK_SPACE: u32 = 0x20;
const VK_PRIOR: u32 = 0x21;
const VK_NEXT: u32 = 0x22;
const VK_END: u32 = 0x23;
const VK_HOME: u32 = 0x24;
const VK_LEFT: u32 = 0x25;
const VK_UP: u32 = 0x26;
const VK_RIGHT: u32 = 0x27;
const VK_DOWN: u32 = 0x28;
const VK_F: u32 = 0x46;
const VK_BROWSER_BACK: u32 = 0xA6;
const VK_BROWSER_FORWARD: u32 = 0xA7;

/// Keyboard steps on a slider.
const SLIDER_STEP: f32 = 0.1;

impl App {
    /// Whether (`x`, `y`) drags the window: the band across the top, except its buttons and
    /// the search field.
    pub fn is_caption(&self, x: f32, y: f32) -> bool {
        y < s::toolbar::HEIGHT && self.menu.is_none() && self.hit(x, y) == Hover::None
    }

    /// What's under the pointer.
    fn hit(&self, x: f32, y: f32) -> Hover {
        let (w, h) = self.dips();
        if let Some(menu) = &self.menu {
            return menu::item_at(menu, x, y).map_or(Hover::None, Hover::Menu);
        }
        if let Some(i) = chrome::caption_buttons(w).iter().position(|r| r.contains(x, y)) {
            return Hover::Caption(i);
        }
        if let Some(i) = chrome::history_buttons().iter().position(|r| r.contains(x, y)) {
            return Hover::History(i);
        }
        if !self.page.is_welcome() && chrome::search().contains(x, y) {
            return Hover::Search;
        }
        if !self.page.is_welcome() && chrome::account().contains(x, y) {
            return Hover::Account;
        }
        let sections: Vec<usize> = self.nav.iter().map(|p| p.section()).collect();
        if let Some(i) = chrome::nav_items(&sections).iter().position(|r| r.contains(x, y)) {
            return Hover::Nav(i);
        }
        if x >= w - s::scrollbar::HOVER_WIDTH - 2.0 * s::scrollbar::MARGIN && y >= s::toolbar::HEIGHT && chrome::scrollbar(self.motion.scroll, self.layout.height, h).is_some() {
            return Hover::Scrollbar;
        }
        if x < s::sidebar::WIDTH || y < s::toolbar::HEIGHT {
            return Hover::None;
        }
        let (px, py) = (x - chrome::page_left(w), y + self.motion.scroll);
        match self.layout.hit(px, py) {
            Some(placed) => {
                let segmented = matches!(self.part(&placed.target), Some(Part::Segmented { .. }));
                let cell = segmented.then(|| placed.cells.iter().position(|c| c.contains(px, py))).flatten();
                Hover::Part(placed.target.clone(), cell)
            }
            None => Hover::None,
        }
    }

    fn set_hover(&mut self, hover: Hover) {
        if hover != self.hover {
            if let (Some(menu), Hover::Menu(i)) = (self.menu.as_mut(), &hover) {
                menu.hot = Some(*i);
            }
            self.hover = hover;
            self.invalidate();
        }
    }

    /// The page moved under the pointer.
    pub fn refresh_hover(&mut self) {
        let hover = self.pointer.map_or(Hover::None, |(x, y)| self.hit(x, y));
        self.set_hover(hover);
    }

    pub fn mouse_move(&mut self, x: f32, y: f32) {
        self.pointer = Some((x, y));
        match &self.drag {
            Some(Drag::Slider(target)) => {
                let target = target.clone();
                if let Some(placed) = self.layout.part(&target) {
                    let value = layout::slider_value(self.to_window(placed.rect), x);
                    self.act(&target, Act::Value(value));
                }
                return;
            }
            Some(Drag::Scrollbar(grab)) => {
                let (_, h) = self.dips();
                self.motion.jump(chrome::scroll_for_thumb(y - grab, self.layout.height, h));
                self.scrolled();
                return;
            }
            None => {}
        }
        self.set_hover(self.hit(x, y));
    }

    /// The pointer is over a text field (it shows the I-beam).
    pub fn over_field(&self) -> bool {
        self.hover == Hover::Search || matches!(&self.hover, Hover::Part(t, _) if matches!(self.part(t), Some(Part::Field { .. })))
    }

    pub fn mouse_leave(&mut self) {
        self.pointer = None;
        self.set_hover(Hover::None);
    }

    /// Returns whether the pointer should be captured (a drag started).
    pub fn mouse_down(&mut self, x: f32, y: f32) -> bool {
        self.pointer = Some((x, y));
        self.focus_visible = false;
        if let Some(m) = &self.menu {
            if !m.rect.contains(x, y) {
                self.menu = None;
                self.invalidate();
            }
            return false;
        }
        if self.state.recording.is_some() {
            self.cancel_recording();
        }
        let hit = self.hit(x, y);
        match hit.clone() {
            Hover::Caption(_) | Hover::History(_) | Hover::Account => self.pressed_chrome = Some(hit),
            Hover::Search => {
                self.focus = Some(Focus::Search);
                self.invalidate();
            }
            Hover::Nav(i) => {
                self.focus = Some(Focus::Nav);
                self.set_page(self.nav[i]);
            }
            Hover::Scrollbar => {
                let (_, h) = self.dips();
                if let Some((top, len)) = chrome::scrollbar(self.motion.scroll, self.layout.height, h) {
                    let grab = if (top..top + len).contains(&y) { y - top } else { len / 2.0 };
                    self.drag = Some(Drag::Scrollbar(grab));
                    self.mouse_move(x, y);
                    return true;
                }
            }
            Hover::Part(target, cell) => {
                self.focus = Some(Focus::Part(target.clone()));
                match self.part(&target).cloned() {
                    Some(Part::Slider { .. }) => {
                        self.drag = Some(Drag::Slider(target));
                        self.mouse_move(x, y);
                        return true;
                    }
                    Some(Part::Segmented { .. }) => {
                        if let Some(i) = cell {
                            self.act(&target, Act::Choose(i));
                        }
                    }
                    Some(Part::Field { .. }) => {}
                    Some(_) => self.pressed = Some(target),
                    None => {}
                }
                self.invalidate();
            }
            Hover::None | Hover::Menu(_) => {
                self.focus = None;
                self.invalidate();
            }
        }
        false
    }

    pub fn mouse_up(&mut self, x: f32, y: f32) {
        if self.drag.take().is_some() {
            self.invalidate();
            return;
        }
        let hit = self.hit(x, y);
        if self.menu.is_some() {
            if let Hover::Menu(i) = hit {
                self.choose(i);
            }
            return;
        }
        if let Some(pressed) = self.pressed_chrome.take() {
            if hit == pressed {
                match pressed {
                    Hover::Caption(0) => unsafe { _ = PostMessageW(Some(self.hwnd), WM_SYSCOMMAND, WPARAM(SC_MINIMIZE as usize), LPARAM(0)) },
                    Hover::Caption(_) => unsafe { _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) },
                    Hover::History(i) => self.travel(if i == 0 { -1 } else { 1 }),
                    Hover::Account if !self.page.is_welcome() => self.set_page(crate::pages::Page::About),
                    _ => {}
                }
            }
            return;
        }
        if let Some(target) = self.pressed.take() {
            if matches!(&hit, Hover::Part(t, _) if *t == target) {
                self.act(&target, Act::Press);
            }
            self.invalidate();
        }
    }

    /// The mouse's back (1) / forward (2) button.
    pub fn x_button(&mut self, button: u16) {
        self.travel(if button == 1 { -1 } else { 1 });
    }

    /// A wheel or touchpad turned by `delta` (120 a notch). Notches ease to their target;
    /// a touchpad's small deltas arrive smooth already, so they move the page directly.
    pub fn wheel(&mut self, delta: i16) {
        let notches = f32::from(delta) / 120.0;
        if let Some(menu) = self.menu.as_mut() {
            menu.scroll = (menu.scroll - notches * s::menu::ITEM_HEIGHT * 3.0).clamp(0.0, menu::max_scroll(menu));
            self.invalidate();
            return;
        }
        let distance = notches * s::scrollbar::WHEEL;
        if delta % 120 != 0 {
            let max = chrome::max_scroll(self.layout.height, self.dips().1);
            let to = (self.motion.scroll_target - distance).clamp(0.0, max);
            self.motion.jump(to);
            self.refresh_hover();
            self.scrolled();
        } else {
            self.scroll_to(self.motion.scroll_target - distance);
        }
    }

    /// A key the system sends as a system key (with Alt): back / forward.
    pub fn sys_key(&mut self, vk: u32) -> bool {
        match vk {
            VK_LEFT => self.travel(-1),
            VK_RIGHT => self.travel(1),
            _ => return false,
        }
        true
    }

    /// Returns whether the key was used.
    pub fn key(&mut self, vk: u32, shift: bool, ctrl: bool) -> bool {
        if self.menu.is_some() {
            return self.menu_key(vk);
        }
        match vk {
            VK_BROWSER_BACK => {
                self.travel(-1);
                return true;
            }
            VK_BROWSER_FORWARD => {
                self.travel(1);
                return true;
            }
            VK_F if ctrl && !self.page.is_welcome() => {
                self.focus = Some(Focus::Search);
                self.invalidate();
                return true;
            }
            VK_TAB if ctrl => {
                let nav = self.page.nav();
                let i = self.page.index() as isize + if shift { -1 } else { 1 };
                self.set_page(nav[i.rem_euclid(nav.len() as isize) as usize]);
                return true;
            }
            VK_TAB => {
                self.move_focus(if shift { -1 } else { 1 });
                return true;
            }
            _ => {}
        }
        if self.focus == Some(Focus::Search) {
            return self.search_key(vk);
        }
        let part = match &self.focus {
            Some(Focus::Part(t)) => self.part(t).cloned().map(|p| (t.clone(), p)),
            _ => None,
        };
        // Typing in the field.
        if let Some((target, Part::Field { .. })) = &part {
            match vk {
                VK_BACK => {
                    self.state.field.pop();
                    self.rebuild();
                    return true;
                }
                VK_RETURN => {
                    self.act(target, Act::Press);
                    return true;
                }
                VK_ESCAPE => {
                    self.focus = None;
                    self.invalidate();
                    return true;
                }
                VK_SPACE => return false, // typed by `char`
                _ => {}
            }
        }
        match (vk, &self.focus, &part) {
            (VK_ESCAPE, ..) => {
                if self.state.armed.take().is_some() {
                    self.rebuild();
                } else {
                    self.focus = None;
                    self.invalidate();
                }
            }
            (VK_UP | VK_DOWN, Some(Focus::Nav), _) => {
                self.focus_visible = true;
                if self.nav.is_empty() {
                    return true;
                }
                let at = self.nav.iter().position(|p| *p == self.page);
                let i = match at {
                    Some(i) => (i as isize + if vk == VK_UP { -1 } else { 1 }).clamp(0, self.nav.len() as isize - 1) as usize,
                    None => 0,
                };
                self.set_page(self.nav[i]);
            }
            (VK_LEFT | VK_RIGHT, _, Some((target, Part::Segmented { options, selected }))) => {
                let next = if vk == VK_LEFT { selected.saturating_sub(1) } else { (selected + 1).min(options.len() - 1) };
                self.focus_visible = true;
                if next != *selected {
                    self.act(target, Act::Choose(next));
                }
            }
            (VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN, _, Some((target, Part::Slider { value, .. }))) => {
                let step = if matches!(vk, VK_LEFT | VK_DOWN) { -SLIDER_STEP } else { SLIDER_STEP };
                self.focus_visible = true;
                self.act(target, Act::Value((value + step).clamp(0.0, 1.0)));
            }
            (VK_UP | VK_DOWN, _, Some((target, Part::Dropdown { .. }))) => {
                self.focus_visible = true;
                self.act(target, Act::Press);
            }
            (VK_UP | VK_DOWN, Some(Focus::Part(_)), _) => self.move_focus(if vk == VK_UP { -1 } else { 1 }),
            (VK_SPACE | VK_RETURN, _, Some((target, _))) => {
                self.focus_visible = true;
                self.act(target, Act::Press);
            }
            (VK_PRIOR | VK_NEXT, ..) => {
                let page = self.dips().1 - s::toolbar::HEIGHT - s::page::GROUP_GAP;
                self.scroll_to(self.motion.scroll_target + if vk == VK_PRIOR { -page } else { page });
            }
            (VK_HOME, ..) => self.scroll_to(0.0),
            (VK_END, ..) => self.scroll_to(f32::MAX),
            _ => return false,
        }
        true
    }

    /// Keys while the search field has the keyboard.
    fn search_key(&mut self, vk: u32) -> bool {
        match vk {
            VK_BACK => {
                let mut query = self.query.clone();
                query.pop();
                self.set_query(query);
            }
            VK_ESCAPE if !self.query.is_empty() => self.set_query(String::new()),
            VK_ESCAPE => {
                self.focus = None;
                self.invalidate();
            }
            VK_RETURN => {
                if let Some(&first) = self.nav.first() {
                    self.set_page(first);
                }
            }
            VK_DOWN => {
                self.focus = Some(Focus::Nav);
                self.focus_visible = true;
                if let Some(&first) = self.nav.first().filter(|_| !self.nav.contains(&self.page)) {
                    self.set_page(first);
                }
                self.invalidate();
            }
            VK_SPACE => return false, // typed by `char`
            _ => return false,
        }
        true
    }

    fn menu_key(&mut self, vk: u32) -> bool {
        let Some(menu) = self.menu.as_mut() else { return false };
        let count = menu.items.len();
        match vk {
            VK_UP | VK_DOWN if count > 0 => {
                let hot = match (menu.hot, vk) {
                    (None, _) => 0,
                    (Some(i), VK_UP) => i.saturating_sub(1),
                    (Some(i), _) => (i + 1).min(count - 1),
                };
                menu.hot = Some(hot);
                // Keep it in view.
                let visible = menu.rect.h() - 2.0 * s::menu::PADDING;
                let top = hot as f32 * s::menu::ITEM_HEIGHT;
                if top < menu.scroll {
                    menu.scroll = top;
                } else if top + s::menu::ITEM_HEIGHT > menu.scroll + visible {
                    menu.scroll = top + s::menu::ITEM_HEIGHT - visible;
                }
            }
            VK_RETURN | VK_SPACE => {
                if let Some(i) = menu.hot {
                    self.choose(i);
                    return true;
                }
            }
            VK_ESCAPE | VK_TAB => self.menu = None,
            _ => return false,
        }
        self.invalidate();
        true
    }

    pub fn char(&mut self, c: char) {
        if c.is_control() {
            return;
        }
        match &self.focus {
            Some(Focus::Search) if self.query.chars().count() < 40 => {
                let mut query = self.query.clone();
                query.push(c);
                self.set_query(query);
            }
            Some(Focus::Part(target)) if matches!(self.part(target), Some(Part::Field { .. })) && self.state.field.chars().count() < 64 => {
                self.state.field.push(c);
                self.rebuild();
            }
            _ => {}
        }
    }

    /// Tab order: the search field, the nav, then every enabled control in reading order.
    fn move_focus(&mut self, step: isize) {
        let mut order: Vec<Focus> = Vec::new();
        if !self.page.is_welcome() {
            order.push(Focus::Search);
        }
        order.push(Focus::Nav);
        order.extend(self.layout.focus_order().into_iter().map(Focus::Part));
        let at = self.focus.as_ref().and_then(|f| order.iter().position(|o| o == f));
        let next = match at {
            Some(i) => (i as isize + step).rem_euclid(order.len() as isize) as usize,
            None if step > 0 => 0,
            None => order.len() - 1,
        };
        self.focus = Some(order[next].clone());
        self.focus_visible = true;
        if let Some(Focus::Part(t)) = &self.focus {
            if let Some(placed) = self.layout.part(t) {
                let rect = Rect { l: placed.rect.l, r: placed.rect.r, ..self.layout.row_of(t).unwrap_or(placed.rect) };
                let h = self.dips().1;
                let target = chrome::scroll_to_show(self.motion.scroll_target, rect, h, chrome::max_scroll(self.layout.height, h));
                self.scroll_to(target);
            }
        }
        self.invalidate();
    }
}
