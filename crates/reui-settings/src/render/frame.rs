//! The window around the page and the order everything is drawn in: the sidebar, the page
//! (clipped under the toolbar), the toolbar (back, forward, title, window buttons, and a
//! hairline once the page scrolls), the scrollbar, and an open menu over it all.

use std::collections::HashMap;

use reui_design::tokens::{color, prefs as s};
use windows::core::Result;

use super::{fade, mix, Align, Painter, Slot};
use crate::ui::layout::{Font, Layout, Rect};
use crate::ui::{chrome, icons, Block, Focus, Hover, Icon, Menu, Target};

/// A page in the sidebar.
pub struct NavItem {
    pub title: &'static str,
    pub icon: Icon,
    pub section: usize,
}

/// Everything a frame shows.
pub struct View<'a> {
    pub width: f32,
    pub height: f32,
    /// The welcome's steps are showing (no search).
    pub welcome: bool,
    pub nav: &'a [NavItem],
    /// The current page's place in `nav`, if the search left it there.
    pub selected: Option<usize>,
    pub query: &'a str,
    pub title: &'a str,
    pub can_back: bool,
    pub can_forward: bool,
    /// The window is in front (the selection is blue, not grey).
    pub active: bool,
    pub version: &'a str,
    pub blocks: &'a [Block],
    pub layout: &'a Layout,
    pub scroll: f32,
    /// How hovered something is right now, 0..=1 (fills ease in and out).
    pub lit: &'a dyn Fn(&Hover) -> f32,
    /// Only while the keyboard is in use.
    pub focus: Option<&'a Focus>,
    /// Where typing goes (search field or a text field): its caret shows.
    pub typing: Option<&'a Focus>,
    pub pressed: Option<&'a Target>,
    /// Toggle knobs mid-slide: row id → position 0 (off) ..= 1 (on).
    pub knobs: &'a HashMap<String, f32>,
    pub menu: Option<&'a Menu>,
    pub scrollbar_active: bool,
    /// The top of the user's screen, for the island previews' stage (once it's read).
    pub backdrop: Option<&'a crate::wallpaper::Backdrop>,
    /// How far the page's island preview has settled in, 0..=1.
    pub enter: f32,
    /// Nothing changed but the scroll: last frame's page and sidebar can be reused.
    pub fresh: bool,
}

impl View<'_> {
    pub fn page_left(&self) -> f32 {
        chrome::page_left(self.width)
    }
}

/// Room left and right of the page column in its cached bitmap.
const PAGE_MARGIN: f32 = 8.0;

pub fn draw(p: &Painter, v: &View) -> Result<()> {
    p.cached(Slot::Sidebar, v.fresh, 0.0, 0.0, s::sidebar::WIDTH, v.height, s::color::SIDEBAR, || {
        p.fill(Rect::new(s::sidebar::WIDTH - p.pixel(), 0.0, s::sidebar::WIDTH, v.height), 0.0, s::color::DIVIDER)?;
        super::sidebar::draw(p, v)
    })?;
    let viewport = Rect::new(s::sidebar::WIDTH, s::toolbar::HEIGHT, v.width, v.height);
    // The page column, with room around it for focus rings, drawn once into a bitmap that
    // scrolling then only moves.
    let margin = PAGE_MARGIN;
    let (left, width) = (v.page_left() - margin, chrome::column_width(v.width) + 2.0 * margin);
    p.clip(viewport, || p.cached(Slot::Page, v.fresh, left, -v.scroll, width, v.layout.height, s::color::CONTENT, || super::page::draw(p, v, margin, 0.0, false)))?;
    toolbar(p, v)?;
    scrollbar(p, v)?;
    if let Some(menu) = v.menu {
        super::menu::draw(p, v, menu)?;
    }
    Ok(())
}

fn toolbar(p: &Painter, v: &View) -> Result<()> {
    let under = (v.scroll / 8.0).min(1.0);
    if under > 0.0 {
        p.hairline(s::sidebar::WIDTH, v.width, s::toolbar::HEIGHT - p.pixel(), fade(s::color::DIVIDER, under))?;
    }
    let [back, forward] = chrome::history_buttons();
    for (i, (r, glyph, on)) in [(back, icons::BACK, v.can_back), (forward, icons::FORWARD, v.can_forward)].into_iter().enumerate() {
        let lit = if on { (v.lit)(&Hover::History(i)) } else { 0.0 };
        if lit > 0.0 {
            p.fill(r, s::toolbar::BUTTON_RADIUS, fade(s::color::HOVER, lit))?;
        }
        let ink = if on { color::label::PRIMARY } else { color::label::TERTIARY };
        p.glyph(glyph, s::toolbar::GLYPH, r, ink)?;
    }
    let title = Rect::new(chrome::title_left(), 0.0, chrome::caption_buttons(v.width)[0].l - s::toolbar::INSET, s::toolbar::HEIGHT);
    p.text(v.title, Font::Toolbar, title, Align::Start, color::label::PRIMARY)?;
    for (i, r) in chrome::caption_buttons(v.width).into_iter().enumerate() {
        let lit = (v.lit)(&Hover::Caption(i));
        let close = i == 1;
        if lit > 0.0 {
            let fill = if close { color::system::RED } else { s::color::HOVER };
            p.fill(r, r.h() / 2.0, fade(fill, lit))?;
        }
        let ink = mix(color::label::SECONDARY, color::label::PRIMARY, lit);
        p.glyph(if close { icons::CLOSE } else { icons::MINIMIZE }, s::toolbar::CAPTION_GLYPH, r, ink)?;
    }
    Ok(())
}

fn scrollbar(p: &Painter, v: &View) -> Result<()> {
    let Some((top, len)) = chrome::scrollbar(v.scroll, v.layout.height, v.height) else { return Ok(()) };
    let lit = if v.scrollbar_active { 1.0 } else { (v.lit)(&Hover::Scrollbar) };
    let w = s::scrollbar::WIDTH + (s::scrollbar::HOVER_WIDTH - s::scrollbar::WIDTH) * lit;
    let r = v.width - s::scrollbar::MARGIN;
    let ink = color::label::PRIMARY.with_alpha(0.22 + 0.2 * lit);
    p.fill(Rect::new(r - w, top, r, top + len), w / 2.0, ink)
}
