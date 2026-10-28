//! The controls, in the vocabulary of macOS forms drawn with ReUI's tokens: switches (the
//! accent when on, a white knob concentric with the track's end), pop-up buttons with
//! their up/down chevrons, segmented controls, sliders with a round knob, push buttons by
//! role, the shortcut recorder's key caps, text fields. Hover fills ease in and out.

use reui_design::tokens::{color, prefs as s};
use reui_design::Color;
use windows::core::Result;

use super::frame::View;
use super::{fade, mix, Align, Painter};
use crate::ui::layout::{self, Font, Placed, Rect, RECORDING};
use crate::ui::{icons, Block, Focus, Hover, Part, Role, Target};

const WHITE: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };

pub fn part_of<'a>(blocks: &'a [Block], target: &Target) -> Option<&'a Part> {
    blocks.iter().find_map(|b| match b {
        Block::Group { rows, .. } => rows.iter().find(|r| r.id == target.row).and_then(|r| r.parts.get(target.part)),
        Block::Intro { row: Some(row), .. } if row.id == target.row => row.parts.get(target.part),
        _ => None,
    })
}

pub fn draw(p: &Painter, v: &View, placed: &Placed, part: &Part, dx: f32, dy: f32) -> Result<()> {
    let r = placed.rect.offset(dx, dy);
    let lit = (v.lit)(&Hover::Part(placed.target.clone(), None));
    let pressed = v.pressed == Some(&placed.target);
    let radius = placed.radius;
    match part {
        Part::Toggle { on } => {
            let t = v.knobs.get(&placed.target.row).copied().unwrap_or(if *on { 1.0 } else { 0.0 });
            p.fill(r, radius, mix(s::color::SWITCH_OFF, s::color::ACCENT, t))?;
            if lit > 0.0 && t < 0.5 {
                p.fill(r, radius, fade(s::color::HOVER, lit))?;
            }
            let knob = r.h() / 2.0 - s::control::TOGGLE_INSET;
            let (left, right) = (r.l + r.h() / 2.0, r.r - r.h() / 2.0);
            knob_at(p, left + (right - left) * t, r.mid_y(), knob)?;
        }
        Part::Segmented { options, selected } => {
            p.fill(r, radius, s::color::TRACK)?;
            for (i, (option, cell)) in options.iter().zip(&placed.cells).enumerate() {
                let c = cell.offset(dx, dy);
                let inner = radius - s::control::SEGMENT_INSET;
                let ink = if i == *selected {
                    p.fill(c, inner, s::color::SEGMENT)?;
                    color::label::PRIMARY
                } else {
                    let cell_lit = (v.lit)(&Hover::Part(placed.target.clone(), Some(i)));
                    if cell_lit > 0.0 {
                        p.fill(c, inner, fade(s::color::HOVER, cell_lit))?;
                    }
                    mix(color::label::SECONDARY, color::label::PRIMARY, 0.4 + 0.6 * cell_lit)
                };
                p.text(option, Font::Control, c, Align::Center, ink)?;
            }
        }
        Part::Slider { value, reading } => {
            let reading_rect = Rect::new(r.l, r.t, r.l + s::control::SLIDER_READING, r.b);
            p.figures(reading, Font::Control, reading_rect, Align::End, color::label::SECONDARY)?;
            let (l, right) = layout::slider_bar(r);
            let h = s::control::SLIDER_TRACK;
            let top = p.snap(r.mid_y() - h / 2.0);
            let end = l + (right - l) * value.clamp(0.0, 1.0);
            p.fill(Rect::new(l - h / 2.0, top, right + h / 2.0, top + h), h / 2.0, s::color::SWITCH_OFF)?;
            p.fill(Rect::new(l - h / 2.0, top, end, top + h), h / 2.0, s::color::ACCENT)?;
            let grow = if pressed { 1.0 } else { lit * 0.5 };
            knob_at(p, end, r.mid_y(), s::control::SLIDER_KNOB / 2.0 + grow)?;
        }
        Part::Button { label, role, armed } => {
            let (fill, ink) = button_colors(*role, *armed);
            p.fill(r, radius, fill)?;
            overlay(p, r, radius, lit, pressed)?;
            p.text(label, Font::Control, r, Align::Center, ink)?;
        }
        Part::Hotkey { caps, recording } => {
            p.fill(r, radius, s::color::FIELD)?;
            if *recording || caps.is_empty() {
                p.stroke(r.inflate(-0.75), radius - 0.75, 1.5, s::color::ACCENT)?;
                p.text(RECORDING, Font::Control, r, Align::Center, s::color::ACCENT)?;
            } else {
                p.edge(r, radius, s::color::FIELD_EDGE)?;
                overlay(p, r, radius, lit, pressed)?;
                for (cap, cell) in caps.iter().zip(&placed.cells) {
                    let c = cell.offset(dx, dy);
                    p.fill(c, radius - (r.h() - c.h()) / 2.0, s::color::BUTTON)?;
                    p.text(cap, Font::Control, c, Align::Center, color::label::PRIMARY)?;
                }
            }
        }
        Part::Dropdown { label } => {
            p.fill(r, radius, s::color::BUTTON)?;
            overlay(p, r, radius, lit, pressed || v.menu.is_some_and(|m| m.target == placed.target))?;
            let pad = s::control::PADDING;
            let chevrons = s::control::CHEVRONS;
            p.text(label, Font::Control, Rect::new(r.l + pad, r.t, r.r - pad - chevrons - s::control::GAP, r.b), Align::Start, color::label::PRIMARY)?;
            let col = Rect::new(r.r - pad - chevrons - 2.0, r.t, r.r - pad + 2.0, r.b);
            let step = chevrons * 0.55;
            p.glyph(icons::UP, chevrons, col.offset(0.0, -step), color::label::PRIMARY)?;
            p.glyph(icons::DOWN, chevrons, col.offset(0.0, step), color::label::PRIMARY)?;
        }
        Part::Field { text, placeholder } => {
            p.fill(r, radius, s::color::FIELD)?;
            let focused = v.typing == Some(&Focus::Part(placed.target.clone()));
            if focused {
                ring(p, r, radius)?;
            } else {
                overlay(p, r, radius, lit * 0.6, false)?;
            }
            p.edge(r, radius, s::color::FIELD_EDGE)?;
            let pad = s::control::PADDING - 3.0;
            let inner = Rect::new(r.l + pad, r.t, r.r - pad, r.b);
            let mut caret_x = inner.l;
            p.clip(Rect::new(inner.l, r.t, inner.r + 2.0, r.b), || {
                if text.is_empty() {
                    p.text(placeholder, Font::Field, inner, Align::Start, color::label::TERTIARY)
                } else {
                    // Long text shows its end, where the typing is.
                    let w = p.measure(text, Font::Field);
                    let x = if w > inner.w() { inner.r - w } else { inner.l };
                    caret_x = x + w;
                    p.text(text, Font::Field, Rect::new(x, r.t, x + w + 1.0, r.b), Align::Start, color::label::PRIMARY)
                }
            })?;
            if focused {
                let h = s::typography::FIELD.line_height;
                p.fill(Rect::at(p.snap(caret_x), r.mid_y() - h / 2.0, 1.5, h), 0.75, s::color::ACCENT)?;
            }
        }
        Part::Remove => {
            let red = color::system::RED;
            p.fill(r, radius, mix(s::color::BUTTON, red, lit))?;
            p.glyph(icons::CANCEL, s::control::GLYPH - 2.0, r, mix(color::label::SECONDARY, WHITE, lit))?;
        }
        Part::Value(text) => p.figures(text, Font::RowTitle, r, Align::End, color::label::SECONDARY)?,
    }
    if matches!(v.focus, Some(Focus::Part(t)) if *t == placed.target) && !matches!(part, Part::Field { .. }) {
        ring(p, r, radius)?;
    }
    Ok(())
}

