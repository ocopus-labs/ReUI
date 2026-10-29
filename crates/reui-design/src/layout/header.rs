//! The expanded card's header: the leading element, the text column (eyebrow → hero →
//! detail), the trailing element or inline glyph buttons; above it the session strip, and
//! the source switcher on the first line. Positions are from the card's left and top.

use crate::look::Look;
use crate::tokens::{component, icon, island, space, typography};

/// What the headline is, which sets the hero line's height.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hero {
    /// A number that changes in place (a timer, a clock): the value font.
    Value,
    Text,
    /// A command in a chip.
    Code,
    /// No hero line: a row under the header is the hero (System stats' gauges), so the
    /// header is only its eyebrow.
    None,
}

impl Hero {
    pub fn height(self) -> f32 {
        match self {
            Hero::Value => typography::VALUE.line_height,
            Hero::Text => typography::HEADLINE.line_height,
            Hero::Code => typography::CODE.line_height + 2.0 * component::chip::PADDING_Y,
            Hero::None => 0.0,
        }
    }
}

/// The text column's lines, top to bottom, each with the gap above the next included in
/// the line before it (so absent lines take no room).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Text {
    /// The eyebrow and the gap under it, or 0.
    pub eyebrow: f32,
    pub hero: f32,
    /// The gap above the detail and its lines, or 0.
    pub detail: f32,
}

impl Text {
    /// `detail_lines`: 0 without a detail, else 1, or 2 when it wraps ([`detail_lines`]).
    pub fn new(eyebrow: bool, hero: Hero, detail_lines: f32) -> Self {
        let gap = island::expanded::LINE_GAP;
        // The gap under the eyebrow only when a line follows it.
        let under = if hero != Hero::None || detail_lines > 0.0 { gap } else { 0.0 };
        Text {
            eyebrow: if eyebrow { typography::EYEBROW.line_height + under } else { 0.0 },
            hero: hero.height(),
            detail: if detail_lines > 0.0 { gap + detail_lines * typography::DETAIL.line_height } else { 0.0 },
        }
    }

    /// A text hero that wraps to `lines` lines ([`hero_lines`]).
    pub fn with_hero_lines(mut self, lines: f32) -> Self {
        if lines > 1.0 {
            self.hero = lines * typography::HEADLINE.line_height;
        }
        self
    }

    pub fn height(&self) -> f32 {
        self.eyebrow + self.hero + self.detail
    }

    /// The first line's height, where the source switcher is centred.
    pub fn first_line(&self) -> f32 {
        if self.eyebrow > 0.0 {
            typography::EYEBROW.line_height
        } else {
            self.hero
        }
    }
}

/// How many lines the detail takes: a roomy card's (an agent's own description) wraps to a
/// second line rather than losing its end; elsewhere it's one. `measured`: its width on one
/// line, `None` without a detail.
pub fn detail_lines(measured: Option<f32>, roomy: bool, text_w: f32) -> f32 {
    match measured {
        Some(w) if roomy && w > text_w => 2.0,
        Some(_) => 1.0,
        None => 0.0,
    }
}

/// A roomy card's text hero (an agent's question, a plan's title) is said in full: it wraps
/// to at most this many lines rather than being cut after one.
pub const HERO_MAX_LINES: u32 = 3;

/// How many lines a text hero takes, given how many it wraps to in its column: one outside
/// roomy cards (a song title, a device name: cut with an ellipsis as before).
pub fn hero_lines(wrapped: u32, roomy: bool) -> f32 {
    if roomy {
        wrapped.clamp(1, HERO_MAX_LINES) as f32
    } else {
        1.0
    }
}

/// The text column beside a leading element `lead` wide and a trailing one `trail` wide
/// (either 0 when absent) in a card `width` wide: its left edge and width.
pub fn columns(look: &Look, width: f32, lead: f32, trail: f32) -> (f32, f32) {
    let (inset, gap) = (look.inset(), look.column_gap());
    let left = inset + if lead > 0.0 { lead + gap } else { 0.0 };
    let right = width - inset - if trail > 0.0 { trail + gap } else { 0.0 };
    (left, right - left)
}

