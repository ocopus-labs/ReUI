//! Coding agents: the pill asking (the Claude mark on a tinted circle, "Allow?") and the
//! permission card (mark tile, eyebrow, the command in a chip, Deny / Allow once).

use reui_design::tokens::{color, component, island, opacity, radius};
use reui_design::Color;
use windows::core::Result;

use super::{compact_width, pill, text, Moment, BLACK};
use crate::render::{Align, Painter};
use crate::ui::layout::{Font, IslandFont, Rect};

/// Claude's own orange: the activity's accent.
const ACCENT: Color = Color { r: 0.851, g: 0.467, b: 0.341, a: 1.0 };
const WIDTH: f32 = 420.0;
const ASK: &str = "Allow?";

fn compact() -> f32 {
    compact_width(component::art::COMPACT + island::compact::SIDE_MARGIN, 52.0 + island::compact::TEXT_MARGIN)
}

fn height() -> f32 {
    use island::expanded as ex;
    2.0 * ex::INSET + component::art::EXPANDED + ex::SECTION_GAP + component::button::HEIGHT
}

pub fn moments() -> Vec<Moment> {
    vec![Moment { w: compact(), draw: draw_compact }, Moment { w: WIDTH, draw: draw_card }]
}

/// The mark on an accent-tinted circle or tile.
fn mark(p: &Painter, r: Rect, radius: f32) -> Result<()> {
    p.fill(r, radius, ACCENT.with_alpha(opacity::TINT_FILL))?;
    let m = r.w() * opacity::LOGO;
    p.claude(Rect::at(r.l + (r.w() - m) / 2.0, r.t + (r.h() - m) / 2.0, m, m))
}

fn draw_compact(p: &Painter, _e: f32) -> Result<()> {
    let (w, h) = (compact(), island::HEIGHT);
    pill(p, Rect::at(0.0, 0.0, w, h))?;
    let a = component::art::COMPACT;
    mark(p, Rect::at(island::compact::SIDE_MARGIN, (h - a) / 2.0, a, a), a / 2.0)?;
    let tm = island::compact::TEXT_MARGIN;
    text(p, ASK, IslandFont::Compact, w - tm - 60.0, h / 2.0, 60.0, Align::End, ACCENT)
}

fn draw_card(p: &Painter, _e: f32) -> Result<()> {
    use island::expanded as ex;
    p.fill(Rect::at(0.0, 0.0, WIDTH, height()), ex::RADIUS, BLACK)?;
    let a = component::art::EXPANDED;
    mark(p, Rect::at(ex::INSET, ex::INSET, a, a), radius::CONCENTRIC)?;
    // Eyebrow, then the command in a chip, then where it runs.
    let x = ex::INSET + a + ex::COLUMN_GAP;
    let ty = reui_design::tokens::typography::EYEBROW.line_height;
    let code = reui_design::tokens::typography::CODE.line_height;
    let chip_h = code + 2.0 * component::chip::PADDING_Y;
    let detail = reui_design::tokens::typography::DETAIL.line_height;
    let total = ty + chip_h + detail + 2.0 * ex::LINE_GAP;
    let top = ex::INSET + (a - total) / 2.0;
    text(p, "Claude Code", IslandFont::Eyebrow, x, top + ty / 2.0, 200.0, Align::Start, ACCENT)?;
    let command = "npm test";
    let cw = p.measure(command, Font::Island(IslandFont::Code)) + 2.0 * component::chip::PADDING_X;
    let chip = Rect::at(x, top + ty + ex::LINE_GAP, cw, chip_h);
    p.fill(chip, radius::CHIP, color::fill::TERTIARY)?;
    text(p, command, IslandFont::Code, chip.l + component::chip::PADDING_X, chip.mid_y(), cw, Align::Start, color::label::PRIMARY)?;
    text(p, "Bash · reui", IslandFont::Detail, x, chip.b + ex::LINE_GAP + detail / 2.0, 200.0, Align::Start, color::label::SECONDARY)?;
    // Deny / Allow once: equal capsules.
    let bh = component::button::HEIGHT;
    let by = ex::INSET + a + ex::SECTION_GAP;
    let bw = (WIDTH - 2.0 * ex::INSET - component::button::GAP) / 2.0;
    let deny = Rect::at(ex::INSET, by, bw, bh);
    let allow = Rect::at(deny.r + component::button::GAP, by, bw, bh);
    p.fill(deny, bh / 2.0, color::fill::SECONDARY)?;
    text(p, "Deny", IslandFont::Button, deny.l, deny.mid_y(), deny.w(), Align::Center, color::label::PRIMARY)?;
    p.fill(allow, bh / 2.0, ACCENT.with_alpha(opacity::TINT_FILL))?;
    text(p, "Allow once", IslandFont::Button, allow.l, allow.mid_y(), allow.w(), Align::Center, ACCENT)
}
