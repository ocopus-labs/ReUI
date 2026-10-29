//! Icon tiles: a rounded square in the page's colour, lit a little at the top, with a white
//! glyph; ReUI's mark (a light screen with the black island at its top); the Claude logo
//! on a cream tile.

use reui_design::tokens::prefs::{color as c, opacity};
use reui_design::Color;
use windows::core::Result;

use super::{mix, Painter};
use crate::ui::layout::Rect;
use crate::ui::Icon;

const WHITE: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
const BLACK: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
const CREAM: Color = Color { r: 0.96, g: 0.94, b: 0.91, a: 1.0 };

/// Draws `icon` filling `r` with corner `radius`; a glyph is `glyph` DIPs.
pub fn tile(p: &Painter, icon: Icon, r: Rect, radius: f32, glyph: f32) -> Result<()> {
    match icon {
        Icon::Glyph(ch, color) => {
            p.gradient(r, radius, mix(color, WHITE, opacity::TILE_LIGHT), color)?;
            p.glyph(ch, glyph, r, WHITE)
        }
        Icon::Mark => mark(p, r, radius),
        Icon::Claude => {
            p.gradient(r, radius, CREAM, mix(CREAM, BLACK, 0.08))?;
            let inset = r.w() * 0.2;
            p.claude(r.inflate(-inset))
        }
    }
}

/// ReUI's mark: a light screen, the black island resting at its top centre.
pub fn mark(p: &Painter, r: Rect, radius: f32) -> Result<()> {
    p.gradient(r, radius, c::MARK, c::MARK_LOW)?;
    // The island and its bubble, centred together near the top, as in the logo
    // (`tools/logo.ps1`).
    let w = r.w() * 0.44;
    let h = (r.w() * 0.17).max(3.0);
    let gap = (r.w() * 0.05).max(1.0);
    let top = r.t + r.h() * 0.15;
    let l = r.l + (r.w() - (w + gap + h)) / 2.0;
    p.fill(Rect::new(p.snap(l), p.snap(top), p.snap(l + w), p.snap(top + h)), h / 2.0, BLACK)?;
    p.circle(p.snap(l + w + gap) + h / 2.0, p.snap(top) + h / 2.0, h / 2.0, reui_design::tokens::color::system::BLUE)
}
