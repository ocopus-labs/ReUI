//! Clipboard: the notch panel, wide, with its filter tabs and a row of preview cards (an
//! image, wrapped text, a link with its page image, a colour), the half card at the edge
//! saying the row goes on.

use reui_design::tokens::{color, component, island, prefs::section};
use reui_design::Color;
use windows::core::Result;

use super::{art, text, Moment, BLACK};
use crate::render::{Align, Painter};
use crate::ui::icons;
use crate::ui::layout::{Font, IslandFont, Rect};

const ACCENT: Color = section::CLIPBOARD;
const INSET: f32 = 14.0;
const WIDTH: f32 = 560.0;
const TABS: [(&str, &str); 4] = [("All", "24"), ("Text", "12"), ("Links", "5"), ("Images", "7")];

fn height() -> f32 {
    INSET + component::tabs::HEIGHT + island::expanded::ROW_GAP + component::card::HEIGHT + INSET
}

pub fn panel() -> Moment {
    Moment { w: WIDTH, draw }
}

fn draw(p: &Painter, _e: f32) -> Result<()> {
    p.fill(Rect::at(0.0, 0.0, WIDTH, height()), island::expanded::PANEL_RADIUS, BLACK)?;
    // Tabs: capsules, the selected one filled, counts quieter.
    let th = component::tabs::HEIGHT;
    let mut x = INSET;
    for (i, (label, count)) in TABS.iter().enumerate() {
        let lw = p.measure(label, Font::Island(IslandFont::Tab));
        let cw = p.measure(count, Font::Island(IslandFont::Tab));
        let w = lw + component::tabs::COUNT_GAP + cw + 2.0 * component::tabs::PADDING;
        let r = Rect::at(x, INSET, w, th);
        if i == 0 {
            p.fill(r, th / 2.0, color::fill::SECONDARY)?;
        }
        let tx = x + component::tabs::PADDING;
        text(p, label, IslandFont::Tab, tx, r.mid_y(), lw + 1.0, Align::Start, color::label::PRIMARY)?;
        super::figures(p, count, IslandFont::Tab, tx + lw + component::tabs::COUNT_GAP, r.mid_y(), cw + 1.0, Align::Start, color::label::SECONDARY)?;
        x += w + component::tabs::GAP;
    }
    // Cards: 4½ across.
    let gap = component::card::GAP;
    let card_w = (WIDTH - INSET - 4.0 * gap) / component::card::VISIBLE_MORE;
    let top = INSET + th + island::expanded::ROW_GAP;
    let h = component::card::HEIGHT;
    let radius = component::card::RADIUS;
    p.clip(Rect::new(INSET, top - 2.0, WIDTH - INSET, top + h + 2.0), || {
        for i in 0..5 {
            let r = Rect::at(INSET + i as f32 * (card_w + gap), top, card_w, h);
            card(p, i, r, radius)?;
        }
        Ok(())
    })?;
    // The selected card's ring.
    let first = Rect::at(INSET, top, card_w, h);
    p.stroke(first.inflate(-component::card::RING_WIDTH / 2.0), radius - component::card::RING_WIDTH / 2.0, component::card::RING_WIDTH, ACCENT)
}

fn card(p: &Painter, i: usize, r: Rect, radius: f32) -> Result<()> {
    let pad = component::card::PADDING;
    let footer = component::card::FOOTER;
    let well = color::fill::TERTIARY;
    match i {
        // An image, cover-fitted, its footer on a scrim.
        0 => {
            art(p, r, radius, Color { r: 0.36, g: 0.55, b: 0.98, a: 1.0 }, Color { r: 0.12, g: 0.2, b: 0.45, a: 1.0 })?;
            p.circle(r.l + r.w() * 0.68, r.t + r.h() * 0.32, r.w() * 0.12, Color { r: 1.0, g: 0.86, b: 0.55, a: 1.0 })?;
            let hill = Rect::new(r.l, r.t + r.h() * 0.58, r.r, r.b);
            p.clip(r, || p.fill(hill.offset(-r.w() * 0.2, 0.0).inflate(r.w() * 0.15), r.w() * 0.45, Color { r: 0.1, g: 0.32, b: 0.3, a: 1.0 }))?;
            p.fill(Rect::new(r.l, r.b - footer, r.r, r.b), 0.0, Color { r: 0.0, g: 0.0, b: 0.0, a: 0.35 })?;
            text(p, "2m", IslandFont::Caption, r.l + pad, r.b - footer / 2.0, r.w(), Align::Start, color::label::PRIMARY)?;
        }
        // Text, wrapped.
        1 => {
            p.fill(r, radius, well)?;
            let body = Rect::new(r.l + pad, r.t + pad, r.r - pad, r.b - footer);
            p.paragraph("Ship the island before Friday, then the settings window", Font::Island(IslandFont::CardText), body, color::label::PRIMARY)?;
            text(p, "5m", IslandFont::Caption, r.l + pad, r.b - footer / 2.0, r.w(), Align::Start, color::label::TERTIARY)?;
        }
        // A link: the page image across the top half, its title under it.
        2 => {
            p.fill(r, radius, well)?;
            let media = Rect::new(r.l, r.t, r.r, r.t + r.h() * component::card::MEDIA);
            // The image's top corners follow the card; its bottom edge is straight.
            p.gradient(Rect::new(r.l, r.t, r.r, media.b + radius), radius, Color { r: 0.75, g: 0.35, b: 0.95, a: 1.0 }, Color { r: 0.4, g: 0.36, b: 0.9, a: 1.0 })?;
            let under = Rect::new(r.l, media.b, r.r, media.b + radius);
            p.fill(under, 0.0, BLACK)?;
            p.fill(under, 0.0, well)?;
            p.glyph(icons::LINK, 18.0, media, Color { r: 1.0, g: 1.0, b: 1.0, a: 0.9 })?;
            text(p, "Dynamic Island", IslandFont::CardLabel, r.l + pad, media.b + 12.0, r.w() - 2.0 * pad, Align::Start, color::label::PRIMARY)?;
            text(p, "github.com", IslandFont::Caption, r.l + pad, r.b - footer / 2.0, r.w(), Align::Start, color::label::TERTIARY)?;
        }
        // A colour: the swatch, its value.
        3 => {
            p.fill(r, radius, color::system::INDIGO)?;
            text(p, "#5E5CE6", IslandFont::CardLabel, r.l + pad, r.b - footer / 2.0, r.w(), Align::Start, color::label::PRIMARY)?;
        }
        _ => {
            p.fill(r, radius, well)?;
            let body = Rect::new(r.l + pad, r.t + pad, r.r - pad, r.b - footer);
            p.paragraph("cargo build --release", Font::Island(IslandFont::CardText), body, color::label::SECONDARY)?;
        }
    }
    Ok(())
}
