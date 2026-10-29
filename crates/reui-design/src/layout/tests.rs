//! The layout against the island's own numbers: whole cards built the way the island
//! builds them (and Settings' previews size their stages), then the pieces.

use super::header::{self, Hero, Text};
use super::{compact, rows, row_gap, Card};
use crate::look::{Appearance, Look, Padding, Style};
use crate::tokens::{component, icon, island, typography};

fn look(a: Appearance) -> Look {
    Look::new(&a, false, None)
}

/// The clipboard panel: no header; search, tabs, then the cards.
fn panel(l: &Look) -> (f32, f32) {
    let mut y = l.inset();
    for (i, h) in [component::search::HEIGHT, component::tabs::HEIGHT, component::card::HEIGHT].into_iter().enumerate() {
        y += row_gap(l, i, true) + h;
    }
    (Card::Wide.width(), Card::Wide.height(l, y + Card::Wide.bottom(l, false)))
}

/// A running timer: the ring, value hero, inline pause and cancel.
fn timer(l: &Look) -> (f32, f32) {
    let text = Text::new(true, Hero::Value, 1.0).height();
    let head = header::header(l.inset(), component::ring::EXPANDED, component::button::INLINE, text, false);
    (Card::Regular.width(), Card::Regular.height(l, l.inset() + head.height + Card::Regular.bottom(l, false)))
}

/// A permission: the session strip, a glyph and the code hero, then three text buttons.
fn permission(l: &Look) -> (f32, f32) {
    let top = l.inset() + header::context_height(component::art::COMPACT) + island::expanded::CONTEXT_GAP;
    let text = Text::new(true, Hero::Code, 1.0).height();
    let head = header::header(top, icon::TILE, 0.0, text, false);
    let y = top + head.height + row_gap(l, 0, false) + component::button::HEIGHT;
    (Card::Roomy.width(), Card::Roomy.height(l, y + Card::Roomy.bottom(l, true)))
}

#[test]
fn todays_island_sizes() {
    let l = Look::default();
    // Inset, search 32, gap, tabs 28, gap, cards 116, inset.
    assert_eq!(panel(&l), (560.0, 20.0 + 32.0 + 12.0 + 28.0 + 12.0 + 116.0 + 20.0));
    assert_eq!(compact::meter(&l, 0.5).width, 200.0);
    // A timer card: the value font's line sets the header.
    let (w, h) = timer(&l);
    assert_eq!(w, 420.0);
    assert!(h >= island::expanded::MIN_HEIGHT && h < 140.0, "{h}");
    let (w, h) = permission(&l);
    assert_eq!(w, 480.0);
    assert!(h > timer(&l).1 && h <= island::expanded::ROOMY_MAX_HEIGHT, "{h}");
}

#[test]
fn roomy_padding_grows_cards_and_their_tallest() {
    let roomy = look(Appearance { padding: Padding::Roomy, ..Appearance::default() });
    let l = Look::default();
    assert!(panel(&roomy).1 > panel(&l).1);
    assert!(permission(&roomy).1 > permission(&l).1);
    assert!(Card::Roomy.max_height(&roomy) > Card::Roomy.max_height(&l));
    assert_eq!(Card::Regular.height(&l, 10.0), island::expanded::MIN_HEIGHT, "never under the minimum");
    assert_eq!(Card::Regular.height(&l, 9999.0), Card::Regular.max_height(&l));
    assert_eq!(super::expanded_surface(&l), (island::expanded::WIDE_WIDTH, island::expanded::ROOMY_MAX_HEIGHT));
}

#[test]
fn the_compact_pill_balances_its_sides() {
    let art = component::art::COMPACT;
    let bars = compact::bars_width();
    let s = compact::sides(art, compact::margin(true), bars, compact::margin(false));
    assert_eq!(s.width, island::compact::CENTER_GAP + 2.0 * (art + island::compact::SIDE_MARGIN).max(bars + island::compact::TEXT_MARGIN));
    assert_eq!(s.leading, island::compact::SIDE_MARGIN);
    assert_eq!(s.width - s.trailing, island::compact::TEXT_MARGIN);
    assert_eq!(compact::left(compact::MAX_WIDTH), 0.0);
    // A label sits beside its mark; a short one keeps the resting width.
    let j = compact::joined(art, island::compact::SIDE_MARGIN, 10.0);
    assert_eq!(j.width, island::idle::WIDTH);
    assert_eq!(j.text_x, island::compact::SIDE_MARGIN + art + island::compact::JOINED_GAP);
    let long = compact::joined(art, island::compact::SIDE_MARGIN, 1000.0);
    assert_eq!(long.text_w, island::compact::JOINED_TEXT);
    assert_eq!(long.width, long.text_x + long.text_w + island::compact::TEXT_MARGIN);
}

