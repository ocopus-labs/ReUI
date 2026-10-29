//! The view switcher under an open place's card (docs/PRD-view-switcher.md), as the island
//! draws it: a capsule of the island's material `island.switcher.gap` under the card,
//! glyph tabs one inset in, the open place's tab on the soft fill. Where it all goes is
//! `reui_design::layout::switcher`, as in the island. Drawn under the timer's card (a
//! running timer is the Timers place) while Settings → General → View switcher is on.

use reui_design::layout::switcher::{Switcher, TAB};
use reui_design::look::Look;
use reui_design::tokens::{icon, island};
use windows::core::Result;

use super::parts::glyph;
use crate::render::Painter;
use crate::ui::layout::Rect;

/// Clipboard, Timers, Devices, System stats: the hub's places in its order (Now Playing
/// joins only while a player has a card, and the timer's stage has none).
const PLACES: [char; 4] = ['\u{E77F}', '\u{E916}', '\u{E772}', '\u{E9D9}'];
/// Timers: the open card's place.
const SELECTED: usize = 1;

/// Under a card `card` wide and `height` tall whose top-left is the origin.
pub fn draw(p: &Painter, look: &Look, card: f32, height: f32) -> Result<()> {
    let s = Switcher::new(look, PLACES.len());
    let (x, y) = (((card - s.width) / 2.0).round(), Switcher::top(0.0, height));
    let pal = &look.palette;
    // Floating even under a notch: it isn't on the screen's edge.
    let r = Rect::at(x, y, s.width, s.height);
    p.fill(r, s.radius, pal.background)?;
    if pal.hairline.a > 0.0 {
        p.edge(r, s.radius, pal.hairline)?;
    }
    let top = y + island::switcher::INSET;
    let g = icon::COMPACT;
    for (i, (&left, &mark)) in s.tabs.iter().zip(PLACES.iter()).enumerate() {
        let selected = i == SELECTED;
        if selected {
            p.fill(Rect::at(x + left, top, TAB, TAB), s.tab_radius, pal.fill_secondary)?;
        }
        let ink = if selected { pal.label_primary } else { pal.label_secondary };
        glyph(p, mark, g, x + left + (TAB - g) / 2.0, top + (TAB - g) / 2.0, ink)?;
    }
    Ok(())
}
