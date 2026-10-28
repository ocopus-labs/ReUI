//! Clocks (timers, stopwatches): how they read, and when the island next has to redraw
//! one. The value itself comes from the protocol's [`Clock`]; this is presentation only.

use reui_protocol::{Activity, Clock, Element, Headline};

const SECOND: u64 = 1000;
const TENTH: u64 = 100;

/// `m:ss` below an hour, `h:mm:ss` from there; with `tenths`, a tenths digit after.
pub fn text(clock: &Clock, now_unix_ms: u64, tenths: bool) -> String {
    let (step, extra) = if tenths { (TENTH, 10) } else { (SECOND, 1) };
    let shown = clock.shown(now_unix_ms, step);
    let secs = shown / extra;
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    let mut out = if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") };
    if tenths {
        out.push_str(&format!(".{}", shown % extra));
    }
    out
}

/// How much of the ring is filled: what's left of a countdown, what's run of a count-up.
pub fn fraction(clock: &Clock, total_ms: u64, now_unix_ms: u64) -> f32 {
    if total_ms == 0 {
        return 0.0;
    }
    (clock.value_at(now_unix_ms) as f64 / total_ms as f64).clamp(0.0, 1.0) as f32
}

/// Milliseconds until a running clock in `activity` reads differently, looking only at the
/// presentations that are on screen (`compact` / `minimal` / `expanded`). Rings move with
/// the seconds too, so they share the numerals' tick.
pub fn next_change(activity: &Activity, compact: bool, minimal: bool, expanded: bool, now_unix_ms: u64) -> Option<u64> {
    let mut soonest: Option<u64> = None;
    let mut note = |clock: &Clock, step: u64| {
        if let Some(ms) = clock.next_change(now_unix_ms, step) {
            soonest = Some(soonest.map_or(ms, |s| s.min(ms)));
        }
    };
    let mut element = |e: &Element| match e {
        Element::Clock(c) | Element::ClockRing { clock: c, .. } => note(c, SECOND),
        _ => {}
    };
    if compact {
        element(&activity.compact.leading);
        element(&activity.compact.trailing);
    }
    if minimal {
        element(&activity.minimal);
    }
    if let Some(view) = activity.expanded.as_ref().filter(|_| expanded) {
        element(&view.leading);
        element(&view.trailing);
        if let Headline::Clock { clock, tenths } = &view.headline {
            note(clock, if *tenths { TENTH } else { SECOND });
        }
    }
    soonest
}

/// Whether a compact side shows a clock (whose width can change as it runs).
pub fn in_compact(activity: &Activity) -> bool {
    [&activity.compact.leading, &activity.compact.trailing].iter().any(|e| matches!(e, Element::Clock(_) | Element::ClockRing { .. }))
}

/// Whether an open card shows a clock.
pub fn in_expanded(activity: &Activity) -> bool {
    activity.expanded.as_ref().is_some_and(|v| {
        matches!(v.headline, Headline::Clock { .. }) || [&v.leading, &v.trailing].iter().any(|e| matches!(e, Element::Clock(_) | Element::ClockRing { .. }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(value_ms: u64, running: bool, down: bool) -> Clock {
        Clock { value_ms, at_unix_ms: 0, running, down }
    }

    #[test]
    fn reads_like_a_clock() {
        assert_eq!(text(&at(4 * 60_000, true, true), 0, false), "4:00");
        assert_eq!(text(&at(4 * 60_000, true, true), 1, false), "4:00");
        assert_eq!(text(&at(4 * 60_000, true, true), 1_000, false), "3:59");
        assert_eq!(text(&at(400, false, true), 0, false), "0:01");
        assert_eq!(text(&at(0, false, true), 0, false), "0:00");
        assert_eq!(text(&at(3_600_000 + 61_000, false, false), 0, false), "1:01:01");
        assert_eq!(text(&at(72_400, false, false), 0, true), "1:12.4");
    }

    #[test]
    fn ring_fills_by_what_is_left() {
        assert_eq!(fraction(&at(30_000, false, true), 60_000, 0), 0.5);
        assert_eq!(fraction(&at(90_000, false, false), 60_000, 0), 1.0);
        assert_eq!(fraction(&at(1, false, true), 0, 0), 0.0);
    }
}
