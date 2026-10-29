//! Clipboard: the notch panel as it opens (`reui-hub/src/modules/clipboard/panel.rs`, drawn
//! by `reui-island/src/render/rows/{search,tabs,cards}.rs`): the search field, the tabs,
//! then the row of cards, four and a half across, the half card fading out at the edge.
//! The cards are what a clipboard holds without pictures: notes, a link, a colour, code.
//! On a stage too narrow for the panel at the look's size, the toast a copy shows instead
//! (`toast.rs`): the copy glyph and the text's first words, side by side.

use reui_design::layout::cards::{tab_width, CardRow};
use reui_design::layout::{compact, rows};
use reui_design::look::{concentric, Look};
use reui_design::tokens::{color, component, icon, island, space, typography};
use reui_design::Color;
use windows::core::Result;

use super::parts::{glyph, line};
use super::rgb;
use crate::render::{Align, Painter};
use crate::ui::layout::{Font, IslandFont, Rect};
use crate::ui::{stage, Moment};

/// The module's accent.
const ACCENT: Color = rgb(10, 132, 255);
const SEARCH: char = '\u{E721}';
const LINK: char = '\u{E774}';
const CODE: char = '\u{E943}';
const COPY: char = '\u{E8C8}';
/// What the toast shows of the copied text (`toast.rs` cuts it at 24 characters).
const GLIMPSE: &str = "Move the review to 3 PM…";
const TABS: [(&str, &str, bool); 2] = [("History", "24", true), ("Pinned", "3", false)];

