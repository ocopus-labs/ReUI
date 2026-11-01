//! Clipboard: the notch panel as it opens (`reui-hub/src/modules/clipboard/panel.rs`, drawn
//! by `reui-island/src/render/rows/{search,cards}.rs`): the toolbar (type filters, the
//! search field, the count, Clear, settings), then the row of cards, three and the
//! edge of a fourth, fading out at the edge.
//! The cards are what a clipboard holds without pictures: a link, a note, files, a colour,
//! code.
//! On a stage too narrow for the panel at the look's size, the toast a copy shows instead
//! (`toast.rs`): the copy glyph and the text's first words, side by side.

use reui_design::layout::cards::{CardRow, Face as CardFace};
use reui_design::layout::rows::{self, Filters};
use reui_design::layout::compact;
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
const LINK: char = '\u{E71B}';
const FILES: char = '\u{E8A5}';
const COPY: char = '\u{E8C8}';
/// What the toast shows of the copied text (`toast.rs` cuts it at 24 characters).
const GLIMPSE: &str = "Move the review to 3 PM…";
/// The type filters (`panel.rs`): All (the history clock, chosen), text, links, images,
/// files, colours.
const FILTERS: [char; 6] = ['\u{E81C}', 'T', '\u{E71B}', '\u{EB9F}', '\u{E8A5}', '\u{E790}'];
const CLEAR: char = '\u{E74D}';
const SETTINGS: char = '\u{E713}';
const COUNT: &str = "24";

