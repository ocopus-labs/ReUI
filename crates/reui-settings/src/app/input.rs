//! Pointer and keyboard. Everything works from the keyboard: Tab and Shift+Tab walk the
//! nav and every control, arrows switch pages in the nav, move segments and sliders and
//! walk menus, Space and Enter press, Esc backs out. Coordinates are window DIPs.

use reui_design::tokens::settings as s;
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, SC_MINIMIZE, WM_CLOSE, WM_SYSCOMMAND};
use windows::Win32::Foundation::{LPARAM, WPARAM};

use super::{App, Drag};
use crate::pages::Page;
use crate::render::menu;
use crate::ui::layout::{self, Rect};
use crate::ui::{Act, Focus, Hover, Part, Target};

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

/// Keyboard steps on a slider.
const SLIDER_STEP: f32 = 0.1;

impl App {
    /// Whether (`x`, `y`) drags the window: the caption band, except its buttons.
    pub fn is_caption(&self, x: f32, y: f32) -> bool {
        y < s::caption::HEIGHT && self.menu.is_none() && !layout::caption_buttons(self.dips().0).iter().any(|r| r.contains(x, y))
    }

    /// What's under the pointer.
    fn hit(&self, x: f32, y: f32) -> Hover {
        let (w, h) = self.dips();
        if let Some(menu) = &self.menu {
            return menu::item_at(menu, x, y).map_or(Hover::None, Hover::Menu);
        }
        if let Some(i) = layout::caption_buttons(w).iter().position(|r| r.contains(x, y)) {
            return Hover::Caption(i);
        }
        if let Some(i) = layout::nav_items(Page::ALL.len()).iter().position(|r| r.contains(x, y)) {
            return Hover::Nav(i);
        }
        if x >= w - s::scrollbar::HOVER_WIDTH - 2.0 * s::scrollbar::MARGIN && y >= s::caption::HEIGHT && layout::scrollbar(self.scroll, self.layout.height, h).is_some() {
            return Hover::Scrollbar;
        }
        if x < s::nav::WIDTH || y < s::caption::HEIGHT {
            return Hover::None;
        }
        let (px, py) = (x - layout::page_left(w), y + self.scroll);
        match self.layout.hit(px, py) {
            Some(placed) => {
                let cell = placed.cells.iter().position(|c| c.contains(px, py));
                Hover::Part(placed.target.clone(), cell)
            }
            None => Hover::None,
        }
    }

    /// The pointer moved or the page did under it.
    pub fn refresh_hover(&mut self) {
        let Some((x, y)) = crate::window::cursor(self.hwnd, self.scale()) else { return };
        let hover = self.hit(x, y);
        if hover != self.hover {
            self.hover = hover;
            if let (Some(menu), Hover::Menu(i)) = (self.menu.as_mut(), &self.hover) {
                menu.hot = Some(*i);
            }
            self.invalidate();
        }
    }