/// A lone row of glyph buttons belongs beside the hero, not under it: that row's width.
pub fn inline_width(n: usize) -> f32 {
    let n = n as f32;
    n * component::button::INLINE + (n - 1.0) * component::button::INLINE_GAP
}

/// A lone text capsule beside a moment's text (a failed turn's Open terminal): its width
/// for a label `label` wide. It's `component::button::INLINE` tall, at the right inset.
pub fn inline_text_width(label: f32) -> f32 {
    label + 2.0 * component::button::INLINE_TEXT_PADDING
}

/// Inline glyph button `i`'s left edge in a card `width` wide holding `n` of them.
pub fn inline_x(look: &Look, width: f32, n: usize, i: usize) -> f32 {
    let mut x = width - look.inset() - inline_width(n);
    for _ in 0..i {
        x += component::button::INLINE + component::button::INLINE_GAP;
    }
    x
}

/// A bare glyph button at the header's end (`Element::Button`, System stats' Task
/// Manager): the glyph is `icon.header`, so it never makes an eyebrow-only header taller,
/// and its click target is a square `header-button.hit` wide around it, centred on the
/// glyph at the card's right inset and on `mid`. Returns the target's (left, top, size).
pub fn end_button(look: &Look, width: f32, mid: f32) -> (f32, f32, f32) {
    let hit = component::header_button::HIT;
    let cx = width - look.inset() - icon::HEADER / 2.0;
    (cx - hit / 2.0, mid - hit / 2.0, hit)
}

/// Where the header's pieces sit vertically.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Header {
    pub height: f32,
    /// The leading and trailing elements' shared centre line.
    pub mid: f32,
    /// The text column's top.
    pub text_top: f32,
}

/// The header from `top`, with a leading element `lead` tall, a trailing one `trail` tall
/// (0 when absent) and a text column `text` tall. The leading tile sits one inset from the
/// top as from the left, and what's at the right shares its centre line; text is centred
/// on it when it fits beside it and starts at the top when it's taller. A view of only rows
/// (`headless`, e.g. clipboard cards) has no header at all.
pub fn header(top: f32, lead: f32, trail: f32, text: f32, headless: bool) -> Header {
    let height = if headless { 0.0 } else { lead.max(trail).max(text) };
    if lead > 0.0 {
        let mid = top + lead.max(trail) / 2.0;
        Header { height, mid, text_top: if text <= lead { mid - text / 2.0 } else { top } }
    } else {
        let mid = top + height / 2.0;
        Header { height, mid, text_top: mid - text / 2.0 }
    }
}

/// The session strip's height: its mark (`mark` tall, compact-sized) or its line,
/// whichever is taller.
pub fn context_height(mark: f32) -> f32 {
    mark.max(typography::CONTEXT.line_height)
}

/// The strip's title starts after the mark (`mark` wide, 0 without one).
pub fn context_left(look: &Look, mark: f32) -> f32 {
    look.inset() + if mark > 0.0 { mark + space::SM } else { 0.0 }
}

/// The strip's facts show only when at least this much room is left after the title.
pub const CONTEXT_FACTS_MIN: f32 = space::LG;

/// The source switcher "‹ 1/2 ›": two chevron targets around its label.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pager {
    pub prev: f32,
    pub label: f32,
    pub next: f32,
    /// Each chevron's target, square.
    pub hit: f32,
}

/// The switcher ending at `right`, its label `label` wide.
pub fn pager(right: f32, label: f32) -> Pager {
    let (hit, gap) = (component::pager::HIT, component::pager::GAP);
    let next = right - hit;
    let label_l = next - gap - label;
    Pager { prev: label_l - gap - hit, label: label_l, next, hit }
}
