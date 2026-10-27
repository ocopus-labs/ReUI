//! The page: its title and paragraph, group eyebrows, row wells with their title and quiet
//! line, figures; controls are drawn by `controls`.

use reui_design::tokens::{color, settings as s};
use reui_design::Color;
use windows::core::Result;

use super::frame::View;
use super::{Align, Painter};
use crate::ui::layout::{Font, Rect, Shape};
use crate::ui::{Block, Tone};

/// Draws the page with its origin at (`dx`, `dy`) in the window.
pub fn draw(p: &Painter, v: &View, dx: f32, dy: f32) -> Result<()> {
    let visible = |r: &Rect| r.b + dy >= 0.0 && r.t + dy <= v.height;
    for shape in &v.layout.shapes {
        match shape {
            Shape::Intro { block, eyebrow, title, body } => {
                let Block::Intro { eyebrow: label, title: text, body: paragraph } = &v.blocks[*block] else { continue };
                if let (Some(r), Some(label)) = (eyebrow, label) {
                    p.text(label, Font::Label, r.offset(dx, dy), Align::Start, color::label::SECONDARY)?;
                }
                p.text(text, Font::Title, title.offset(dx, dy), Align::Start, color::label::PRIMARY)?;
                if let (Some(r), Some(paragraph)) = (body, paragraph) {
                    p.paragraph(paragraph, Font::Intro, r.offset(dx, dy), color::label::SECONDARY)?;
                }
            }
            Shape::Label { block, rect } if visible(rect) => {
                let text = match &v.blocks[*block] {
                    Block::Group { label: Some(label), .. } | Block::Figures { label: Some(label), .. } | Block::Text { label: Some(label), .. } => label,
                    _ => continue,
                };
                p.text(text, Font::Label, rect.offset(dx, dy), Align::Start, color::label::SECONDARY)?;
            }
            Shape::Row { block, row, rect, top, bottom, text } if visible(rect) => {
                let Block::Group { rows, .. } = &v.blocks[*block] else { continue };
                let row = &rows[*row];
                if !row.bare {
                    p.fill_corners(rect.offset(dx, dy), *top, *bottom, s::color::WELL)?;
                }
                p.alpha.set(if row.enabled { 1.0 } else { 0.4 });
                let t = text.offset(dx, dy);
                let title_font = if row.mono { Font::Mono } else { Font::RowTitle };
                let title_h = s::typography::ROW_TITLE.line_height;
                match &row.detail {
                    Some(detail) => {
                        let detail_h = s::typography::ROW_DETAIL.line_height;
                        let top = t.t + (t.h() - title_h - s::row::LINE_GAP - detail_h) / 2.0;
                        p.text(&row.title, title_font, Rect::new(t.l, top, t.r, top + title_h), Align::Start, color::label::PRIMARY)?;
                        let dt = top + title_h + s::row::LINE_GAP;
                        p.text(detail, Font::RowDetail, Rect::new(t.l, dt, t.r, dt + detail_h), Align::Start, tone(row.tone))?;
                    }
                    None => p.text(&row.title, title_font, t, Align::Start, color::label::PRIMARY)?,
                }
                p.alpha.set(1.0);
            }
            Shape::Text { block, rect, title, body } if visible(rect) => {
                let Block::Text { title: heading, body: text, accent, .. } = &v.blocks[*block] else { continue };
                let well = if *accent { color::accent::DEFAULT.with_alpha(reui_design::tokens::opacity::TINT_FILL) } else { s::color::WELL };
                p.fill(rect.offset(dx, dy), s::row::RADIUS, well)?;
                p.text(heading, Font::RowTitle, title.offset(dx, dy), Align::Start, color::label::PRIMARY)?;
                p.paragraph(text, Font::RowDetail, body.offset(dx, dy), color::label::SECONDARY)?;
            }
            Shape::Figures { block, rect } if visible(rect) => {
                let Block::Figures { items, .. } = &v.blocks[*block] else { continue };
                let r = rect.offset(dx, dy);
                p.fill(r, s::row::RADIUS, s::color::WELL)?;
                let mut x = r.l + s::row::PADDING;
                let top = r.t + s::row::PADDING;
                for (figure, caption) in items {
                    let w = p.measure(figure, Font::Figure).max(p.measure(caption, Font::Label));
                    let fh = s::typography::FIGURE.line_height;
                    p.figures(figure, Font::Figure, Rect::new(x, top, x + w + 1.0, top + fh), Align::Start, color::label::PRIMARY)?;
                    let ct = top + fh + s::row::LINE_GAP;
                    p.text(caption, Font::Label, Rect::new(x, ct, x + w + 1.0, ct + s::typography::LABEL.line_height), Align::Start, color::label::SECONDARY)?;
                    x += w + s::stat::GAP;
                }
            }
            _ => {}
        }
    }
    for placed in &v.layout.parts {
        let r = placed.rect.offset(dx, dy);
        if r.b < 0.0 || r.t > v.height {
            continue;
        }
        let Some(part) = super::controls::part_of(v.blocks, &placed.target) else { continue };
        p.alpha.set(if placed.enabled { 1.0 } else { 0.4 });
        let drawn = super::controls::draw(p, v, placed, part, dx, dy);
        p.alpha.set(1.0);
        drawn?;
    }
    Ok(())
}

pub fn tone(tone: Tone) -> Color {
    match tone {
        Tone::Secondary => color::label::SECONDARY,
        Tone::Positive => color::system::GREEN,
        Tone::Warning => color::system::ORANGE,
        Tone::Negative => color::system::RED,
    }
}
