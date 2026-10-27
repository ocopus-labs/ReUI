//! The window's state machine: which page, scroll, hover, focus, the open menu, drags and
//! the short animations; it rebuilds the page from the [`State`] after every change and
//! draws on demand. Input handling is in [`input`].

mod input;

use std::collections::HashMap;
use std::time::Instant;

use reui_design::tokens::settings as s;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::InvalidateRect;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, PostMessageW, SetTimer, WM_CLOSE};

use crate::keys;
use crate::pages::{self, Effect, Page};
use crate::render::{self, Renderer, View};
use crate::state::State;
use crate::ui::layout::{self, Layout, Measure};
use crate::ui::{Act, Block, Focus, Hover, Menu, Part, Role, Target};
use crate::updates::{self, Phase};

pub const TIMER_ANIM: usize = 1;
pub const TIMER_STATS: usize = 2;
pub const TIMER_ARM: usize = 3;
pub const TIMER_CLOCK: usize = 4;
/// A destructive button waits this long for its confirming press.
const ARM_MS: u32 = 4000;
const CLOCK_MS: u32 = 30_000;

enum Drag {
    Slider(Target),
    /// The scrollbar thumb, grabbed this far below its top.
    Scrollbar(f32),
}

pub struct App {
    pub hwnd: HWND,
    pub state: State,
    page: Page,
    blocks: Vec<Block>,
    layout: Layout,
    renderer: Renderer,
    dpi: u32,
    /// Client size in pixels.
    size: (u32, u32),
    scroll: f32,
    scroll_target: f32,
    hover: Hover,
    focus: Option<Focus>,
    /// The keyboard is in use: the focus ring shows.
    focus_visible: bool,
    pressed: Option<Target>,
    /// A caption button held down.
    pressed_caption: Option<usize>,
    menu: Option<Menu>,
    drag: Option<Drag>,
    /// Toggles sliding: row id → (from, to, start).
    knobs: HashMap<String, (f32, f32, Instant)>,
    last_frame: Instant,
    reduced_motion: bool,
    /// The welcome is open and not finished yet: closing the window finishes it (P-1).
    welcoming: bool,
    /// The welcome was replayed from About: Done goes back to Settings, not away.
    replaying: bool,
}

impl App {
    pub fn new(hwnd: HWND, state: State, page: Page, dpi: u32, reduced_motion: bool) -> windows::core::Result<Self> {
        let renderer = Renderer::new(dpi as f32)?;
        let mut app = Self {
            hwnd,
            state,
            page,
            blocks: Vec::new(),
            layout: Layout::default(),
            renderer,
            dpi,
            size: (0, 0),
            scroll: 0.0,
            scroll_target: 0.0,
            hover: Hover::None,
            focus: None,
            focus_visible: false,
            pressed: None,
            pressed_caption: None,
            menu: None,
            drag: None,
            knobs: HashMap::new(),
            last_frame: Instant::now(),
            reduced_motion,
            welcoming: false,
            replaying: false,
        };
        app.set_page(page);
        Ok(app)
    }

    fn scale(&self) -> f32 {
        self.dpi as f32 / 96.0
    }

    /// Client size in DIPs.
    pub fn dips(&self) -> (f32, f32) {
        (self.size.0 as f32 / self.scale(), self.size.1 as f32 / self.scale())
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.size = (width, height);
        self.renderer.resize(width, height);
        self.menu = None;
        self.rebuild();
    }

    pub fn set_dpi(&mut self, dpi: u32) {
        self.dpi = dpi;
        self.renderer.set_dpi(dpi as f32);
        self.rebuild();
    }

    pub fn invalidate(&self) {
        unsafe { _ = InvalidateRect(Some(self.hwnd), None, false) };
    }

    /// Builds the page again from the state and lays it out.
    pub fn rebuild(&mut self) {
        self.blocks = pages::build(self.page, &self.state);
        let (w, h) = self.dips();
        self.layout = layout::layout(&self.blocks, layout::column_width(w), &self.renderer);
        let max = layout::max_scroll(self.layout.height, h);
        self.scroll = self.scroll.clamp(0.0, max);
        self.scroll_target = self.scroll_target.clamp(0.0, max);
        // A control that went away (an app removed) loses the focus.
        if let Some(Focus::Part(t)) = &self.focus {
            if self.layout.part(t).is_none_or(|p| !p.enabled) {
                self.focus = None;
            }
        }
        self.invalidate();
    }

