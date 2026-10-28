//! The page: the header card (the feature's island on its stage, the tile, title and
//! paragraph), section headers, grouped containers with hairlines between rows, row text
//! and leading tiles, footnotes, text and figure containers. Controls are in `controls`.

use reui_design::tokens::{color, opacity, prefs as s};
use reui_design::Color;
use windows::core::Result;

use super::frame::View;
use super::{icons, preview, Align, Painter};
use crate::ui::layout::{Font, Rect, Shape};
use crate::ui::{Block, Tone};

/// Draws the page with its origin at (`dx`, `dy`); `cull` skips what's outside the window
/// (off when the whole page is drawn into its bitmap).
pub fn draw(p: &Painter, v: &View, dx: f32, dy: f32, cull: bool) -> Result<()> {
    let visible = |r: &Rect| !cull || (r.b + dy >= s::toolbar::HEIGHT && r.t + dy <= v.height);
    for shape in &v.layout.shapes {
        match shape {
            Shape::Header { block, rect, stage, icon, eyebrow, title, body, note } if visible(rect) => {
                let Block::Intro { eyebrow: eyebrow_text, title: title_text, body: body_text, icon: tile, preview: shows, row } = &v.blocks[*block] else { continue };
                let card = rect.offset(dx, dy);
                container(p, card)?;
                if let (Some(stage), Some(shows)) = (stage, shows) {
                    let stage = stage.offset(dx, dy);
                    let tint = match tile {
                        Some(crate::ui::Icon::Glyph(_, c)) => *c,
                        _ => s::section::GENERAL,
                    };
                    preview::draw(p, *shows, stage, tint, v.enter)?;
                }
                if let Some(tile) = tile {
                    icons::tile(p, *tile, icon.offset(dx, dy), s::header::ICON_RADIUS, s::header::ICON_GLYPH)?;
                }
                if let (Some(r), Some(text)) = (eyebrow, eyebrow_text) {
                    p.text(text, Font::Label, r.offset(dx, dy), Align::Start, s::color::ACCENT)?;
                }
                p.text(title_text, Font::Hero, title.offset(dx, dy), Align::Start, color::label::PRIMARY)?;
                if let (Some(r), Some(text)) = (body, body_text) {
                    p.paragraph(text, Font::HeroBody, r.offset(dx, dy), color::label::SECONDARY)?;
                }
                if let (Some(r), Some(row)) = (note, row) {
                    if let Some(detail) = &row.detail {
                        p.text(detail, Font::RowDetail, r.offset(dx, dy), Align::Start, tone(row.tone))?;
                    }
                }
            }
            Shape::Container { rect, .. } if visible(rect) => container(p, rect.offset(dx, dy))?,
            Shape::Label { block, rect } if visible(rect) => {
                let text = match &v.blocks[*block] {
                    Block::Group { label: Some(label), .. } | Block::Figures { label: Some(label), .. } | Block::Text { label: Some(label), .. } => label,
                    _ => continue,
                };
                p.text(text, Font::Label, rect.offset(dx, dy), Align::Start, color::label::SECONDARY)?;
            }
            Shape::Footer { block, rect } if visible(rect) => {
                if let Block::Group { footer: Some(text), .. } = &v.blocks[*block] {
                    p.paragraph(text, Font::Footnote, rect.offset(dx, dy), color::label::SECONDARY)?;
                }
            }
            Shape::Row { block, row, rect, text, icon, separator } if visible(rect) => {
                let Block::Group { rows, .. } = &v.blocks[*block] else { continue };
                let row = &rows[*row];
                if *separator {
                    p.hairline(text.l + dx, rect.r + dx - s::group::PADDING, rect.t + dy, s::color::SEPARATOR)?;
                }
                p.alpha.set(if row.enabled { 1.0 } else { s::opacity::DISABLED });
                if let (Some(r), Some(tile)) = (icon, row.icon) {
                    icons::tile(p, tile, r.offset(dx, dy), s::group::ICON_RADIUS, s::group::ICON_GLYPH)?;
                }
                let t = text.offset(dx, dy);
                let title_font = if row.mono { Font::Mono } else { Font::RowTitle };
                let title_h = s::typography::ROW_TITLE.line_height;
                match &row.detail {
                    Some(detail) => {
                        let detail_h = s::typography::ROW_DETAIL.line_height;
                        let top = t.t + (t.h() - title_h - s::group::LINE_GAP - detail_h) / 2.0;
                        p.text(&row.title, title_font, Rect::new(t.l, top, t.r, top + title_h), Align::Start, color::label::PRIMARY)?;
                        let dt = top + title_h + s::group::LINE_GAP;
                        p.text(detail, Font::RowDetail, Rect::new(t.l, dt, t.r, dt + detail_h), Align::Start, tone(row.tone))?;
                    }
                    None => p.text(&row.title, title_font, t, Align::Start, color::label::PRIMARY)?,
                }
                p.alpha.set(1.0);
            }
            Shape::Text { block, rect, title, body } if visible(rect) => {
                let Block::Text { title: heading, body: text, accent, .. } = &v.blocks[*block] else { continue };
                let r = rect.offset(dx, dy);
                container(p, r)?;
                if *accent {
                    p.fill(r, s::group::RADIUS, s::color::ACCENT.with_alpha(opacity::TINT_FILL * 0.5))?;
                }
                p.text(heading, Font::RowTitle, title.offset(dx, dy), Align::Start, color::label::PRIMARY)?;
                p.paragraph(text, Font::RowDetail, body.offset(dx, dy), color::label::SECONDARY)?;
            }
            Shape::Figures { block, rect } if visible(rect) => {
                let Block::Figures { items, .. } = &v.blocks[*block] else { continue };
                let r = rect.offset(dx, dy);
                container(p, r)?;
                let mut x = r.l + s::group::PADDING;
                let top = r.t + s::group::PADDING;
                for (figure, caption) in items {
                    let w = p.measure(figure, Font::Figure).max(p.measure(caption, Font::Label));
                    let fh = s::typography::FIGURE.line_height;
                    p.figures(figure, Font::Figure, Rect::new(x, top, x + w + 1.0, top + fh), Align::Start, color::label::PRIMARY)?;
                    let ct = top + fh + s::group::LINE_GAP;
                    p.text(caption, Font::Label, Rect::new(x, ct, x + w + 1.0, ct + s::typography::LABEL.line_height), Align::Start, color::label::SECONDARY)?;
                    x += w + s::stat::GAP;
                }
            }
            _ => {}
        }
    }
    for placed in &v.layout.parts {
        if !visible(&placed.rect) {
            continue;
        }
        let Some(part) = super::controls::part_of(v.blocks, &placed.target) else { continue };
        p.alpha.set(if placed.enabled { 1.0 } else { s::opacity::DISABLED });
        let drawn = super::controls::draw(p, v, placed, part, dx, dy);
        p.alpha.set(1.0);
        drawn?;
    }
    Ok(())
}

/// A grouped container: the raised surface with a hairline edge.
fn container(p: &Painter, r: Rect) -> Result<()> {
    p.fill(r, s::group::RADIUS, s::color::GROUP)?;
    p.edge(r, s::group::RADIUS, s::color::GROUP_EDGE)
}

pub fn tone(tone: Tone) -> Color {
    match tone {
        Tone::Secondary => color::label::SECONDARY,
        Tone::Positive => color::system::GREEN,
        Tone::Warning => color::system::ORANGE,
        Tone::Negative => color::system::RED,
    }
}
