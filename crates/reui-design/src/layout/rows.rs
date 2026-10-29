//! The expanded card's full-width rows: buttons, bars with their times, the slider, the
//! search field, choices, text blocks, lyrics and notes. Positions are from the card's
//! left; each row's top is the caller's.

use super::Card;
use crate::look::Look;
use crate::tokens::{component, icon, space, typography};

/// All-glyph rows are transport-style circles centred as a group.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transport {
    /// The first circle's left edge; each next one is `size + gap` further.
    pub left: f32,
    pub size: f32,
    pub gap: f32,
}

pub fn transport(width: f32, n: usize) -> Transport {
    let n = n as f32;
    let (size, gap) = (component::button::TRANSPORT, component::button::TRANSPORT_GAP);
    Transport { left: (width - (n * size + (n - 1.0) * gap)) / 2.0, size, gap }
}

/// The inset a button's corners are concentric with: a roomy card's sit a little nearer
/// the edge, like an alert's.
pub fn button_inset(look: &Look, card: Card) -> f32 {
    if card.roomy() {
        look.padded(component::button::ROOMY_INSET)
    } else {
        look.inset()
    }
}

/// Any text makes a row equal-width capsules across the card.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextButtons {
    pub inset: f32,
    pub width: f32,
    pub gap: f32,
    pub height: f32,
}

impl TextButtons {
    /// Button `i`'s left edge.
    pub fn x(&self, i: usize) -> f32 {
        self.inset + i as f32 * (self.width + self.gap)
    }
}

/// `n` text buttons across a card `width` wide. A roomy card's sit like an alert's: a
/// little nearer the edge, further apart.
pub fn text_buttons(look: &Look, card: Card, width: f32, n: usize) -> TextButtons {
    let (inset, gap) = match card {
        Card::Roomy => (button_inset(look, card), look.padded(component::button::ROOMY_GAP)),
        _ => (look.inset(), component::button::GAP),
    };
    let n = n as f32;
    TextButtons { inset, width: (width - 2.0 * inset - gap * (n - 1.0)) / n, gap, height: component::button::HEIGHT }
}

/// A caption as wide as it can get: every digit counted as the widest (tabular figures
/// make that every digit), so a ticking time doesn't move what's next to it. Measure this.
pub fn caption_widest(text: &str) -> String {
    text.replace(|c: char| c.is_ascii_digit(), "0")
}

/// Do a bar's times (the wider `widest`, from [`caption_widest`]) fit at the ends of a row
/// of `n` transport circles? Then they share its line, under the bar.
pub fn times_fit_beside(look: &Look, width: f32, n: usize, widest: f32) -> bool {
    let n = n as f32;
    let group = n * component::button::TRANSPORT + (n - 1.0).max(0.0) * component::button::TRANSPORT_GAP;
    let free = (width - 2.0 * look.inset() - group) / 2.0;
    widest + space::SM <= free
}

/// Otherwise the times flank the bar on its own line: the bar's span between them (`lead`
/// and `trail` wide).
pub fn bar_between_times(look: &Look, width: f32, lead: f32, trail: f32) -> (f32, f32) {
    let inset = look.inset();
    (inset + lead + space::SM, width - inset - trail - space::SM)
}

/// Space between a bar and the times or controls under it.
pub const UNDER_BAR: f32 = space::SM;

/// Elapsed and remaining times at the card's insets: the leading one left-aligned from
/// `left`, the trailing one right-aligned from `left + half`, each `half` wide.
pub fn captions(look: &Look, width: f32) -> (f32, f32) {
    let inset = look.inset();
    let (l, r) = (inset, width - inset);
    (l, (r - l) / 2.0)
}

/// The slider row (volume): the leading toggle, the bar, the trailing menu button.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slider {
    /// The leading button's left edge; buttons are `size` square.
    pub button: f32,
    pub size: f32,
    /// The bar's ends, and its top from the row's.
    pub bar: (f32, f32),
    pub bar_top: f32,
    /// The menu button's left edge, when there is one.
    pub menu: Option<f32>,
}

pub fn slider(look: &Look, width: f32, menu: bool) -> Slider {
    let (h, gap) = (component::slider::HEIGHT, component::slider::GAP);
    let inset = look.inset();
    let (mut l, mut r) = (inset, width - inset);
    let button = l;
    l += h + gap;
    let menu = menu.then(|| {
        let x = r - h;
        r -= h + gap;
        x
    });
    Slider { button, size: h, bar: (l, r), bar_top: (h - component::progress::HEIGHT) / 2.0, menu }
}

/// A button at the right of the search field: square for a glyph, else its label (`text`
/// wide) padded.
pub fn search_button(text: Option<f32>) -> f32 {
    match text {
        None => component::search::HEIGHT,
        Some(w) => w + 2.0 * component::search::PADDING,
    }
}

/// Where the search field's text starts: after the padding, the glyph and a gap.
pub fn search_text_left(look: &Look) -> f32 {
    look.inset() + component::search::PADDING + icon::SEARCH + space::SM
}