    pub fn set_page(&mut self, page: Page) {
        let timer = |id, ms: Option<u32>| unsafe {
            match ms {
                Some(ms) => _ = SetTimer(Some(self.hwnd), id, ms, None),
                None => _ = KillTimer(Some(self.hwnd), id),
            }
        };
        if page.is_welcome() && !self.welcoming {
            self.welcoming = true;
            self.replaying = !self.blocks.is_empty() && !self.page.is_welcome();
            pages::start_welcome(&mut self.state);
        }
        self.page = page;
        self.scroll = 0.0;
        self.scroll_target = 0.0;
        self.menu = None;
        self.pressed = None;
        self.state.armed = None;
        if !matches!(self.focus, Some(Focus::Nav)) {
            self.focus = None;
        }
        if page == Page::About {
            self.state.stats.reset();
            self.state.stats.sample();
        }
        if page == Page::About {
            self.state.whats_new_seen();
        }
        if page == Page::Privacy {
            self.state.refresh_sizes();
        }
        timer(TIMER_STATS, (page == Page::About).then_some(s::motion::STATS));
        // Times that move on their own: the pause's, the agents' last events.
        timer(TIMER_CLOCK, matches!(page, Page::General | Page::Agents).then_some(CLOCK_MS));
        self.rebuild();
    }

    /// The window came to the front: files may have been edited meanwhile (P-4, P-7).
    pub fn activated(&mut self, active: bool) {
        if active {
            self.state.refresh();
            self.rebuild();
        } else {
            self.cancel_recording();
            self.menu = None;
            self.invalidate();
        }
    }

    pub fn paint(&mut self) {
        let (width, height) = self.dips();
        let now = Instant::now();
        let knobs: HashMap<String, f32> = self.knobs.iter().map(|(id, &(from, to, start))| (id.clone(), knob_at(from, to, start, now))).collect();
        let titles: Vec<&'static str> = self.page.nav().iter().map(|p| p.title()).collect();
        let typing = match (&self.focus, &self.menu) {
            (Some(Focus::Part(t)), None) if matches!(self.part(t), Some(Part::Field { .. })) => Some(t.clone()),
            _ => None,
        };
        let view = View {
            width,
            height,
            nav: &titles,
            page: self.page.index(),
            blocks: &self.blocks,
            layout: &self.layout,
            scroll: self.scroll,
            hover: &self.hover,
            focus: self.focus.as_ref().filter(|_| self.focus_visible),
            pressed: self.pressed.as_ref(),
            knobs: &knobs,
            menu: self.menu.as_ref(),
            typing: typing.as_ref(),
            scrollbar_active: matches!(self.drag, Some(Drag::Scrollbar(_))),
        };
        let (hwnd, size) = (self.hwnd, self.size);
        if let Err(e) = self.renderer.draw(hwnd, size.0, size.1, |p| render::frame::draw(p, &view)) {
            eprintln!("settings: draw failed: {e}");
        }
    }

    pub fn on_timer(&mut self, id: usize) {
        match id {
            TIMER_ANIM => self.tick(),
            TIMER_STATS => {
                self.state.stats.sample();
                self.rebuild();
            }
            TIMER_ARM => {
                unsafe { _ = KillTimer(Some(self.hwnd), TIMER_ARM) };
                self.state.armed = None;
                self.rebuild();
            }
            TIMER_CLOCK => {
                self.state.refresh();
                self.rebuild();
            }
            _ => {}
        }
    }

    /// One animation frame: knobs slide, the scroll eases to its target.
    fn tick(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32() * 1000.0;
        self.last_frame = now;
        self.knobs.retain(|_, &mut (_, _, start)| now.duration_since(start).as_millis() < u128::from(s::motion::TOGGLE));
        let gap = self.scroll_target - self.scroll;
        if gap.abs() < 0.5 {
            self.scroll = self.scroll_target;
        } else {
            self.scroll += gap * (1.0 - (-dt / s::motion::SCROLL as f32).exp());
        }
        if self.knobs.is_empty() && self.scroll == self.scroll_target {
            unsafe { _ = KillTimer(Some(self.hwnd), TIMER_ANIM) };
        }
        self.refresh_hover();
        self.invalidate();
    }

    fn animate(&mut self) {
        if self.reduced_motion {
            self.knobs.clear();
            self.scroll = self.scroll_target;
            self.invalidate();
            return;
        }
        self.last_frame = Instant::now();
        unsafe { _ = SetTimer(Some(self.hwnd), TIMER_ANIM, 1000 / s::motion::FPS as u32, None) };
    }

    fn scroll_to(&mut self, target: f32) {
        let max = layout::max_scroll(self.layout.height, self.dips().1);
        self.scroll_target = target.clamp(0.0, max);
        if self.scroll_target != self.scroll {
            self.animate();
        }
    }

    fn part(&self, target: &Target) -> Option<&Part> {
        render::part_of(&self.blocks, target)
    }

