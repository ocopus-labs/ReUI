//! The window's state machine: which page, the sidebar's search, history (back, forward),
//! hover, focus, the open menu, drags; it rebuilds the page from the [`State`] when
//! content changes (never for hover or focus) and draws on demand.
//!
//! Frames: a change marks the window dirty; the message loop draws when the swap chain can
//! take a frame ([`App::needs_frame`], [`App::frame`]). While something moves
//! ([`motion`]), that's once per display refresh; at rest, nothing is drawn and no timer
//! runs. Input handling is in [`input`].

mod input;
mod motion;
mod search;
mod shot;

use std::time::Instant;

use reui_design::tokens::{prefs, settings as s};
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, PostMessageW, SetTimer, WM_CLOSE};

use self::motion::Motion;
use crate::keys;
use crate::pages::{self, Effect, Page};
use crate::render::frame::NavItem;
use crate::render::{self, Renderer, View};
use crate::state::State;
use crate::ui::layout::{self, Layout, Measure};
use crate::ui::{chrome, Act, Block, Focus, Hover, Menu, Part, Role, Target};
use crate::updates::{self, Phase};

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
    motion: Motion,
    /// Something changed that the screen doesn't show yet.
    dirty: bool,
    /// The page drawn last frame is still right (only its scroll may have changed).
    page_fresh: bool,
    /// The pointer in window DIPs, while it's over the window.
    pointer: Option<(f32, f32)>,
    hover: Hover,
    focus: Option<Focus>,
    /// The keyboard is in use: the focus ring shows.
    focus_visible: bool,
    pressed: Option<Target>,
    /// A caption or toolbar button held down.
    pressed_chrome: Option<Hover>,
    menu: Option<Menu>,
    drag: Option<Drag>,
    /// The sidebar's search, and the pages it leaves in the nav.
    query: String,
    nav: Vec<Page>,
    /// Pages visited, and where in them we are (back / forward).
    history: Vec<Page>,
    history_at: usize,
    active: bool,
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
            motion: Motion::new(reduced_motion),
            dirty: true,
            page_fresh: false,
            pointer: None,
            hover: Hover::None,
            focus: None,
            focus_visible: false,
            pressed: None,
            pressed_chrome: None,
            menu: None,
            drag: None,
            query: String::new(),
            nav: page.nav().to_vec(),
            history: vec![page],
            history_at: 0,
            active: true,
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
        if self.size == (width, height) {
            return;
        }
        self.size = (width, height);
        self.renderer.resize(width, height);
        self.menu = None;
        self.relayout();
    }

    pub fn set_dpi(&mut self, dpi: u32) {
        self.dpi = dpi;
        self.renderer.set_dpi(dpi as f32);
        self.relayout();
    }

    /// Marks the window for drawing on the next frame.
    pub fn invalidate(&mut self) {
        self.dirty = true;
        self.page_fresh = false;
    }

    /// Only the scroll changed: the next frame moves the page it drew last.
    fn scrolled(&mut self) {
        self.dirty = true;
    }

    /// Builds the page again from the state, then lays it out.
    pub fn rebuild(&mut self) {
        self.blocks = pages::build(self.page, &self.state);
        self.relayout();
    }

    /// Lays the page out again (content or size changed; hover and focus never do this).
    fn relayout(&mut self) {
        crate::perf::layout();
        let (w, h) = self.dips();
        self.layout = layout::layout(&self.blocks, chrome::column_width(w), &self.renderer);
        let max = chrome::max_scroll(self.layout.height, h);
        self.motion.scroll = self.motion.scroll.clamp(0.0, max);
        self.motion.scroll_target = self.motion.scroll_target.clamp(0.0, max);
        // A control that went away (an app removed) loses the focus.
        if let Some(Focus::Part(t)) = &self.focus {
            if self.layout.part(t).is_none_or(|p| !p.enabled) {
                self.focus = None;
            }
        }
        self.refresh_hover();
        self.invalidate();
    }

    pub fn set_page(&mut self, page: Page) {
        self.go(page);
        if self.history.get(self.history_at) != Some(&page) {
            self.history.truncate(self.history_at + 1);
            self.history.push(page);
            self.history_at = self.history.len() - 1;
        }
    }

    /// Back (-1) or forward (+1) through the pages visited.
    fn travel(&mut self, step: isize) {
        let at = self.history_at as isize + step;
        if at < 0 || at as usize >= self.history.len() {
            return;
        }
        self.history_at = at as usize;
        self.go(self.history[self.history_at]);
    }

    fn go(&mut self, page: Page) {
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
        let changed = page != self.page || self.blocks.is_empty();
        if page.is_welcome() != self.page.is_welcome() || self.nav.is_empty() {
            self.query.clear();
            self.nav = page.nav().to_vec();
        }
        self.page = page;
        self.motion.jump(0.0);
        self.menu = None;
        self.pressed = None;
        self.state.armed = None;
        if !matches!(self.focus, Some(Focus::Nav | Focus::Search)) {
            self.focus = None;
        }
        if page == Page::About {
            self.state.stats.reset();
            self.state.stats.sample();
            self.state.whats_new_seen();
        }
        if page == Page::Privacy {
            self.state.refresh_sizes();
        }
        timer(TIMER_STATS, (page == Page::About).then_some(s::motion::STATS));
        // Times that move on their own: the pause's, the agents' last events.
        timer(TIMER_CLOCK, matches!(page, Page::General | Page::Agents).then_some(CLOCK_MS));
        if changed {
            self.motion.enter_page();
            self.motion.wake();
        }
        self.rebuild();
        self.reveal_match();
    }

    /// With a search typed, the first row it matches on this page scrolls into view.
    fn reveal_match(&mut self) {
        if let Some(rect) = search::first_match(&self.blocks, &self.layout, &self.query) {
            let h = self.dips().1;
            let target = chrome::scroll_to_show(self.motion.scroll_target, rect, h, chrome::max_scroll(self.layout.height, h));
            self.motion.jump(target);
        }
    }

    /// The search changed: the nav keeps the pages that match.
    fn set_query(&mut self, query: String) {
        self.query = query;
        self.nav = search::filter(self.page.nav(), &self.query, &self.state);
        self.hover = Hover::None;
        self.invalidate();
    }

    /// The window came to the front: files may have been edited meanwhile (P-4, P-7).
    pub fn activated(&mut self, active: bool) {
        self.active = active;
        if active {
            self.state.refresh();
            self.rebuild();
        } else {
            self.cancel_recording();
            self.menu = None;
            self.invalidate();
        }
    }

    /// Signalled when the swap chain can take another frame.
    pub fn waitable(&self) -> Option<HANDLE> {
        self.renderer.waitable()
    }

    /// Whether the next display frame should be drawn.
    pub fn needs_frame(&self) -> bool {
        self.dirty || self.motion.moving(&self.hover)
    }

    /// One frame: motion moves on, then the window is drawn and presented.
    pub fn frame(&mut self) {
        let started = Instant::now();
        if crate::perf::enabled() {
            crate::perf::reason(if self.dirty { "dirty" } else { self.motion.why(&self.hover) });
        }
        // After a rest, time starts now rather than at the last frame drawn.
        if self.dirty && !self.motion.moving(&self.hover) {
            self.motion.wake();
        }
        let scroll = self.motion.scroll;
        if self.motion.restless(&self.hover) {
            self.page_fresh = false;
        }
        self.motion.step(&self.hover);
        if self.motion.scroll != scroll {
            self.refresh_hover();
        }
        self.dirty = false;
        let (hwnd, size) = (self.hwnd, self.size);
        let drawn = self.with_view(|renderer, view| renderer.draw(hwnd, size.0, size.1, prefs::color::CONTENT, |p| render::frame::draw(p, view)));
        // A failed frame (the GPU went away) is drawn again on the next change rather than
        // straight away, so a device that can't come back doesn't spin the loop.
        self.page_fresh = drawn.is_ok();
        if let Err(e) = drawn {
            eprintln!("settings: draw failed: {e}");
        }
        crate::perf::frame(started.elapsed().as_secs_f64() * 1000.0);
    }

    fn with_view<R>(&mut self, f: impl FnOnce(&mut Renderer, &View) -> R) -> R {
        let (width, height) = self.dips();
        let knobs = self.motion.knobs();
        let nav: Vec<NavItem> = self.nav.iter().map(|&page| NavItem { title: page.title(), icon: page.icon(), section: page.section() }).collect();
        let typing = match (&self.focus, &self.menu) {
            (Some(Focus::Search), None) => Some(Focus::Search),
            (Some(Focus::Part(t)), None) if matches!(self.part(t), Some(Part::Field { .. })) => Some(Focus::Part(t.clone())),
            _ => None,
        };
        let version = format!("Version {}", pages::VERSION);
        let backdrop = crate::wallpaper::current();
        let motion = &self.motion;
        let lit = |key: &Hover| motion.lit(key);
        let view = View {
            width,
            height,
            welcome: self.page.is_welcome(),
            nav: &nav,
            selected: self.nav.iter().position(|p| *p == self.page),
            query: &self.query,
            title: self.page.title(),
            can_back: self.history_at > 0,
            can_forward: self.history_at + 1 < self.history.len(),
            active: self.active,
            version: &version,
            blocks: &self.blocks,
            layout: &self.layout,
            scroll: self.motion.scroll,
            lit: &lit,
            focus: self.focus.as_ref().filter(|_| self.focus_visible),
            typing: typing.as_ref(),
            pressed: self.pressed.as_ref(),
            knobs: &knobs,
            menu: self.menu.as_ref(),
            scrollbar_active: matches!(self.drag, Some(Drag::Scrollbar(_))),
            backdrop: backdrop.as_deref(),
            enter: self.motion.enter(),
            fresh: self.page_fresh,
        };
        f(&mut self.renderer, &view)
    }

    pub fn on_timer(&mut self, id: usize) {
        match id {
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

    fn scroll_to(&mut self, target: f32) {
        let max = chrome::max_scroll(self.layout.height, self.dips().1);
        if !self.motion.moving(&self.hover) {
            self.motion.wake();
        }
        self.motion.scroll_target = target.clamp(0.0, max);
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
            if !self.motion.moving(&self.hover) {
                self.motion.wake();
            }
            self.motion.slide(&target.row, *on);
        }
        match pages::act(self.page, &mut self.state, target, act) {
            Effect::None => {}
            Effect::Navigate(page) => {
                self.set_page(page);
                return;
            }
            Effect::Record(file) => {
                self.state.recording = Some(file);
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
    /// The wallpaper was read again: the stage shows the new one.
    pub fn on_wallpaper(&mut self) {
        self.invalidate();
    }

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
        let widest = items.iter().map(|i| self.renderer.width(i, layout::Font::Control)).fold(0.0, f32::max);
        let rect = render::menu::place(anchor, items.len().max(1), widest, self.dips());
        let mut menu = Menu { target: target.clone(), items, checked, rect, hot: checked, scroll: 0.0 };
        if let Some(i) = checked {
            // The checked entry in view.
            let visible = rect.h() - 2.0 * prefs::menu::PADDING;
            menu.scroll = (i as f32 * prefs::menu::ITEM_HEIGHT - visible / 2.0).clamp(0.0, render::menu::max_scroll(&menu));
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
        if self.state.recording.is_none() {
            return;
        }
        if vk == 0 {
            self.state.recording = None;
        } else {
            pages::chord(&mut self.state, mods, vk);
        }
        self.rebuild();
    }

    pub fn cancel_recording(&mut self) {
        keys::stop();
        if self.state.recording.take().is_some() {
            self.rebuild();
        }
    }

    /// Page rect → window rect.
    fn to_window(&self, r: layout::Rect) -> layout::Rect {
        r.offset(chrome::page_left(self.dips().0), -self.motion.scroll)
    }
}