/// A choice list's columns in a card `width` wide: the wells' ends and the text's width
/// (a multi-select list keeps room for its checks).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChoiceColumns {
    pub left: f32,
    pub right: f32,
    pub text_w: f32,
}

pub fn choice_columns(look: &Look, width: f32, multi: bool) -> ChoiceColumns {
    let inset = look.inset();
    let (l, r) = (inset, width - inset);
    let pad = component::choice::PADDING_X;
    let check = component::choice::CHECK;
    let text_r = if multi { r - pad - check - space::SM } else { r - pad };
    ChoiceColumns { left: l, right: r, text_w: text_r - l - pad }
}

/// One choice's detail lines: one, or two when it doesn't fit (`measured` on one line),
/// none without a detail.
pub fn choice_detail_lines(measured: Option<f32>, text_w: f32) -> f32 {
    match measured {
        Some(w) if w > text_w => 2.0,
        Some(_) => 1.0,
        None => 0.0,
    }
}

/// One choice's height: its label, then its detail lines.
pub fn choice_height(detail_lines: f32) -> f32 {
    let detail = detail_lines * typography::CHOICE_DETAIL.line_height;
    2.0 * component::choice::PADDING_Y + typography::CHOICE.line_height + detail
}

/// A list of choices `heights` tall: each one's top in the list, how tall the list shows
/// (at most `choice.max-height`, less `taken`; a longer one scrolls inside the card) and
/// how far it can scroll. `taken`: what the header grew by (a question said over several
/// lines), so the card keeps its height and nothing under the list is pushed out of it;
/// the first choice always shows.
#[derive(Clone, Debug, PartialEq)]
pub struct ChoiceList {
    pub tops: Vec<f32>,
    pub shown: f32,
    pub max_scroll: f32,
}

pub fn choice_list(heights: &[f32], taken: f32) -> ChoiceList {
    let mut tops = Vec::with_capacity(heights.len());
    let mut top = 0.0;
    for (i, h) in heights.iter().enumerate() {
        if i > 0 {
            top += component::choice::GAP;
        }
        tops.push(top);
        top += h;
    }
    let room = (component::choice::MAX_HEIGHT - taken).max(heights.first().copied().unwrap_or(0.0));
    let shown = top.min(room);
    ChoiceList { tops, shown, max_scroll: top - shown }
}

/// A code or diff block of `lines` lines in its well: how much of it shows (whole lines
/// only, so the last one shown is never cut through), the well's height, and how far it
/// can scroll. `taken`: what the header grew by ([`choice_list`]); three lines always show.
pub fn code_block(lines: usize, taken: f32) -> (f32, f32, f32) {
    let line = typography::BLOCK_CODE.line_height;
    let pad = component::block::PADDING;
    let content = lines.max(1) as f32 * line;
    let fits = ((component::block::MAX_HEIGHT - taken - 2.0 * pad) / line).floor().max(3.0);
    let shown = content.min(fits * line);
    (shown, shown + 2.0 * pad, content - shown)
}

/// Prose `content` tall (bare wrapped text): how much shows and how far it can scroll.
/// `taken`: what the header grew by ([`choice_list`]); three lines always show.
pub fn prose_block(content: f32, taken: f32) -> (f32, f32) {
    let room = (component::block::MAX_HEIGHT - taken).max(3.0 * typography::BLOCK_PROSE.line_height);
    let shown = content.min(room);
    (shown, content - shown)
}

/// The lyrics row: always the same height, so lines changing never resize the card.
pub fn lyrics_height() -> f32 {
    let rows = component::lyrics::LINES.max(1.0) as u32;
    typography::LYRIC.line_height * rows as f32
}

/// An agenda of `n` lines (a calendar's rest of the day), each one line of `fact` type.
pub fn agenda_height(n: usize) -> f32 {
    let n = n as f32;
    if n == 0.0 {
        return 0.0;
    }
    n * typography::FACT.line_height + (n - 1.0) * component::agenda::LINE_GAP
}

/// One agenda line's columns across a card `width` wide: the time column `time_w` wide
/// (the widest time, so titles line up), then the title, and the where at the right end,
/// `detail_w` wide but never more than its share of the line. Returns (title left, title
/// width, detail left, detail width); no detail, or no room for one, gives it 0.
pub fn agenda_line(look: &Look, width: f32, time_w: f32, detail_w: f32) -> (f32, f32, f32, f32) {
    let inset = look.inset();
    let title_left = inset + time_w + component::agenda::TIME_GAP;
    let right = width - inset;
    let free = (right - title_left).max(0.0);
    let detail_w = detail_w.min(free * component::agenda::DETAIL_SHARE);
    if detail_w <= 0.0 {
        return (title_left, free, right, 0.0);
    }
    let detail_left = right - detail_w;
    (title_left, (detail_left - space::MD - title_left).max(0.0), detail_left, detail_w)
}

/// A note standing in for empty content. Under a search field it keeps the cards' height,
/// so the panel doesn't jump while typing.
pub fn note_height(searchable: bool) -> f32 {
    if searchable {
        component::card::HEIGHT
    } else {
        typography::NOTE.line_height
    }
}
