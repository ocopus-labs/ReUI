//! The controls, in the island's language: capsules concentric with their wells, the
//! accent used once per control, tabular numbers, a quiet fill on hover.

use reui_design::tokens::{color, component, opacity, settings as s};
use reui_design::Color;
use windows::core::Result;

use super::frame::View;
use super::{mix, Align, Painter};
use crate::ui::layout::{self, Font, Placed, Rect, SLIDER_READING};
use crate::ui::{Block, Focus, Hover, Part, Role, Target};

const ACCENT: Color = color::accent::DEFAULT;
/// White laid over a control while hovered / pressed.
const HOVER: f32 = 0.07;
const PRESS: f32 = 0.13;

pub fn part_of<'a>(blocks: &'a [Block], target: &Target) -> Option<&'a Part> {
    blocks.iter().find_map(|b| match b {
        Block::Group { rows, .. } => rows.iter().find(|r| r.id == target.row).and_then(|r| r.parts.get(target.part)),
        _ => None,
    })
}

pub fn draw(p: &Painter, v: &View, placed: &Placed, part: &Part, dx: f32, dy: f32) -> Result<()> {
    let r = placed.rect.offset(dx, dy);
    let hover = match v.hover {
        Hover::Part(t, cell) if *t == placed.target => Some(*cell),
        _ => None,
    };
    let pressed = v.pressed == Some(&placed.target);
    let radius = placed.radius;
    match part {
        Part::Toggle { on } => {
            let t = v.knobs.get(&placed.target.row).copied().unwrap_or(if *on { 1.0 } else { 0.0 });
            p.fill(r, radius, mix(color::fill::PRIMARY, ACCENT, t))?;
            let knob = r.h() / 2.0 - s::control::TOGGLE_INSET;
            let left = r.l + r.h() / 2.0;
            let right = r.r - r.h() / 2.0;
            p.circle(left + (right - left) * t, (r.t + r.b) / 2.0, knob, s::color::KNOB)?;
        }
        Part::Segmented { options, selected } => {
            p.fill(r, radius, color::fill::TERTIARY)?;
            for (i, (option, cell)) in options.iter().zip(&placed.cells).enumerate() {
                let c = cell.offset(dx, dy);
                let ink = if i == *selected {
                    p.fill(c, c.h() / 2.0, color::fill::PRIMARY)?;
                    color::label::PRIMARY
                } else if hover == Some(Some(i)) {
                    color::label::PRIMARY
                } else {
                    color::label::SECONDARY
                };
                p.text(option, Font::Control, c, Align::Center, ink)?;
            }
        }
        Part::Slider { value, reading } => {
            let reading_rect = Rect::new(r.l, r.t, r.l + SLIDER_READING, r.b);
            p.figures(reading, Font::Control, reading_rect, Align::End, color::label::SECONDARY)?;
            let (l, right) = layout::slider_bar(r);
            let h = component::progress::HEIGHT;
            let top = p.snap((r.t + r.b - h) / 2.0);
            let end = l + (right - l) * value.clamp(0.0, 1.0);
            p.fill(Rect::new(l, top, right, top + h), h / 2.0, color::fill::TRACK)?;
            p.fill(Rect::new(l, top, end.max(l + h), top + h), h / 2.0, ACCENT)?;
            let thumb = component::thumb::SIZE / 2.0 + if hover.is_some() || pressed { 1.0 } else { 0.0 };
            p.circle(end, (r.t + r.b) / 2.0, thumb, s::color::KNOB)?;
        }
        Part::Button { label, role, armed } => {
            let (fill, ink) = button_colors(*role, *armed);
            p.fill(r, radius, fill)?;
            overlay(p, r, radius, hover.is_some(), pressed)?;
            p.text(label, Font::Control, r, Align::Center, ink)?;
        }
        Part::Hotkey { caps, recording } => {
            p.fill(r, radius, color::fill::TERTIARY)?;
            if *recording || caps.is_empty() {
                p.stroke(r.inflate(-0.75), radius - 0.75, 1.5, ACCENT)?;
                p.text("Press keys", Font::Control, r, Align::Center, ACCENT)?;
            } else {
                overlay(p, r, radius, hover.is_some(), pressed)?;
                for (cap, cell) in caps.iter().zip(&placed.cells) {
                    let c = cell.offset(dx, dy);
                    p.fill(c, c.h() / 2.0, color::fill::SECONDARY)?;
                    p.text(cap, Font::Control, c, Align::Center, color::label::PRIMARY)?;
                }
            }
        }
        Part::Dropdown { label } => {
            p.fill(r, radius, color::fill::SECONDARY)?;
            overlay(p, r, radius, hover.is_some(), pressed || v.menu.is_some_and(|m| m.target == placed.target))?;
            let pad = s::control::PADDING;
            let icon = s::icon::CONTROL;
            p.text(label, Font::Control, Rect::new(r.l + pad, r.t, r.r - pad - icon - s::control::GAP, r.b), Align::Start, color::label::PRIMARY)?;
            p.glyph('\u{E70D}', icon, Rect::new(r.r - pad - icon, r.t, r.r - pad, r.b), color::label::SECONDARY)?;
        }
        Part::Field { text, placeholder } => {
            p.fill(r, radius, color::fill::TERTIARY)?;
            let focused = v.typing == Some(&placed.target);
            if focused {
                p.stroke(r.inflate(-0.75), radius - 0.75, 1.5, ACCENT)?;
            } else {
                overlay(p, r, radius, hover.is_some(), false)?;
            }
            let pad = s::control::PADDING;
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
                let x = p.snap(caret_x);
                let top = (r.t + r.b - h) / 2.0;
                p.fill(Rect::new(x, top, x + component::search::CARET, top + h), component::search::CARET / 2.0, ACCENT)?;
            }
        }
        Part::Remove => {
            let (fill, ink) = if hover.is_some() { (color::system::RED.with_alpha(opacity::TINT_FILL), color::system::RED) } else { (color::fill::TERTIARY, color::label::SECONDARY) };
            p.fill(r, radius, fill)?;
            p.glyph('\u{E711}', s::icon::CONTROL, r, ink)?;
        }
        Part::Value(text) => p.figures(text, Font::RowTitle, r, Align::End, color::label::SECONDARY)?,
    }
    if matches!(v.focus, Some(Focus::Part(t)) if *t == placed.target) && !matches!(part, Part::Field { .. }) {
        ring(p, r, radius)?;
    }
    Ok(())
}

/// The keyboard focus ring: accent, just outside the control, following its radius.
pub fn ring(p: &Painter, r: Rect, radius: f32) -> Result<()> {
    let w = s::control::FOCUS_WIDTH;
    let out = s::control::FOCUS_GAP + w / 2.0;
    p.stroke(r.inflate(out), radius + out, w, ACCENT)
}

fn overlay(p: &Painter, r: Rect, radius: f32, hover: bool, pressed: bool) -> Result<()> {
    let a = if pressed { PRESS } else if hover { HOVER } else { return Ok(()) };
    p.fill(r, radius, Color { r: 1.0, g: 1.0, b: 1.0, a })
}

/// (fill, ink)
fn button_colors(role: Role, armed: bool) -> (Color, Color) {
    match role {
        Role::Primary => (color::label::PRIMARY, color::label::ON_ACCENT),
        Role::Secondary => (color::fill::SECONDARY, color::label::PRIMARY),
        Role::Accent => (ACCENT.with_alpha(opacity::TINT_FILL), ACCENT),
        Role::Destructive if armed => (color::system::RED, color::label::PRIMARY),
        Role::Destructive => (color::system::RED.with_alpha(opacity::TINT_FILL), color::system::RED),
    }
}
