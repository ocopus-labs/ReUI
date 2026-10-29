//! How big each page's island is, and so how tall its stage: the same maths the island's
//! renderer does for that view (`reui-island/src/render/expanded.rs`, `compact.rs`), from
//! the same tokens and the user's [`Look`]. Pure, so the layout can size the stage before
//! anything is drawn, and tested against the island's own numbers.
//!
//! Sizes are in island units (DIPs before the look's size scales them).

use reui_design::look::Look;
use reui_design::tokens::{component, icon, island, notch, prefs::header, typography};

use super::{Moment, Preview};

/// Width of four level bars.
pub fn bars_width() -> f32 {
    4.0 * component::bars::WIDTH + 3.0 * component::bars::GAP
}

/// The compact pill's width for its sides (each `content + margin`): both as wide as the
/// wider, around the centre gap.
pub fn compact_width(leading: f32, trailing: f32) -> f32 {
    island::compact::CENTER_GAP + 2.0 * leading.max(trailing)
}

/// The Now Playing compact pill: artwork (or its glyph tile) and level bars.
pub fn music_pill() -> f32 {
    compact_width(component::art::COMPACT + island::compact::SIDE_MARGIN, bars_width() + island::compact::TEXT_MARGIN)
}

/// Eyebrow, hero and a one-line detail, with the gaps between them.
pub fn text_height(eyebrow: bool, hero: f32, detail: bool) -> f32 {
    let gap = island::expanded::LINE_GAP;
    let eyebrow = if eyebrow { typography::EYEBROW.line_height + gap } else { 0.0 };
    let detail = if detail { gap + typography::DETAIL.line_height } else { 0.0 };
    eyebrow + hero + detail
}

/// A code hero: the command in its chip.
pub fn code_height() -> f32 {
    typography::CODE.line_height + 2.0 * component::chip::PADDING_Y
}

/// The session strip: its mark or its line, whichever is taller.
pub fn context_height() -> f32 {
    component::art::COMPACT.max(typography::CONTEXT.line_height)
}

/// The header of a card with a 56 leading tile and `trailing` at the right.
fn header(trailing: f32, hero: f32) -> f32 {
    component::art::EXPANDED.max(trailing).max(text_height(true, hero, true))
}

/// A card's height: its content, clamped as the island clamps it.
fn card(look: &Look, natural: f32, roomy: bool) -> f32 {
    let max = look.max_height(if roomy { island::expanded::ROOMY_MAX_HEIGHT } else { island::expanded::MAX_HEIGHT });
    natural.clamp(island::expanded::MIN_HEIGHT, max)
}

/// The island's size for `moment` in `look`, in island units: the shape itself (a second
/// activity beside the compact pill included).
pub fn size(moment: Moment, look: &Look) -> (f32, f32) {
    use island::expanded as ex;
    let inset = look.inset();
    match moment {
        Moment::Compact => {
            let joint = if look.notch() { notch::JOIN_GAP } else { island::minimal::GAP };
            (music_pill() + joint + island::minimal::SIZE, island::HEIGHT)
        }
        Moment::Meter => (island::compact::METER_WIDTH, island::HEIGHT),
        // As wide as its text: measured where it's drawn.
        Moment::Toast => (island::idle::WIDTH, island::HEIGHT),
        Moment::Music => {
            let head = header(component::bars::HEIGHT, typography::HEADLINE.line_height);
            // The timeline with the transport under it (its times at the transport's
            // ends), then the volume.
            let rows = look.section_gap() + component::progress::HEIGHT + reui_design::tokens::space::SM + component::button::TRANSPORT + look.row_gap() + component::slider::HEIGHT;
            (ex::WIDTH, card(look, inset + head + rows + inset, false))
        }
        Moment::Clipboard => {
            // No header: search, tabs, then the cards.
            let rows = component::search::HEIGHT + look.row_gap() + component::tabs::HEIGHT + look.row_gap() + component::card::HEIGHT;
            (ex::WIDE_WIDTH, card(look, inset + rows + inset, false))
        }
        Moment::Devices => (ex::WIDTH, card(look, inset + header(component::ring::EXPANDED, typography::HEADLINE.line_height) + inset, false)),
        Moment::Timer => {
            let head = component::ring::EXPANDED.max(component::button::INLINE).max(text_height(true, typography::VALUE.line_height, true));
            (ex::WIDTH, card(look, inset + head + inset, false))
        }
        Moment::Permission => {
            // The strip, then a glyph and the text beside it, then Deny / Always / Allow
            // sitting like an alert's, a little nearer the bottom edge.
            let head = icon::TILE.max(text_height(true, code_height(), true));
            let natural = inset + context_height() + ex::CONTEXT_GAP + head + look.section_gap() + component::button::HEIGHT + look.padded(component::button::ROOMY_INSET);
            (ex::ROOMY_WIDTH, card(look, natural, true))
        }
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
        Moment::Clipboard if w * preview.look.size > width - 2.0 * header::PADDING => Moment::Toast,
        moment => moment,
    }
}

/// The stage's height: the gap, the island at its real size, and room under it.
pub fn height(preview: Preview, width: f32) -> f32 {
    let (_, h) = size(shown(preview, width), &preview.look);
    (top(&preview.look) + h * preview.look.size + header::STAGE_ROOM).max(header::STAGE_MIN).ceil()
}

#[cfg(test)]
mod tests {
    use super::*;
    use reui_design::look::{Appearance, Padding, Style};

    fn look(a: Appearance) -> Look {
        Look::new(&a, false, None)
    }

    #[test]
    fn todays_island_sizes() {
        let l = Look::default();
        // Inset, search 32, gap, tabs 28, gap, cards 116, inset.
        assert_eq!(size(Moment::Clipboard, &l), (560.0, 20.0 + 32.0 + 12.0 + 28.0 + 12.0 + 116.0 + 20.0));
        assert_eq!(size(Moment::Meter, &l), (200.0, island::HEIGHT));
        // A timer card: the value font's line sets the header.
        let (w, h) = size(Moment::Timer, &l);
        assert_eq!(w, 420.0);
        assert!(h >= island::expanded::MIN_HEIGHT && h < 140.0, "{h}");
        let (w, h) = size(Moment::Permission, &l);
        assert_eq!(w, 480.0);
        assert!(h > size(Moment::Timer, &l).1 && h <= island::expanded::ROOMY_MAX_HEIGHT, "{h}");
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
        assert_eq!(height(compact, 600.0), header::STAGE_MIN.max((island::TOP + island::HEIGHT + header::STAGE_ROOM).ceil()));
    }
}