    /// Runs a control's action and whatever it asks of the window.
    fn act(&mut self, target: &Target, act: Act) {
        let Some(part) = self.part(target).cloned() else { return };
        // A destructive button asks twice.
        if let Part::Button { role: Role::Destructive, armed: false, .. } = part {
            self.state.armed = Some(target.clone());
            unsafe { _ = SetTimer(Some(self.hwnd), TIMER_ARM, ARM_MS, None) };
            self.rebuild();
            return;
        }
        if self.state.armed.is_some() {
            self.state.armed = None;
            unsafe { _ = KillTimer(Some(self.hwnd), TIMER_ARM) };
        }
        if let (Part::Dropdown { .. }, Act::Press) = (&part, &act) {
            self.open_menu(target);
            return;
        }
        if let (Part::Toggle { on }, Act::Press) = (&part, &act) {
            let (from, to) = if *on { (1.0, 0.0) } else { (0.0, 1.0) };
            self.knobs.insert(target.row.clone(), (from, to, Instant::now()));
            self.animate();
        }
        match pages::act(self.page, &mut self.state, target, act) {
            Effect::None => {}
            Effect::Navigate(page) => {
                self.set_page(page);
                return;
            }
            Effect::Record => {
                self.state.recording = true;
                self.state.hotkey_note = None;
                keys::start(self.hwnd);
            }
            Effect::Finish { open_settings } => {
                self.finish_welcome();
                if open_settings || std::mem::take(&mut self.replaying) {
                    self.set_page(Page::General);
                } else {
                    unsafe { _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) };
                }
                return;
            }
            Effect::CheckUpdates => self.state.updates.check(&self.state.paths, self.hwnd),
            Effect::Download => self.state.updates.download(self.hwnd),
        }
        self.rebuild();
    }

    fn finish_welcome(&mut self) {
        if std::mem::take(&mut self.welcoming) {
            pages::finish_welcome(&self.state);
        }
    }

    /// The window is going away: an open welcome counts as done.
    pub fn closing(&mut self) {
        self.finish_welcome();
    }

    /// The update worker moved on.
    pub fn on_updates(&mut self) {
        if let Phase::Ready(path) = self.state.updates.phase() {
            if self.state.paths.sandbox {
                // Tests never install anything.
                self.state.updates.reset(&self.state.paths);
                self.state.write_error = Some(format!("Downloaded and verified {}; the sandbox doesn't install it", path.display()));
            } else {
                match updates::run_installer(&path) {
                    // The installer stops ReUI and this window, then starts ReUI again.
                    Ok(()) => unsafe { _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) },
                    Err(e) => {
                        self.state.updates.reset(&self.state.paths);
                        self.state.write_error = Some(e);
                    }
                }
            }
        }
        if self.page == Page::About {
            self.rebuild();
        }
    }

    fn open_menu(&mut self, target: &Target) {
        let Some((items, checked)) = pages::menu(self.page, &mut self.state, target) else { return };
        let Some(placed) = self.layout.part(target) else { return };
        let anchor = self.to_window(placed.rect);
        let widest = items.iter().map(|i| self.renderer.width(i, layout::Font::Nav)).fold(0.0, f32::max);
        let rect = render::menu::place(anchor, items.len().max(1), widest, self.dips());
        let mut menu = Menu { target: target.clone(), items, checked, rect, hot: checked, scroll: 0.0 };
        if let Some(i) = checked {
            // The checked entry in view.
            let visible = rect.h() - 2.0 * s::menu::PADDING;
            menu.scroll = (i as f32 * s::menu::ITEM_HEIGHT - visible / 2.0).clamp(0.0, render::menu::max_scroll(&menu));
        }
        self.menu = Some(menu);
        self.invalidate();
    }

    fn choose(&mut self, i: usize) {
        if let Some(menu) = self.menu.take() {
            self.act(&menu.target, Act::Choose(i));
        }
    }

    /// The hotkey recorder heard a chord (`vk` 0: Esc).
    pub fn on_chord(&mut self, mods: u32, vk: u32) {
        if !self.state.recording {
            return;
        }
        if vk == 0 {
            self.state.recording = false;
        } else {
            pages::clipboard_chord(&mut self.state, mods, vk);
        }
        self.rebuild();
    }

    pub fn cancel_recording(&mut self) {
        keys::stop();
        if self.state.recording {
            self.state.recording = false;
            self.rebuild();
        }
    }

    /// Page rect → window rect.
    fn to_window(&self, r: layout::Rect) -> layout::Rect {
        r.offset(layout::page_left(self.dips().0), -self.scroll)
    }
}

/// A knob's position `from` → `to`, eased out.
fn knob_at(from: f32, to: f32, start: Instant, now: Instant) -> f32 {
    let t = (now.duration_since(start).as_secs_f32() * 1000.0 / s::motion::TOGGLE as f32).clamp(0.0, 1.0);
    let eased = 1.0 - (1.0 - t).powi(3);
    from + (to - from) * eased
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn knobs_ease_out_to_their_end() {
        let start = Instant::now();
        assert_eq!(knob_at(0.0, 1.0, start, start), 0.0);
        let mid = knob_at(0.0, 1.0, start, start + Duration::from_millis(u64::from(s::motion::TOGGLE) / 2));
        assert!(mid > 0.5 && mid < 1.0, "past halfway at half time: {mid}");
        assert_eq!(knob_at(1.0, 0.0, start, start + Duration::from_secs(1)), 0.0);
    }
}
