//! Rules of the music card, pure so they're testable without a window: scrubbing, the
//! optimistic seek hold, the volume slider, the current lyric line and swipe-to-skip.
//! `pill.rs` drives them; `render.rs` draws what they decide.

use std::time::{Duration, Instant};

use reui_protocol::{LyricLine, Row};

/// A seek is shown at once and held this long while the player catches up (E-T6)...
pub const SEEK_HOLD_MS: u64 = 2_000;
/// ...or until it reports a position this close to the held one.
pub const SEEK_CONFIRM_MS: u64 = 1_500;
/// Dragging a slider tells the module at most this often (20 Hz); the release always goes.
pub const SLIDER_SEND_MS: u64 = 50;
/// One mouse-wheel notch over a slider.
pub const SLIDER_WHEEL_STEP: f32 = 0.05;
/// After a swipe skips, horizontal scrolling must pause this long before the next one
/// counts, so a trackpad gesture (and its inertia) is one skip.
pub const SWIPE_COOLDOWN: Duration = Duration::from_millis(400);
/// One wheel notch (`WHEEL_DELTA`): how far a trackpad must travel sideways to skip.
pub const WHEEL_NOTCH: i32 = 120;

/// Where `x` falls along a bar from `left` to `right`, 0..=1.
pub fn fraction_at(x: f32, left: f32, right: f32) -> f32 {
    ((x - left) / (right - left).max(1.0)).clamp(0.0, 1.0)
}

/// The track position under the pointer on a seek bar.
pub fn scrub_ms(x: f32, left: f32, right: f32, length_ms: u64) -> u64 {
    (fraction_at(x, left, right) as f64 * length_ms as f64) as u64
}

/// A seek the island showed before the player confirmed it: `position_ms` at `at_unix_ms`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeekHold {
    pub position_ms: u64,
    pub at_unix_ms: u64,
}

impl SeekHold {
    /// The held position at `now`, moving on like the timeline would.
    pub fn position(&self, length_ms: u64, playing: bool, rate: f32, now: u64) -> u64 {
        Row::timeline_position(self.position_ms, length_ms, playing, self.at_unix_ms, rate, now)
    }

    /// Whether the hold still overrides what the player reports (`reported_ms`, already
    /// advanced to `now`): until it agrees within [`SEEK_CONFIRM_MS`] or the hold expires.
    pub fn holds(&self, reported_ms: u64, length_ms: u64, playing: bool, rate: f32, now: u64) -> bool {
        now.saturating_sub(self.at_unix_ms) < SEEK_HOLD_MS && reported_ms.abs_diff(self.position(length_ms, playing, rate, now)) > SEEK_CONFIRM_MS
    }
}

/// `<action><value>` with two decimals, as a [`Row::Slider`] wants it.
pub fn slider_action(action: &str, value: f32) -> String {
    format!("{action}{:.2}", value.clamp(0.0, 1.0))
}

/// The slider value after `delta` wheel units (120 a notch, positive = up = louder).
pub fn wheel_value(value: f32, delta: i32) -> f32 {
    (value + SLIDER_WHEEL_STEP * delta as f32 / WHEEL_NOTCH as f32).clamp(0.0, 1.0)
}

/// Limits a drag's updates to [`SLIDER_SEND_MS`] apart.
#[derive(Default, Debug)]
pub struct Throttle {
    last: Option<Instant>,
}

impl Throttle {
    /// May an update go now? Records it if so.
    pub fn ready(&mut self, now: Instant) -> bool {
        if self.last.is_some_and(|t| now.duration_since(t) < Duration::from_millis(SLIDER_SEND_MS)) {
            return false;
        }
        self.last = Some(now);
        true
    }
}

/// The line being sung at `position_ms`: the last one that has started.
pub fn current_line(lines: &[LyricLine], position_ms: u64) -> Option<usize> {
    lines.partition_point(|l| l.at_ms <= position_ms).checked_sub(1)
}

