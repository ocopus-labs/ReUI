//! Gauges side by side (System stats' CPU and Memory) and the line of facts under them.
//!
//! A gauge is a column: a ring with a glyph in it beside its text (label → numerals →
//! detail), then the last minute as a sparkline across the whole column. Gauges in a row
//! share one height (a detail on either reserves the line on both), so their labels,
//! numerals and lines sit level. Facts are short label-over-value pairs spread across the
//! card, the first at the left inset and the last ending at the right one.
//!
//! Positions are from the card's left and the row's top.

use super::header;
use crate::look::Look;
use crate::tokens::{component, island, space, typography};

/// `n` gauges across a card `width` wide: each column's left edge and their shared width.
pub fn columns(look: &Look, width: f32, n: usize) -> (Vec<f32>, f32) {
    let n = n.max(1);
    let (inset, gap) = (look.inset(), look.padded(component::gauge::GAP));
    let w = (width - 2.0 * inset - gap * (n - 1) as f32) / n as f32;
    ((0..n).map(|i| inset + i as f32 * (w + gap)).collect(), w)
}

/// One gauge's pieces, from its column's left and the row's top.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gauge {
    /// The ring's centre.
    pub ring: (f32, f32),
    pub text_left: f32,
    pub text_w: f32,
    /// Each text line's top: the label, the numerals (and their unit), the detail.
    pub label: f32,
    pub value: f32,
    pub detail: f32,
    /// The sparkline's top; it is `component::gauge::SPARK_HEIGHT` tall, the column wide.
    pub spark: f32,
    /// The whole row's height.
    pub height: f32,
}

/// A gauge in a column `col_w` wide; `detail`: whether any gauge in the row has a detail
/// line (they all keep its room, so the row reads level).
pub fn gauge(look: &Look, col_w: f32, detail: bool) -> Gauge {
    let ring = component::ring::EXPANDED;
    let gap = island::expanded::LINE_GAP;
    let label = typography::GAUGE_LABEL.line_height;
    let value = typography::VALUE.line_height;
    let text = label + gap + value + if detail { gap + typography::DETAIL.line_height } else { 0.0 };
    let body = ring.max(text);
    let text_top = (body - text) / 2.0;
    let text_left = ring + look.column_gap();
    let spark = body + look.padded(component::gauge::SPARK_GAP);
    Gauge {
        ring: (ring / 2.0, body / 2.0),
        text_left,
        text_w: (col_w - text_left).max(0.0),
        label: text_top,
        value: text_top + label + gap,
        detail: text_top + label + gap + value + gap,
        spark,
        height: spark + component::gauge::SPARK_HEIGHT,
    }
}

/// A sparkline's points from `history` (oldest first, 0..=1, the last is now) in a box
/// from `left` to `right` and `top`, `SPARK_HEIGHT` tall, holding at most `capacity`
/// points a step apart. The newest is at the right end and older ones run left, so a
/// minute that has only just started is a short line at the right: nothing is drawn for
/// time that wasn't measured. The ends stay half a dot inside the box (the newest point
/// is marked with one), and 0 sits on the zero line at the bottom.
pub fn spark_points(history: &[f32], capacity: usize, left: f32, right: f32, top: f32) -> Vec<(f32, f32)> {
    let capacity = capacity.max(2);
    let shown = &history[history.len().saturating_sub(capacity)..];
    let dot = component::gauge::SPARK_DOT / 2.0;
    let (l, r) = (left + dot, right - dot);
    let step = (r - l) / (capacity - 1) as f32;
    let (high, zero) = (top + dot, spark_zero(top));
    let n = shown.len();
    shown.iter().enumerate().map(|(i, v)| (r - (n - 1 - i) as f32 * step, zero - v.clamp(0.0, 1.0) * (zero - high))).collect()
}

/// The sparkline's zero line, for a box whose top is `top`: the track sits on it, the area
/// under the line ends there.
pub fn spark_zero(top: f32) -> f32 {
    top + component::gauge::SPARK_HEIGHT - component::gauge::SPARK_TRACK / 2.0
}

/// A line of facts: a label over its value.
pub fn facts_height() -> f32 {
    typography::FACT_LABEL.line_height + component::fact::LINE_GAP + typography::FACT.line_height
}

/// Where each fact's value starts under its label.
pub fn fact_value_top() -> f32 {
    typography::FACT_LABEL.line_height + component::fact::LINE_GAP
}