enum Face {
    /// Several lines: the first as a title, the rest quieter.
    Note(&'static str, &'static str),
    Text(&'static str),
    Link(&'static str),
    Color(Color, &'static str),
    Code(&'static str),
}

/// Each card: its face, its age, its one fact.
const CARDS: [(Face, &str, &str); 5] = [
    (Face::Note("Thursday", "Move the review to 3 PM and send the agenda before lunch"), "now", "2 lines"),
    (Face::Link("Keyboard shortcuts in Windows"), "4m", "microsoft.com"),
    (Face::Color(rgb(94, 92, 230), "#5E5CE6"), "18m", "HEX"),
    (Face::Code("npm run build"), "1h", ""),
    (Face::Text("See you at the station at 6"), "2h", ""),
];

pub fn draw(p: &Painter, look: &Look) -> Result<()> {
    let (w, h) = stage::size(Moment::Clipboard, look);
    let radius = look.panel_radius();
    super::parts::shape(p, look, Rect::at(0.0, 0.0, w, h), radius, (true, true))?;
    let pal = &look.palette;
    let inset = look.inset();
    let capsule = |h: f32| look.capsule(h, radius, inset);
    let [search_y, tabs_y, cards_y] = stage::panel_rows(look);
    // Search: the field across the panel, its glyph and placeholder.
    let (sh, pad) = (component::search::HEIGHT, component::search::PADDING);
    let y = search_y;
    p.fill(Rect::new(inset, y, w - inset, y + sh), capsule(sh), pal.fill_quaternary)?;
    let g = icon::SEARCH;
    glyph(p, SEARCH, g, inset + pad, y + (sh - g) / 2.0, pal.label_secondary)?;
    let lh = typography::DETAIL.line_height;
    line(p, "Search", IslandFont::Detail, rows::search_text_left(look), y + (sh - lh) / 2.0, 200.0, Align::Start, pal.label_tertiary)?;
    // Tabs: the selected one filled, each with its count.
    let y = tabs_y;
    let th = component::tabs::HEIGHT;
    let tpad = component::tabs::PADDING;
    let mut x = inset;
    let tab_lh = typography::TAB.line_height;
    for (label, count, selected) in TABS {
        let lw = p.measure(label, Font::Island(IslandFont::Tab));
        let cw = p.measure(count, Font::Island(IslandFont::Tab));
        let tw = tab_width(lw, cw);
        let (ink, quiet) = if selected { (pal.label_primary, pal.label_secondary) } else { (pal.label_secondary, pal.label_tertiary) };
        if selected {
            p.fill(Rect::at(x, y, tw, th), capsule(th), pal.fill_secondary)?;
        }
        let top = y + (th - tab_lh) / 2.0;
        line(p, label, IslandFont::Tab, x + tpad, top, lw + 1.0, Align::Start, ink)?;
        p.figures(count, Font::Island(IslandFont::Tab), Rect::at(x + tw - tpad - cw, top, cw + 1.0, tab_lh), Align::Start, quiet)?;
        x += tw + component::tabs::GAP;
    }
    // Cards: four and a half across, clipped at the inset, fading out at the right.
    let y = cards_y;
    let row = CardRow::new(w, CARDS.len(), inset);
    let cw = row.card_width();
    let ch = component::card::HEIGHT;
    let corner = concentric(radius, inset);
    p.clip(Rect::new(inset, y - 2.0, w - inset, y + ch + 2.0), || {
        for (i, (face, age, fact)) in CARDS.iter().enumerate() {
            card(p, look, face, age, fact, Rect::at(row.left(i, 0.0), y, cw, ch), corner)?;
        }
        let fade = component::card::FADE;
        p.fade_in_right(Rect::new(w - inset - fade, y - 2.0, w - inset, y + ch + 2.0), pal.background)
    })
}

fn card(p: &Painter, look: &Look, face: &Face, age: &str, fact: &str, r: Rect, corner: f32) -> Result<()> {
    let pal = &look.palette;
    let pad = component::card::PADDING;
    let footer = component::card::FOOTER;
    let body = Rect::new(r.l + pad, r.t + pad, r.r - pad, r.b - footer - pad);
    let wrap = |text: &str, font: IslandFont, top: f32, bottom: f32, ink: Color| {
        let line = super::super::fonts::style(Font::Island(font)).line_height;
        let lines = ((bottom - top) / line).floor();
        if lines < 1.0 {
            return Ok(());
        }
        p.clip(Rect::new(body.l, top, body.r, top + lines * line), || p.paragraph(text, Font::Island(font), Rect::new(body.l, top, body.r, top + lines * line), ink))
    };
    let mut on_media = false;
    match face {
        Face::Note(first, rest) => {
            p.fill(r, corner, pal.fill_tertiary)?;
            line(p, first, IslandFont::CardLabel, body.l, body.t, body.w(), Align::Start, pal.label_primary)?;
            let top = body.t + typography::CARD_LABEL.line_height + space::XS;
            wrap(rest, IslandFont::CardText, top, body.b, pal.label_secondary)?;
        }
        Face::Text(text) => {
            p.fill(r, corner, pal.fill_tertiary)?;
            wrap(text, IslandFont::CardText, body.t, body.b, pal.label_primary)?;
        }
        Face::Code(text) => {
            p.fill(r, corner, pal.fill_tertiary)?;
            let k = component::card::KIND_ICON;
            glyph(p, CODE, k, r.r - pad - k, body.t, pal.label_tertiary)?;
            let code = Rect::new(body.l, body.t, body.r - k - space::XS, body.b);
            let lh = typography::CARD_CODE.line_height;
            let lines = (code.h() / lh).floor();
            p.clip(Rect::new(code.l, code.t, code.r, code.t + lines * lh), || p.paragraph(text, Font::Island(IslandFont::CardCode), code, pal.label_primary))?;
        }
        Face::Link(title) => {
            p.fill(r, corner, pal.fill_tertiary)?;
            // No page image: the link glyph in the accent, then the title.
            glyph(p, LINK, icon::COMPACT, body.l, body.t, ACCENT)?;
            let top = body.t + icon::COMPACT + pad * 0.5;
            wrap(title, IslandFont::CardTitle, top, r.b - footer - pad * 0.5, pal.label_primary)?;
        }
        Face::Color(swatch, hex) => {
            p.fill(r, corner, *swatch)?;
            let light = 0.2126 * swatch.r + 0.7152 * swatch.g + 0.0722 * swatch.b > 0.55;
            let ink = if light { color::label::ON_ACCENT } else { color::label::PRIMARY };
            line(p, hex, IslandFont::CardLabel, body.l, body.t, body.w(), Align::Start, ink)?;
            on_media = !light;
        }
    }
    // The footer: the age, and the one fact at the right.
    let top = r.b - footer - pad * 0.25;
    let mid = top + footer / 2.0;
    let (ink, quiet) = if on_media { (color::label::PRIMARY, color::label::PRIMARY.with_alpha(0.75)) } else { (pal.label_secondary, pal.label_tertiary) };
    let cap = typography::CAPTION.line_height;
    let age_w = p.measure(age, Font::Island(IslandFont::Caption));
    line(p, age, IslandFont::Caption, r.l + pad, mid - cap / 2.0, age_w + 1.0, Align::Start, ink)?;
    let from = r.l + pad + age_w + pad * 0.75;
    if !fact.is_empty() && r.r - pad > from {
        line(p, fact, IslandFont::Caption, from, mid - cap / 2.0, r.r - pad - from, Align::End, quiet)?;
    }
    Ok(())
}

/// The toast's width: the glyph tile, the glimpse beside it (`compact.rs`, a text label
/// with its mark), never narrower than the resting pill.
pub fn toast_width(p: &Painter) -> f32 {
    toast_layout(p).width
}

fn toast_layout(p: &Painter) -> compact::Joined {
    compact::joined(component::art::COMPACT, compact::margin(true), p.measure(GLIMPSE, Font::Island(IslandFont::Compact)))
}

pub fn toast(p: &Painter, look: &Look) -> Result<()> {
    let j = toast_layout(p);
    let (w, h) = (j.width, island::HEIGHT);
    super::parts::shape(p, look, Rect::at(0.0, 0.0, w, h), look.rounded(h), (true, true))?;
    let (m, a) = (compact::margin(true), component::art::COMPACT);
    super::parts::glyph_tile(p, look, look.radius(), true, COPY, m, (h - a) / 2.0, ACCENT)?;
    let x = j.text_x;
    let lh = typography::COMPACT.line_height;
    line(p, GLIMPSE, IslandFont::Compact, x, (h - lh) / 2.0, w - x - island::compact::TEXT_MARGIN + 1.0, Align::Start, look.palette.label_secondary)
}
