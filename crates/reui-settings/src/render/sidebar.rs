//! The sidebar: the search field, the ReUI row (mark, name, version), and the pages in
//! sections, each with its coloured tile; the selected one on the blue highlight (grey
//! while the window is in the background).

use reui_design::tokens::{color, prefs as s};
use windows::core::Result;

use super::frame::View;
use super::{fade, icons as tiles, Align, Painter};
use crate::ui::layout::{Font, Rect};
use crate::ui::{chrome, icons, Focus, Hover};

pub fn draw(p: &Painter, v: &View) -> Result<()> {
    if !v.welcome {
        search(p, v)?;
    }
    account(p, v)?;
    let sections: Vec<usize> = v.nav.iter().map(|n| n.section).collect();
    let rects = chrome::nav_items(&sections);
    for (i, (item, r)) in v.nav.iter().zip(&rects).enumerate() {
        let selected = v.selected == Some(i);
        let radius = s::sidebar::ITEM_RADIUS;
        if selected {
            p.fill(*r, radius, if v.active { s::color::SELECTION } else { s::color::SELECTION_INACTIVE })?;
        } else {
            let lit = (v.lit)(&Hover::Nav(i));
            if lit > 0.0 {
                p.fill(*r, radius, fade(s::color::HOVER, lit))?;
            }
        }
        let size = s::sidebar::ICON;
        let tile = Rect::at(r.l + s::sidebar::ITEM_PADDING, r.mid_y() - size / 2.0, size, size);
        tiles::tile(p, item.icon, tile, s::sidebar::ICON_RADIUS, s::sidebar::ICON_GLYPH)?;
        let text = Rect::new(tile.r + s::sidebar::LABEL_GAP, r.t, r.r - s::sidebar::ITEM_PADDING, r.b);
        p.text(item.title, Font::Nav, text, Align::Start, color::label::PRIMARY)?;
        if let Some(badge) = item.badge {
            p.text(badge, Font::RowDetail, text, Align::End, color::label::SECONDARY)?;
        }
        if selected && v.focus == Some(&Focus::Nav) {
            super::controls::ring(p, *r, radius)?;
        }
    }
    if v.nav.is_empty() {
        let r = Rect::at(s::sidebar::INSET, chrome::account().b + s::sidebar::SECTION_GAP, s::sidebar::WIDTH - 2.0 * s::sidebar::INSET, s::sidebar::ITEM_HEIGHT);
        p.text("No results", Font::Nav, r, Align::Center, color::label::SECONDARY)?;
    }
    Ok(())
}

fn search(p: &Painter, v: &View) -> Result<()> {
    let r = chrome::search();
    let radius = s::sidebar::SEARCH_RADIUS;
    p.fill(r, radius, s::color::FIELD)?;
    let typing = v.typing == Some(&Focus::Search);
    let lit = (v.lit)(&Hover::Search);
    if typing {
        super::controls::ring(p, r, radius)?;
    } else if lit > 0.0 {
        p.fill(r, radius, fade(s::color::HOVER, lit))?;
    }
    p.edge(r, radius, s::color::FIELD_EDGE)?;
    let glyph = Rect::at(r.l + 8.0, r.t, 16.0, r.h());
    p.glyph(icons::SEARCH, s::control::GLYPH + 1.0, glyph, color::label::SECONDARY)?;
    let text = Rect::new(glyph.r + 5.0, r.t, r.r - 8.0, r.b);
    if v.query.is_empty() {
        p.text("Search", Font::Field, text, Align::Start, color::label::TERTIARY)?;
    } else {
        p.text(v.query, Font::Field, text, Align::Start, color::label::PRIMARY)?;
    }
    if typing {
        let w = if v.query.is_empty() { 0.0 } else { p.measure(v.query, Font::Field) };
        let x = p.snap((text.l + w).min(text.r));
        let h = s::typography::FIELD.line_height;
        p.fill(Rect::at(x, r.mid_y() - h / 2.0, 1.5, h), 0.75, s::color::ACCENT)?;
    }
    Ok(())
}

fn account(p: &Painter, v: &View) -> Result<()> {
    let r = chrome::account();
    let lit = (v.lit)(&Hover::Account);
    if lit > 0.0 {
        p.fill(r, s::sidebar::ITEM_RADIUS, fade(s::color::HOVER, lit))?;
    }
    let size = s::sidebar::ACCOUNT_MARK;
    let mark = Rect::at(r.l + s::sidebar::ITEM_PADDING, r.mid_y() - size / 2.0, size, size);
    tiles::mark(p, mark, size * 0.24)?;
    let left = mark.r + s::sidebar::LABEL_GAP + 1.0;
    let name_h = s::typography::ACCOUNT.line_height;
    let detail_h = s::typography::ACCOUNT_DETAIL.line_height;
    let top = r.mid_y() - (name_h + 1.0 + detail_h) / 2.0;
    p.text("ReUI", Font::Account, Rect::at(left, top, r.r - left, name_h), Align::Start, color::label::PRIMARY)?;
    p.figures(v.version, Font::AccountDetail, Rect::at(left, top + name_h + 1.0, r.r - left, detail_h), Align::Start, color::label::SECONDARY)
}