/// Facts `widths` wide (each the wider of its label and value) across a card `width`
/// wide: each one's left edge and the room it may take. The first starts at the left
/// inset and the last ends at the right one, the rest spread evenly between; when they
/// don't fit, the gaps shrink to `space.md` and the last is cut.
pub fn facts(look: &Look, width: f32, widths: &[f32]) -> Vec<(f32, f32)> {
    let inset = look.inset();
    let (l, r) = (inset, width - inset);
    let n = widths.len();
    if n == 0 {
        return Vec::new();
    }
    let total: f32 = widths.iter().sum();
    let gap = if n > 1 { ((r - l - total) / (n - 1) as f32).max(space::MD) } else { 0.0 };
    let mut x = l;
    widths
        .iter()
        .map(|w| {
            let at = x;
            x += w + gap;
            (at, (r - at).min(*w).max(0.0))
        })
        .collect()
}

/// The header of a view whose hero is a row of gauges: just the eyebrow's line.
pub fn eyebrow_only() -> header::Text {
    header::Text::new(true, header::Hero::None, 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::{Appearance, Padding};

    #[test]
    fn two_gauges_split_the_card_between_the_insets() {
        let look = Look::default();
        let (lefts, w) = columns(&look, 480.0, 2);
        assert_eq!(lefts[0], look.inset());
        assert!((lefts[1] + w - (480.0 - look.inset())).abs() < 0.01);
        assert!((lefts[1] - lefts[0] - w - component::gauge::GAP).abs() < 0.01);
    }

    #[test]
    fn a_gauge_reserves_its_detail_line_for_the_row() {
        let look = Look::default();
        let with = gauge(&look, 208.0, true);
        let without = gauge(&look, 208.0, false);
        assert!(with.height > without.height);
        // Text taller than the ring: the ring is centred on it, the text starts at the top.
        assert_eq!(with.label, 0.0);
        assert_eq!(with.ring.1, (with.spark - look.padded(component::gauge::SPARK_GAP)) / 2.0);
        assert_eq!(with.value, typography::GAUGE_LABEL.line_height + island::expanded::LINE_GAP);
        assert!(with.text_left > component::ring::EXPANDED);
        let roomy = Look::new(&Appearance { padding: Padding::Roomy, ..Appearance::default() }, false, None);
        assert!(gauge(&roomy, 208.0, true).spark > with.spark, "roomy padding opens the gap to the line");
    }

    #[test]
    fn a_sparkline_fills_in_from_the_right() {
        let (l, r, top) = (0.0, 120.0, 10.0);
        let dot = component::gauge::SPARK_DOT / 2.0;
        assert!(spark_points(&[], 60, l, r, top).is_empty());
        let one = spark_points(&[0.5], 60, l, r, top);
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].0, r - dot, "the newest point is at the right end");
        let three = spark_points(&[0.0, 1.0, 0.25], 60, l, r, top);
        let step = (r - l - 2.0 * dot) / 59.0;
        assert!((three[1].0 - (r - dot - step)).abs() < 1e-4 && (three[0].0 - (r - dot - 2.0 * step)).abs() < 1e-4);
        assert_eq!(three[0].1, spark_zero(top), "zero sits on the zero line");
        assert_eq!(three[1].1, top + dot, "full reaches the top, half a dot in");
        // A full minute spans the box; more than that keeps only the newest.
        let full: Vec<f32> = (0..75).map(|i| i as f32 / 75.0).collect();
        let points = spark_points(&full, 60, l, r, top);
        assert_eq!(points.len(), 60);
        assert!((points[0].0 - (l + dot)).abs() < 1e-3);
        assert!(spark_points(&[2.0, -1.0], 60, l, r, top).iter().all(|p| p.1 >= top && p.1 <= spark_zero(top)), "clamped");
    }

    #[test]
    fn facts_run_from_inset_to_inset() {
        let look = Look::default();
        let i = look.inset();
        let placed = facts(&look, 480.0, &[90.0, 30.0, 150.0]);
        assert_eq!(placed[0], (i, 90.0));
        assert!((placed[2].0 + 150.0 - (480.0 - i)).abs() < 0.01, "the last ends at the right inset");
        assert!(((placed[1].0 - placed[0].0 - 90.0) - (placed[2].0 - placed[1].0 - 30.0)).abs() < 0.01, "even gaps");
        assert_eq!(facts(&look, 480.0, &[60.0]), vec![(i, 60.0)]);
        // Too wide: the gaps shrink to the minimum and the last is cut at the inset.
        let tight = facts(&look, 300.0, &[130.0, 130.0]);
        assert_eq!(tight[1].0, i + 130.0 + space::MD);
        assert!((tight[1].0 + tight[1].1 - (300.0 - i)).abs() < 0.01);
    }

    #[test]
    fn a_gauge_card_header_is_its_eyebrow_line() {
        assert_eq!(eyebrow_only().height(), typography::EYEBROW.line_height, "no hero, no gap under the eyebrow");
    }
}