#[test]
fn the_meter_fills_from_its_round_end() {
    let l = Look::default();
    assert_eq!(compact::meter(&l, 0.0).fill, None);
    let inner = island::HEIGHT - 2.0 * island::compact::METER_INSET;
    assert_eq!(compact::meter(&l, 0.001).fill, Some(inner), "on but quiet still shows its end");
    let full = compact::meter(&l, 1.0);
    assert_eq!(full.fill, Some(full.width - 2.0 * full.inset));
    assert!(full.corner <= inner / 2.0);
    let notch = look(Appearance { style: Style::Notch, ..Appearance::default() });
    assert_eq!(compact::bubble_gap(&notch), crate::tokens::notch::JOIN_GAP);
    assert_eq!(compact::bubble_gap(&l), island::minimal::GAP);
}

#[test]
fn the_header_centres_short_text_on_the_tile() {
    let l = Look::default();
    let text = Text::new(true, Hero::Text, 1.0);
    assert_eq!(text.height(), typography::EYEBROW.line_height + typography::HEADLINE.line_height + typography::DETAIL.line_height + 2.0 * island::expanded::LINE_GAP);
    assert_eq!(text.first_line(), typography::EYEBROW.line_height);
    let tile = component::art::EXPANDED;
    let h = header::header(l.inset(), tile, 0.0, 30.0, false);
    assert_eq!((h.height, h.mid, h.text_top), (tile, l.inset() + tile / 2.0, l.inset() + tile / 2.0 - 15.0));
    // Taller than the tile: from the top.
    let h = header::header(l.inset(), 20.0, 0.0, 60.0, false);
    assert_eq!((h.height, h.text_top), (60.0, l.inset()));
    assert_eq!(header::header(0.0, 0.0, 0.0, 0.0, true).height, 0.0);
    // Columns beside what's there.
    let (left, w) = header::columns(&l, 420.0, tile, 0.0);
    assert_eq!(left, l.inset() + tile + l.column_gap());
    assert_eq!(left + w, 420.0 - l.inset());
    assert_eq!(header::inline_x(&l, 420.0, 2, 0) + header::inline_width(2), 420.0 - l.inset());
    assert_eq!(header::detail_lines(Some(500.0), true, 300.0), 2.0);
    assert_eq!(header::detail_lines(Some(500.0), false, 300.0), 1.0);
    assert_eq!(header::detail_lines(None, true, 300.0), 0.0);
}

#[test]
fn button_rows_fill_the_card() {
    let l = Look::default();
    let b = rows::text_buttons(&l, Card::Regular, 420.0, 3);
    assert!((b.x(2) + b.width - (420.0 - l.inset())).abs() < 1e-3);
    let roomy = rows::text_buttons(&l, Card::Roomy, 480.0, 3);
    assert!(roomy.inset <= l.inset() && roomy.gap >= component::button::GAP, "like an alert");
    let t = rows::transport(420.0, 3);
    assert_eq!(t.left * 2.0 + 3.0 * t.size + 2.0 * t.gap, 420.0);
    assert!(rows::times_fit_beside(&l, 420.0, 3, 30.0));
    assert!(!rows::times_fit_beside(&l, 420.0, 3, 300.0));
    assert_eq!(rows::caption_widest("-3:41"), "-0:00");
    let s = rows::slider(&l, 420.0, true);
    assert_eq!(s.menu, Some(420.0 - l.inset() - s.size));
    assert_eq!(s.bar.1, 420.0 - l.inset() - s.size - component::slider::GAP);
}

#[test]
fn long_lists_and_blocks_scroll_inside_the_card() {
    let h = rows::choice_height(1.0);
    let list = rows::choice_list(&[h; 12]);
    assert_eq!(list.shown, component::choice::MAX_HEIGHT);
    assert!(list.max_scroll > 0.0);
    assert_eq!(list.tops[1], h + component::choice::GAP);
    let short = rows::choice_list(&[h, h]);
    assert_eq!((short.shown, short.max_scroll), (2.0 * h + component::choice::GAP, 0.0));
    let (shown, well, max) = rows::code_block(200);
    assert!(well <= component::block::MAX_HEIGHT && max > 0.0);
    assert_eq!(shown % typography::BLOCK_CODE.line_height, 0.0, "whole lines");
    assert_eq!(rows::code_block(0).2, 0.0);
}
