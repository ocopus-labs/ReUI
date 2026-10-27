//! The window around the page: nav, window buttons, scrollbar, the fade under the caption,
//! and the order everything is drawn in.

use std::collections::HashMap;

use reui_design::tokens::{color, settings as s};
use windows::core::Result;

use super::{Align, Painter};
use crate::ui::layout::{self, Font, Layout, Rect};
use crate::ui::{Block, Focus, Hover, Menu, Target};

/// Everything a frame shows.
pub struct View<'a> {
    pub width: f32,
    pub height: f32,
    pub nav: &'a [&'static str],
    pub page: usize,
    pub blocks: &'a [Block],
    pub layout: &'a Layout,
    pub scroll: f32,
    pub hover: &'a Hover,
    /// Only while the keyboard is in use.
    pub focus: Option<&'a Focus>,
    pub pressed: Option<&'a Target>,
    /// Toggle knobs mid-slide: row id → position 0 (off) ..= 1 (on).
    pub knobs: &'a HashMap<String, f32>,
    pub menu: Option<&'a Menu>,
    /// The field that has the keyboard (its caret shows).
    pub typing: Option<&'a Target>,
    pub scrollbar_active: bool,
}

impl View<'_> {
    /// Left edge of the page column in the window.
    pub fn page_left(&self) -> f32 {
        layout::page_left(self.width)
    }
}

pub fn draw(p: &Painter, v: &View) -> Result<()> {
    nav(p, v)?;
    let viewport = Rect::new(s::nav::WIDTH, 0.0, v.width, v.height);
    p.clip(viewport, || super::page::draw(p, v, v.page_left(), -v.scroll))?;
    // Content scrolled under the caption fades out instead of being cut.
    if v.scroll > 0.0 {
        let strength = (v.scroll / s::page::GROUP_GAP).min(1.0);
        let band = Rect::new(s::nav::WIDTH, 0.0, v.width, s::caption::HEIGHT);
        p.fill(band, 0.0, s::color::BACKGROUND)?;
        p.fade_down(Rect::new(s::nav::WIDTH, s::caption::HEIGHT, v.width, s::caption::HEIGHT + s::page::GROUP_GAP), s::color::BACKGROUND.with_alpha(strength))?;
    }
    scrollbar(p, v)?;
    caption(p, v)?;
    if let Some(menu) = v.menu {
        super::menu::draw(p, menu)?;
    }
    Ok(())
}

fn nav(p: &Painter, v: &View) -> Result<()> {
    let items = layout::nav_items(v.nav.len());
    for (i, (r, label)) in items.iter().zip(v.nav).enumerate() {
        let selected = i == v.page;
        let hovered = *v.hover == Hover::Nav(i);
        if selected {
            p.fill(*r, r.h() / 2.0, color::fill::SECONDARY)?;
        } else if hovered {
            p.fill(*r, r.h() / 2.0, color::fill::QUATERNARY)?;
        }
        let ink = if selected || hovered { color::label::PRIMARY } else { color::label::SECONDARY };
        let text = Rect::new(r.l + s::nav::ITEM_PADDING, r.t, r.r - s::nav::ITEM_PADDING, r.b);
        p.text(label, Font::Nav, text, Align::Start, ink)?;
        if selected && v.focus == Some(&Focus::Nav) {
            super::controls::ring(p, *r, r.h() / 2.0)?;
        }
    }
    Ok(())
}

fn caption(p: &Painter, v: &View) -> Result<()> {
    let glyphs = ['\u{E921}', '\u{E8BB}'];
    for (i, r) in layout::caption_buttons(v.width).iter().enumerate() {
        let hovered = *v.hover == Hover::Caption(i);
        let ink = if hovered { color::label::PRIMARY } else { color::label::SECONDARY };
        if hovered {
            let fill = if i == 1 { color::system::RED.with_alpha(reui_design::tokens::opacity::TINT_FILL) } else { color::fill::TERTIARY };
            p.fill(*r, r.h() / 2.0, fill)?;
        }
        let ink = if hovered && i == 1 { color::system::RED } else { ink };
        p.glyph(glyphs[i], s::icon::CAPTION, *r, ink)?;
    }
    Ok(())
}

fn scrollbar(p: &Painter, v: &View) -> Result<()> {
    let Some((top, len)) = layout::scrollbar(v.scroll, v.layout.height, v.height) else { return Ok(()) };
    let wide = v.scrollbar_active || *v.hover == Hover::Scrollbar;
    let w = if wide { s::scrollbar::HOVER_WIDTH } else { s::scrollbar::WIDTH };
    let r = v.width - s::scrollbar::MARGIN;
    let ink = if wide { color::label::SECONDARY } else { color::fill::PRIMARY };
    p.fill(Rect::new(r - w, top, r, top + len), w / 2.0, ink)
}