/// Ease-out for the lyric slide: quick to start, settling softly (`t` 0..=1 of the duration).
pub fn lyric_ease(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// Milliseconds of track until the next line starts, if any does.
pub fn next_line_in(lines: &[LyricLine], position_ms: u64) -> Option<u64> {
    lines.get(lines.partition_point(|l| l.at_ms <= position_ms)).map(|l| l.at_ms - position_ms)
}

/// Turns a stream of horizontal wheel deltas into single skips. Content moves with the
/// fingers: swiping left (positive delta, like scrolling right) brings the next track in
/// from the right, so it's "next"; swiping right is "previous".
#[derive(Default, Debug)]
pub struct Swipe {
    travel: i32,
    /// When the last skip fired, and the latest event since (cooldown runs from it).
    quiet_since: Option<Instant>,
}

impl Swipe {
    /// Feeds one event; returns `Some(true)` for next, `Some(false)` for previous.
    pub fn feed(&mut self, delta: i32, now: Instant) -> Option<bool> {
        if let Some(t) = self.quiet_since {
            if now.duration_since(t) < SWIPE_COOLDOWN {
                // Still the same gesture (or its inertia): keep it from counting again.
                self.quiet_since = Some(now);
                return None;
            }
            self.quiet_since = None;
        }
        // A change of direction starts over.
        if self.travel.signum() * delta.signum() < 0 {
            self.travel = 0;
        }
        self.travel += delta;
        if self.travel.abs() < WHEEL_NOTCH {
            return None;
        }
        let next = self.travel > 0;
        self.travel = 0;
        self.quiet_since = Some(now);
        Some(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrubbing_maps_the_pointer_onto_the_track_and_clamps() {
        assert_eq!(scrub_ms(210.0, 20.0, 400.0, 200_000), 100_000);
        assert_eq!(scrub_ms(-50.0, 20.0, 400.0, 200_000), 0);
        assert_eq!(scrub_ms(999.0, 20.0, 400.0, 200_000), 200_000);
        // A degenerate bar never divides by zero.
        assert_eq!(scrub_ms(20.0, 20.0, 20.0, 1_000), 0);
    }

    #[test]
    fn a_seek_holds_until_confirmed_or_two_seconds_pass() {
        let hold = SeekHold { position_ms: 60_000, at_unix_ms: 10_000 };
        // Player still reports the old place: hold.
        assert!(hold.holds(5_000, 200_000, true, 1.0, 10_500));
        // The held position moves on while playing; a report near it confirms.
        assert_eq!(hold.position(200_000, true, 1.0, 11_000), 61_000);
        assert!(!hold.holds(60_200, 200_000, true, 1.0, 11_000));
        // Never confirmed: let go after the hold time, snapping back to what's reported.
        assert!(!hold.holds(5_000, 200_000, true, 1.0, 12_000));
        // Paused, it doesn't move.
        assert_eq!(hold.position(200_000, false, 1.0, 11_500), 60_000);
    }

    #[test]
    fn slider_values_clamp_and_print_two_decimals() {
        assert_eq!(slider_action("volume:", 0.426), "volume:0.43");
        assert_eq!(slider_action("volume:", 1.7), "volume:1.00");
        assert_eq!(wheel_value(0.5, 120), 0.55);
        assert_eq!(wheel_value(0.5, -240), 0.4);
        assert_eq!(wheel_value(0.98, 120), 1.0);
        assert_eq!(wheel_value(0.02, -120), 0.0);
        assert!((wheel_value(0.5, 60) - 0.525).abs() < 1e-6);
    }

    #[test]
    fn slider_updates_are_throttled_to_twenty_a_second() {
        let mut throttle = Throttle::default();
        let t = Instant::now();
        assert!(throttle.ready(t));
        assert!(!throttle.ready(t + Duration::from_millis(30)));
        assert!(throttle.ready(t + Duration::from_millis(50)));
        assert!(!throttle.ready(t + Duration::from_millis(99)));
        assert!(throttle.ready(t + Duration::from_millis(100)));
    }

    #[test]
    fn the_lyric_slide_eases_out_and_lands() {
        assert_eq!(lyric_ease(0.0), 0.0);
        assert_eq!(lyric_ease(1.0), 1.0);
        assert_eq!(lyric_ease(2.0), 1.0);
        assert!(lyric_ease(0.5) > 0.5, "quick to start, soft to settle");
    }

    #[test]
    fn the_current_lyric_is_the_last_one_started() {
        let line = |at_ms, text: &str| LyricLine { at_ms, text: text.into() };
        let lines = [line(1_000, "one"), line(4_000, ""), line(6_500, "three")];
        assert_eq!(current_line(&lines, 0), None);
        assert_eq!(next_line_in(&lines, 0), Some(1_000));
        assert_eq!(current_line(&lines, 1_000), Some(0));
        assert_eq!(current_line(&lines, 3_999), Some(0));
        assert_eq!(next_line_in(&lines, 3_999), Some(1));
        assert_eq!(current_line(&lines, 5_000), Some(1));
        assert_eq!(current_line(&lines, 9_000), Some(2));
        assert_eq!(next_line_in(&lines, 9_000), None);
        assert_eq!(current_line(&[], 9_000), None);
    }

    #[test]
    fn one_swipe_is_one_skip() {
        let mut swipe = Swipe::default();
        let t = Instant::now();
        let ms = |n| t + Duration::from_millis(n);
        // Small trackpad deltas add up to a notch.
        assert_eq!(swipe.feed(50, ms(0)), None);
        assert_eq!(swipe.feed(80, ms(10)), Some(true));
        // The rest of the gesture and its inertia don't skip again...
        assert_eq!(swipe.feed(200, ms(100)), None);
        assert_eq!(swipe.feed(200, ms(450)), None);
        // ...until it has been quiet for the cooldown.
        assert_eq!(swipe.feed(-130, ms(900)), Some(false));
        // A reversal starts the count over.
        let mut swipe = Swipe::default();
        assert_eq!(swipe.feed(100, ms(0)), None);
        assert_eq!(swipe.feed(-100, ms(5)), None);
        assert_eq!(swipe.feed(-30, ms(10)), Some(false));
    }
}
