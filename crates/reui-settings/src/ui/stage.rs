//! How big each page's island is, and so how tall its stage; and, for each open card, where
//! its header and rows go. The maths is the island's own (`reui_design::layout`, which
//! the island's renderer lays out with too), fed the content each preview shows. Pure, so
//! the layout can size the stage before anything is drawn.
//!
//! Sizes are in island units (DIPs before the look's size scales them).

use reui_design::layout::compact;
use reui_design::layout::header::{self, Header, Hero, Text};
use reui_design::layout::{row_gap, rows, Card};
use reui_design::look::Look;
use reui_design::tokens::{component, icon, island, prefs::header as stage_header};

use super::{Moment, Preview};

/// The Now Playing compact pill: artwork (or its glyph tile) and level bars.
pub fn music_pill() -> compact::Sides {
    compact::sides(component::art::COMPACT, compact::margin(true), compact::bars_width(), compact::margin(false))
}

/// An open card as a preview draws it: its size, its header, and where its rows start.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Open {
    pub card: Card,
    pub height: f32,
    /// The text column's lines.
    pub text: Text,
    pub header: Header,
    /// The first row's top (under the header and its section gap).
    pub rows: f32,
}

/// A card whose header starts at `top`: a leading element and a trailing one (`ends`: their
/// heights, 0 when absent) around the text column, then rows `rows.0` tall in all (none
/// when 0), the last of them text buttons when `rows.1`.
fn open(look: &Look, card: Card, top: f32, ends: (f32, f32), text: Text, rows: (f32, bool)) -> Open {
    let (rows_h, text_buttons_last) = rows;
    let header = header::header(top, ends.0, ends.1, text.height(), false);
    let first = top + header.height + row_gap(look, 0, false);
    let natural = if rows_h > 0.0 { first + rows_h } else { top + header.height };
    let height = card.height(look, natural + card.bottom(look, text_buttons_last));
    Open { card, height, text, header, rows: first }
}

/// The open card a moment shows, laid out; `None` for the compact moments and the panel.
pub fn card(moment: Moment, look: &Look) -> Option<Open> {
    let inset = look.inset();
    let line = Text::new(true, Hero::Text, 1.0);
    Some(match moment {
        // The 56 tile and the bars; the timeline with the transport under it (its times at
        // the transport's ends), then the volume.
        Moment::Music => {
            let rows_h = component::progress::HEIGHT + rows::UNDER_BAR + component::button::TRANSPORT + row_gap(look, 1, false) + component::slider::HEIGHT;
            open(look, Card::Regular, inset, (component::art::EXPANDED, component::bars::HEIGHT), line, (rows_h, false))
        }
        // The tile and the battery's ring.
        Moment::Devices => open(look, Card::Regular, inset, (component::art::EXPANDED, component::ring::EXPANDED), line, (0.0, false)),
        // The ring and the value, pause and cancel inline at the right.
        Moment::Timer => open(look, Card::Regular, inset, (component::ring::EXPANDED, component::button::INLINE), Text::new(true, Hero::Value, 1.0), (0.0, false)),
        // The strip, then a glyph and the code hero, then Deny / Always / Allow sitting like
        // an alert's, a little nearer the bottom edge.
        Moment::Permission => {
            let top = inset + context_height() + island::expanded::CONTEXT_GAP;
            open(look, Card::Roomy, top, (icon::TILE, 0.0), Text::new(true, Hero::Code, 1.0), (component::button::HEIGHT, true))
        }
        Moment::Compact | Moment::Meter | Moment::Toast | Moment::Clipboard => return None,
    })
}

/// The session strip: its mark (compact art) or its line, whichever is taller.
pub fn context_height() -> f32 {
    header::context_height(component::art::COMPACT)
}

/// The clipboard panel's rows from the top: search, tabs, then the cards (no header).
pub fn panel_rows(look: &Look) -> [f32; 3] {
    let mut y = look.inset();
    let mut tops = [0.0; 3];
    for (i, h) in [component::search::HEIGHT, component::tabs::HEIGHT, component::card::HEIGHT].into_iter().enumerate() {
        y += row_gap(look, i, true);
        tops[i] = y;
        y += h;
    }
    tops
}

