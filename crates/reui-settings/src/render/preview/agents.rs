//! Coding agents: Claude Code asking to run a command, as the island draws it now
//! (`reui-hub/src/modules/agent/card.rs`, `describe.rs`; `rows/context.rs`): the session
//! strip (the mark, the project, the prompt and how long the turn has run), the terminal
//! glyph, "Permission" → the command in a chip → what it's for, then Deny / Always / Allow
//! sitting like an alert's, Allow tinted with the agent's accent.

use reui_design::layout::header;
use reui_design::look::Look;
use reui_design::tokens::{component, icon, opacity, typography};
use reui_design::Color;
use windows::core::Result;

use super::parts::{glyph, line, Card, Hero, Role};
use super::rgb;
use crate::render::{Align, Painter};
use crate::ui::layout::{Font, IslandFont, Rect};
use crate::ui::stage::{self, context_height};
use crate::ui::Moment;

/// Claude's own orange: the activity's accent.
const ACCENT: Color = rgb(217, 119, 87);
const TERMINAL: char = '\u{E756}';

pub fn draw(p: &Painter, look: &Look) -> Result<()> {
    let open = stage::card(Moment::Permission, look).expect("an open card");
    let c = Card::new(p, look, ACCENT, open.card);
    let width = c.width;
    c.shape(open.height)?;
    let pal = &look.palette;
    let inset = look.inset();
    // The session strip: the mark bare (its own colours), the project, then the facts.
    let strip = context_height();
    let mid = inset + strip / 2.0;
    let a = component::art::COMPACT;
    let mark = a * opacity::LOGO_BARE;
    p.claude(Rect::at(inset + (a - mark) / 2.0, mid - mark / 2.0, mark, mark))?;
    let cl = typography::CONTEXT.line_height;
    let x = header::context_left(look, a);
    let title = "reui";
    let tw = p.measure(title, Font::Island(IslandFont::Context));
    line(p, title, IslandFont::Context, x, mid - cl / 2.0, tw + 1.0, Align::Start, pal.label_secondary)?;
    line(p, " · Run the tests and fix what fails · 4m", IslandFont::Context, x + tw, mid - cl / 2.0, width - inset - x - tw, Align::Start, pal.label_tertiary)?;
    // The header: the glyph on the first line's centre, the text from the top.
    let g = icon::TILE;
    let (left, text_w) = c.columns(g, 0.0);
    let (gmid, text_top) = (open.header.mid, open.header.text_top);
    glyph(p, TERMINAL, g, inset, gmid - g / 2.0, ACCENT)?;
    c.text(&open.text, left, text_top, text_w, "Permission", Hero::Code("npm test"), "Run the test suite")?;
    c.text_buttons(&[("Deny", Role::Secondary), ("Always", Role::Secondary), ("Allow", Role::Accent)], open.rows)
}