enum Face {
    Text(&'static str),
    /// Site, title, address.
    Link(&'static str, &'static str, &'static str),
    Color(Color),
    Code(&'static str),
    /// Files: the glyph until thumbnails come.
    Files,
}

/// A card as the hub sends it (`panel.rs`): its face, tag, name, the fact after the tag,
/// and its age.
type Card = (Face, &'static str, Option<&'static str>, Option<&'static str>, &'static str);

const CARDS: [Card; 5] = [
    (Face::Link("Wikipedia", "Sea of clouds", "en.wikipedia.org/wiki/Sea_of_clouds"), "links", None, Some("en.wikipedia.org"), "just now"),
    (Face::Text("Move the review to 3 PM and send the agenda before lunch"), "text", None, None, "4m ago"),
    (Face::Files, "3 files", Some("Itinerary.pdf"), None, "18m ago"),
    (Face::Color(rgb(94, 92, 230)), "color", Some("#5E5CE6"), None, "1h ago"),
    (Face::Code("npm run build"), "code", None, None, "2h ago"),
];

pub fn draw(p: &Painter, look: &Look) -> Result<()> {
    let (w, h) = stage::size(Moment::Clipboard, look);
    let radius = look.panel_radius();
    super::parts::shape(p, look, Rect::at(0.0, 0.0, w, h), radius, (true, true))?;
    let pal = &look.palette;
    let inset = look.inset();
    let capsule = |h: f32| look.capsule(h, radius, inset);
    let [search_y, cards_y] = stage::panel_rows(look);
    // The toolbar, right to left as the island lays it out: the bare gear, Clear, the
    // count; then the filters from the left, and the field between.
    let (sh, pad) = (component::search::HEIGHT, component::search::PADDING);
    let y = search_y;
    let button_lh = typography::BUTTON.line_height;
    let mut right = w - inset;
    let gear = sh * 0.75;
    right -= gear;
    glyph(p, SETTINGS, icon::BUTTON, right + (gear - icon::BUTTON) / 2.0, y + (sh - icon::BUTTON) / 2.0, pal.label_primary)?;
    right -= component::search::BUTTON_GAP;
    let clear_text = p.measure("Clear", Font::Island(IslandFont::Button));
    let clear_w = rows::search_button(Some(icon::BUTTON + space::XS + clear_text));
    right -= clear_w;
    p.fill(Rect::at(right, y, clear_w, sh), capsule(sh), pal.fill_quaternary)?;
    glyph(p, CLEAR, icon::BUTTON, right + pad, y + (sh - icon::BUTTON) / 2.0, pal.label_primary)?;
    line(p, "Clear", IslandFont::Button, right + pad + icon::BUTTON + space::XS, y + (sh - button_lh) / 2.0, clear_text + 1.0, Align::Start, pal.label_primary)?;
    right -= component::search::BUTTON_GAP;
    let count_w = rows::search_button(Some(p.measure(COUNT, Font::Island(IslandFont::Button))));
    right -= count_w;
    p.fill(Rect::at(right, y, count_w, sh), capsule(sh), pal.fill_quaternary)?;
    line(p, COUNT, IslandFont::Button, right, y + (sh - button_lh) / 2.0, count_w, Align::Center, pal.label_primary)?;
    right -= component::search::BUTTON_GAP;
    let group = Filters::new(inset, FILTERS.len());
    p.fill(Rect::at(inset, y, group.width(), sh), capsule(sh), pal.fill_quaternary)?;
    let (chip, g) = (Filters::chip(), icon::SEARCH);
    let chip_top = y + component::search::FILTER_INSET;
    for (i, &filter) in FILTERS.iter().enumerate() {
        let l = group.chip_left(i);
        // The chosen one, as a primary button: the label's ink as the fill.
        let ink = if i == 0 {
            p.fill(Rect::at(l, chip_top, chip, chip), chip / 2.0, pal.label_primary)?;
            pal.label_on_accent
        } else {
            pal.label_secondary
        };
        if filter.is_ascii_alphanumeric() {
            // A letter is set in the text face, as the island does.
            let lh = typography::FILTER_LETTER.line_height;
            line(p, &filter.to_string(), IslandFont::FilterLetter, l, chip_top + (chip - lh) / 2.0, chip, Align::Center, ink)?;
        } else {
            glyph(p, filter, g, l + (chip - g) / 2.0, chip_top + (chip - g) / 2.0, ink)?;
        }
    }
    let field = group.field_left();
    right = right.min(field + component::search::TOOLBAR_WIDTH);
    p.fill(Rect::new(field, y, right, y + sh), capsule(sh), pal.fill_quaternary)?;
    glyph(p, SEARCH, g, field + pad, y + (sh - g) / 2.0, pal.label_secondary)?;
    let lh = typography::DETAIL.line_height;
    let text_left = rows::search_text_after(field);
    line(p, "Search history\u{2026}", IslandFont::Detail, text_left, y + (sh - lh) / 2.0, (right - pad - text_left).max(0.0), Align::Start, pal.label_tertiary)?;
    // Cards: three and the edge of a fourth, clipped at the inset, fading out at the right.
    let y = cards_y;
    let row = CardRow::new(w, CARDS.len(), inset);
    let cw = row.card_width();
    let ch = component::card::HEIGHT;
    let corner = concentric(radius, inset);
    p.clip(Rect::new(inset, y - 2.0, w - inset, y + ch + 2.0), || {
        for (i, card_data) in CARDS.iter().enumerate() {
            let l = row.left(i, 0.0);
            card(p, look, card_data, Rect::at(l, y, cw, ch), &CardFace::new(l, y, cw), corner)?;
        }
        let fade = component::card::FADE;
        p.fade_in_right(Rect::new(w - inset - fade, y - 2.0, w - inset, y + ch + 2.0), pal.background)
    })
}

/// One card as the island draws it (`reui-island/src/render/rows/faces.rs`).
fn card(p: &Painter, look: &Look, (face, tag, name, fact, age): &Card, r: Rect, f: &CardFace, corner: f32) -> Result<()> {
    let pal = &look.palette;
    let one_line = name.is_none();
    let bottom = if one_line { f.tall_body_bottom() } else { f.body_bottom };
    let wrap = |text: &str, font: IslandFont, top: f32, bottom: f32, ink: Color| {
        let line = super::super::fonts::style(Font::Island(font)).line_height;
        let lines = ((bottom - top) / line).floor();
        if lines < 1.0 {
            return Ok(());
        }
        let area = Rect::new(f.left, top, f.right, top + lines * line);
        p.clip(area, || p.paragraph(text, Font::Island(font), area, ink))
    };
    let (mut tag_ink, mut name_ink, mut age_ink) = (pal.label_secondary, pal.label_primary, pal.label_secondary.with_alpha(0.8));
    match face {
        Face::Text(text) => {
            p.fill(r, corner, pal.fill_quaternary)?;
            wrap(text, IslandFont::CardText, f.top, bottom, pal.label_primary)?;
        }
        Face::Code(text) => {
            p.fill(r, corner, pal.fill_quaternary)?;
            wrap(text, IslandFont::CardCode, f.top, bottom, pal.label_primary)?;
        }
        Face::Link(site, title, address) => {
            p.fill(r, corner, pal.fill_quaternary)?;
            let lh = typography::CARD_LABEL.line_height;
            let g = icon::STAR + 3.0;
            glyph(p, LINK, g, f.left, f.top + (lh - g) / 2.0, pal.label_secondary)?;
            line(p, site, IslandFont::CardLabel, f.left + g + 6.0, f.top, f.width() - g - 6.0, Align::Start, pal.label_secondary)?;
            let title_top = f.top + lh + 6.0;
            line(p, title, IslandFont::CardTitle, f.left, title_top, f.width(), Align::Start, pal.label_primary)?;
            line(p, address, IslandFont::CardLabel, f.left, title_top + typography::CARD_TITLE.line_height + 2.0, f.width(), Align::Start, pal.label_tertiary)?;
        }
        Face::Color(swatch) => {
            p.fill(r, corner, *swatch)?;
            let light = 0.2126 * swatch.r + 0.7152 * swatch.g + 0.0722 * swatch.b > 0.55;
            let ink = if light { color::label::ON_ACCENT } else { color::label::PRIMARY };
            (tag_ink, name_ink, age_ink) = (ink.with_alpha(0.7), ink, ink.with_alpha(0.55));
        }
        Face::Files => {
            p.fill(r, corner, pal.fill_quaternary)?;
            let g = icon::TILE;
            glyph(p, FILES, g, f.left + (f.width() - g) / 2.0, f.top + (bottom - f.top - g) / 2.0, pal.label_tertiary)?;
        }
    }
    // The footer: the age at the right; the tag over the name, or the tag and its fact.
    let name_lh = typography::CARD_NAME.line_height;
    let age_w = p.measure(age, Font::Island(IslandFont::CardAge));
    line(p, age, IslandFont::CardAge, f.right - age_w, f.name_top + (name_lh - typography::CARD_AGE.line_height) / 2.0, age_w + 1.0, Align::End, age_ink)?;
    let room = (f.right - age_w - component::card::PADDING * 0.5 - f.left).max(0.0);
    match name {
        Some(name) => {
            line(p, tag, IslandFont::CardTag, f.left, f.tag_top, room, Align::Start, tag_ink)?;
            line(p, name, IslandFont::CardName, f.left, f.name_top, room, Align::Start, name_ink)?;
        }
        None => {
            let top = f.name_top + (name_lh - typography::CARD_LABEL.line_height) / 2.0;
            let tw = p.measure(tag, Font::Island(IslandFont::CardLabel)).min(room);
            line(p, tag, IslandFont::CardLabel, f.left, top, tw + 1.0, Align::Start, if fact.is_some() { name_ink } else { tag_ink })?;
            if let Some(fact) = fact {
                line(p, &format!(" \u{00B7} {fact}"), IslandFont::CardLabel, f.left + tw, top, (room - tw).max(0.0), Align::Start, pal.label_secondary)?;
            }
        }
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