/// The island's size for `moment` in `look`, in island units: the shape itself (a second
/// activity beside the compact pill included).
pub fn size(moment: Moment, look: &Look) -> (f32, f32) {
    match moment {
        Moment::Compact => (music_pill().width + compact::bubble_gap(look) + island::minimal::SIZE, island::HEIGHT),
        Moment::Meter => (compact::meter(look, 0.0).width, island::HEIGHT),
        // As wide as its text: measured where it's drawn.
        Moment::Toast => (island::idle::WIDTH, island::HEIGHT),
        Moment::Clipboard => {
            let card = Card::Wide;
            let bottom = panel_rows(look)[2] + component::card::HEIGHT;
            (card.width(), card.height(look, bottom + card.bottom(look, false)))
        }
        _ => card(moment, look).map_or((0.0, 0.0), |c| (c.card.width(), c.height)),
    }
}

/// From the stage's top edge (the screen's) to the island's top: the gap, or none for a
/// notch. In Settings DIPs.
pub fn top(look: &Look) -> f32 {
    look.top() * look.size
}

/// What a stage `width` wide shows: the page's moment, or for the clipboard panel when
/// it's wider than the stage at the look's size (the island is never shrunk to fit), the
/// toast a copy shows.
pub fn shown(preview: Preview, width: f32) -> Moment {
    let (w, _) = size(preview.moment, &preview.look);
    match preview.moment {
        Moment::Clipboard if w * preview.look.size > width - 2.0 * stage_header::PADDING => Moment::Toast,
        moment => moment,
    }
}

/// The stage's height: the gap, the island at its real size, and room under it.
pub fn height(preview: Preview, width: f32) -> f32 {
    let (_, h) = size(shown(preview, width), &preview.look);
    (top(&preview.look) + h * preview.look.size + stage_header::STAGE_ROOM).max(stage_header::STAGE_MIN).ceil()
}

#[cfg(test)]
mod tests {
    use super::*;
    use reui_design::look::{Appearance, Padding, Style};

    fn look(a: Appearance) -> Look {
        Look::new(&a, false, None)
    }

    // The island's own sizes are pinned in `reui_design::layout`'s tests; these check that
    // each page asks for the card it shows.
    #[test]
    fn each_moment_is_its_cards_size() {
        let l = Look::default();
        assert_eq!(size(Moment::Clipboard, &l), (560.0, 20.0 + 32.0 + 12.0 + 28.0 + 12.0 + 116.0 + 20.0));
        assert_eq!(size(Moment::Meter, &l), (200.0, island::HEIGHT));
        assert_eq!(size(Moment::Timer, &l).0, 420.0);
        assert_eq!(size(Moment::Permission, &l).0, 480.0);
        assert!(size(Moment::Permission, &l).1 > size(Moment::Timer, &l).1);
        let music = card(Moment::Music, &l).unwrap();
        assert_eq!(music.rows, l.inset() + music.header.height + l.section_gap());
    }

    #[test]
    fn padding_and_size_grow_the_stage() {
        let regular = Preview { moment: Moment::Music, look: Look::default() };
        let roomy = Preview { moment: Moment::Music, look: look(Appearance { padding: Padding::Roomy, ..Appearance::default() }) };
        let big = Preview { moment: Moment::Music, look: look(Appearance { size: 1.25, ..Appearance::default() }) };
        assert!(height(roomy, 600.0) > height(regular, 600.0));
        assert!(height(big, 600.0) > height(regular, 600.0));
    }

    #[test]
    fn a_panel_too_wide_for_the_stage_shows_the_toast() {
        let clipboard = |size: f32| Preview { moment: Moment::Clipboard, look: look(Appearance { size, ..Appearance::default() }) };
        assert_eq!(shown(clipboard(1.0), 640.0), Moment::Clipboard);
        assert_eq!(shown(clipboard(1.25), 640.0), Moment::Toast);
        assert!(height(clipboard(1.25), 640.0) < height(clipboard(1.0), 640.0));
    }

    #[test]
    fn a_notch_starts_at_the_edge_and_a_floating_island_below_it() {
        let notch = look(Appearance { style: Style::Notch, top_gap: 20.0, ..Appearance::default() });
        assert_eq!(top(&notch), 0.0);
        let floating = look(Appearance { top_gap: 20.0, size: 1.25, ..Appearance::default() });
        assert_eq!(top(&floating), 20.0, "the gap is in DIPs at any size");
        let compact = Preview { moment: Moment::Meter, look: Look::default() };
        assert_eq!(height(compact, 600.0), stage_header::STAGE_MIN.max((island::TOP + island::HEIGHT + stage_header::STAGE_ROOM).ceil()));
    }
}