/// A white knob with a soft edge under it.
fn knob_at(p: &Painter, cx: f32, cy: f32, radius: f32) -> Result<()> {
    p.circle(cx, cy + 0.5, radius + 0.5, s::color::KNOB_SHADOW)?;
    p.circle(cx, cy, radius, s::color::KNOB)
}

/// The keyboard focus ring: the accent, just outside the control, following its radius.
pub fn ring(p: &Painter, r: Rect, radius: f32) -> Result<()> {
    let w = s::control::FOCUS_WIDTH;
    p.stroke(r.inflate(w / 2.0), radius + w / 2.0, w, s::color::FOCUS)
}

fn overlay(p: &Painter, r: Rect, radius: f32, lit: f32, pressed: bool) -> Result<()> {
    if pressed {
        return p.fill(r, radius, s::color::PRESS);
    }
    if lit > 0.0 {
        p.fill(r, radius, fade(s::color::HOVER, lit))?;
    }
    Ok(())
}

/// (fill, ink)
fn button_colors(role: Role, armed: bool) -> (Color, Color) {
    match role {
        Role::Primary => (s::color::ACCENT, WHITE),
        Role::Secondary => (s::color::BUTTON, color::label::PRIMARY),
        Role::Accent => (s::color::BUTTON, s::color::ACCENT),
        Role::Destructive if armed => (color::system::RED, WHITE),
        Role::Destructive => (s::color::BUTTON, color::system::RED),
    }
}