    pub fn mouse_move(&mut self, x: f32, y: f32) {
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
                self.scroll = layout::scroll_for_thumb(y - grab, self.layout.height, h);
                self.scroll_target = self.scroll;
                self.invalidate();
                return;
            }
            None => {}
        }
        let hover = self.hit(x, y);
        if hover != self.hover {
            if let (Some(menu), Hover::Menu(i)) = (self.menu.as_mut(), &hover) {
                menu.hot = Some(*i);
            }
            self.hover = hover;
            self.invalidate();
        }
    }

    /// The pointer is over a text field (it shows the I-beam).
    pub fn over_field(&self) -> bool {
        matches!(&self.hover, Hover::Part(t, _) if matches!(self.part(t), Some(Part::Field { .. })))
    }

    pub fn mouse_leave(&mut self) {
        if self.hover != Hover::None {
            self.hover = Hover::None;
            self.invalidate();
        }
    }

    /// Returns whether the pointer should be captured (a drag started).
    pub fn mouse_down(&mut self, x: f32, y: f32) -> bool {
        self.focus_visible = false;
        if let Some(m) = &self.menu {
            if !m.rect.contains(x, y) {
                self.menu = None;
                self.invalidate();
            }
            return false;
        }
        if self.state.recording {
            self.cancel_recording();
        }
        match self.hit(x, y) {
            Hover::Caption(i) => self.pressed_caption = Some(i),
            Hover::Nav(i) => {
                self.focus = Some(Focus::Nav);
                self.set_page(Page::ALL[i]);
            }
            Hover::Scrollbar => {
                let (_, h) = self.dips();
                if let Some((top, len)) = layout::scrollbar(self.scroll, self.layout.height, h) {
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
        if let Some(m) = &self.menu {
            if let Hover::Menu(i) = hit {
                let _ = m;
                self.choose(i);
            }
            return;
        }
        if let Some(i) = self.pressed_caption.take() {
            if hit == Hover::Caption(i) {
                unsafe {
                    if i == 0 {
                        _ = PostMessageW(Some(self.hwnd), WM_SYSCOMMAND, WPARAM(SC_MINIMIZE as usize), LPARAM(0));
                    } else {
                        _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                    }
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

    pub fn wheel(&mut self, delta: i16) {
        let notches = f32::from(delta) / 120.0;
        if let Some(menu) = self.menu.as_mut() {
            menu.scroll = (menu.scroll - notches * s::menu::ITEM_HEIGHT * 3.0).clamp(0.0, menu::max_scroll(menu));
            self.invalidate();
            return;
        }
        self.scroll_to(self.scroll_target - notches * s::scrollbar::WHEEL);
    }

    /// Returns whether the key was used.
    pub fn key(&mut self, vk: u32, shift: bool, ctrl: bool) -> bool {
        if self.menu.is_some() {
            return self.menu_key(vk);
        }
        if vk == VK_TAB && ctrl {
            let i = self.page.index() as isize + if shift { -1 } else { 1 };
            self.set_page(Page::ALL[i.rem_euclid(Page::ALL.len() as isize) as usize]);
            return true;
        }
        if vk == VK_TAB {
            self.move_focus(if shift { -1 } else { 1 });
            return true;
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
                let i = self.page.index() as isize + if vk == VK_UP { -1 } else { 1 };
                self.set_page(Page::ALL[i.clamp(0, Page::ALL.len() as isize - 1) as usize]);
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
                let page = self.dips().1 - s::caption::HEIGHT - s::page::GROUP_GAP;
                self.scroll_to(self.scroll_target + if vk == VK_PRIOR { -page } else { page });
            }
            (VK_HOME, ..) => self.scroll_to(0.0),
            (VK_END, ..) => self.scroll_to(f32::MAX),
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
        let Some(Focus::Part(target)) = &self.focus else { return };
        if !matches!(self.part(target), Some(Part::Field { .. })) || c.is_control() || self.state.field.chars().count() >= 64 {
            return;
        }
        self.state.field.push(c);
        self.rebuild();
    }

    /// Tab order: the nav, then every enabled control in reading order.
    fn move_focus(&mut self, step: isize) {
        let mut order: Vec<Focus> = vec![Focus::Nav];
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
                let rect = Rect { l: placed.rect.l, r: placed.rect.r, ..row_rect(&self.layout, t).unwrap_or(placed.rect) };
                let target = layout::scroll_to_show(self.scroll_target, rect, self.dips().1, layout::max_scroll(self.layout.height, self.dips().1));
                self.scroll_to(target);
            }
        }
        self.invalidate();
    }
}

/// The well of the row a control is in (scrolling shows the whole row, not just the control).
fn row_rect(l: &layout::Layout, target: &Target) -> Option<Rect> {
    let placed = l.part(target)?;
    l.shapes.iter().find_map(|s| match s {
        layout::Shape::Row { rect, .. } if rect.t <= placed.rect.t && rect.b >= placed.rect.b => Some(*rect),
        _ => None,
    })
}
