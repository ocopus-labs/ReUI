//! Coding agents: Claude Code asking to run a command, as the island draws it now
//! (`reui-hub/src/modules/agent/card.rs`, `describe.rs`; `rows/context.rs`): the session
//! strip (the mark, the project, the prompt and how long the turn has run), the terminal
//! glyph, "Permission" → the command in a chip → what it's for, then Deny / Always / Allow
//! sitting like an alert's, Allow tinted with the agent's accent.

use reui_design::look::Look;
use reui_design::tokens::{component, icon, island, opacity, space, typography};
use reui_design::Color;
use windows::core::Result;

use super::parts::{glyph, line, Card, Hero, Role};
use super::rgb;
use crate::render::{Align, Painter};
use crate::ui::layout::{Font, IslandFont, Rect};
use crate::ui::stage::{self, code_height, context_height, text_height};
use crate::ui::Moment;

/// Claude's own orange: the activity's accent.
const ACCENT: Color = rgb(217, 119, 87);
const TERMINAL: char = '\u{E756}';

pub fn draw(p: &Painter, look: &Look) -> Result<()> {
    let (width, h) = stage::size(Moment::Permission, look);
    let c = Card { p, look, accent: ACCENT, width, radius: look.radius(), roomy: true };
    c.shape(h)?;
    let pal = &look.palette;
    let inset = look.inset();
    // The session strip: the mark bare (its own colours), the project, then the facts.
    let strip = context_height();
    let mid = inset + strip / 2.0;
    let a = component::art::COMPACT;
    let mark = a * opacity::LOGO_BARE;
    p.claude(Rect::at(inset + (a - mark) / 2.0, mid - mark / 2.0, mark, mark))?;
    let cl = typography::CONTEXT.line_height;
    let x = inset + a + space::SM;
    let title = "reui";
    let tw = p.measure(title, Font::Island(IslandFont::Context));
    line(p, title, IslandFont::Context, x, mid - cl / 2.0, tw + 1.0, Align::Start, pal.label_secondary)?;
    line(p, " · Run the tests and fix what fails · 4m", IslandFont::Context, x + tw, mid - cl / 2.0, width - inset - x - tw, Align::Start, pal.label_tertiary)?;
    // The header: the glyph on the first line's centre, the text from the top.
    let top = inset + strip + island::expanded::CONTEXT_GAP;
    let g = icon::TILE;
    let (left, text_w) = c.columns(g, 0.0);
    let text_h = text_height(true, code_height(), true);
    let gmid = top + g / 2.0;
    let text_top = if text_h <= g { gmid - text_h / 2.0 } else { top };
    glyph(p, TERMINAL, g, inset, gmid - g / 2.0, ACCENT)?;
    c.text(left, text_top, text_w, "Permission", Hero::Code("npm test"), "Run the test suite")?;
    let y = top + g.max(text_h) + look.section_gap();
    c.text_buttons(&[("Deny", Role::Secondary), ("Always", Role::Secondary), ("Allow", Role::Accent)], y)
}
