//! Levels (volume, brightness) ease between values per activity, so a run of key presses
//! reads as one movement. The pill drives the easing on a short frame timer that stops when
//! every level has landed (no compositor loop); this part is pure, so it's testable.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use reui_design::tokens::motion;
use reui_protocol::{Activity, ActivityId, Element};

/// One activity's level: easing from `from` to `to` since `since`, or resting at `to`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Track {
    from: f32,
    to: f32,
    since: Option<Instant>,
}

impl Track {
    fn at(&self, now: Instant) -> f32 {
        match self.since {
            Some(since) => self.from + (self.to - self.from) * ease_out(progress(now.saturating_duration_since(since))),
            None => self.to,
        }
    }
}

/// The levels the island last drew, by activity.
#[derive(Default, Debug)]
pub struct Levels {
    tracks: HashMap<ActivityId, Track>,
}

impl Levels {
    /// Takes the levels of a new snapshot. A new activity's first value is drawn as it is; a
    /// changed one eases from where it's shown now (or jumps, without `animate`); an
    /// activity that's gone, or no longer shows a level, is forgotten.
    pub fn sync(&mut self, activities: &[Activity], now: Instant, animate: bool) {
        let mut next = HashMap::new();
        for a in activities {
            let Some(value) = level_of(a) else { continue };
            let track = match self.tracks.get(&a.id) {
                Some(old) if old.to == value => *old,
                Some(old) if animate => Track { from: old.at(now), to: value, since: Some(now) },
                _ => Track { from: value, to: value, since: None },
            };
            next.insert(a.id.clone(), track);
        }
        self.tracks = next;
    }

    /// The value to draw for `id` at `now`.
    pub fn value(&self, id: &ActivityId, now: Instant) -> Option<f32> {
        self.tracks.get(id).map(|t| t.at(now))
    }

    pub fn moving(&self) -> bool {
        self.tracks.values().any(|t| t.since.is_some())
    }

    /// Lands the levels whose easing is over; returns whether any is still moving.
    pub fn settle(&mut self, now: Instant) -> bool {
        for t in self.tracks.values_mut() {
            if t.since.is_some_and(|since| progress(now.saturating_duration_since(since)) >= 1.0) {
                *t = Track { from: t.to, to: t.to, since: None };
            }
        }
        self.moving()
    }
}

/// How far through the easing `elapsed` is, 0..=1.
fn progress(elapsed: Duration) -> f32 {
    (elapsed.as_secs_f32() * 1000.0 / motion::level::DURATION as f32).clamp(0.0, 1.0)
}

/// Cubic ease-out: quick to respond, gentle to land.
pub fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// The activity's level: the first [`Element::Level`] it shows anywhere (compact, minimal,
/// expanded header). One per activity, so all its presentations move together.
pub fn level_of(a: &Activity) -> Option<f32> {
    let expanded = a.expanded.as_ref().map(|v| [&v.leading, &v.trailing]);
    [&a.compact.trailing, &a.compact.leading, &a.minimal].into_iter().chain(expanded.into_iter().flatten()).find_map(|e| match e {
        Element::Level { value, .. } => Some(value.clamp(0.0, 1.0)),
        _ => None,
    })
}

/// Whether `e` is a level (the surfaces holding one redraw while it eases).
pub fn is_level(e: &Element) -> bool {
    matches!(e, Element::Level { .. })
}

#[cfg(test)]
mod tests {
    use super::*;
    use reui_protocol::{CompactView, Priority};

    fn volume(key: &str, value: f32) -> Activity {
        Activity {
            id: ActivityId { module: "osd".into(), key: key.into() },
            priority: Priority::Brief,
            accent: None,
            compact: CompactView { leading: Element::Glyph { glyph: '\u{E767}', tone: reui_protocol::Tone::Primary }, trailing: Element::Level { value, dim: false } },
            minimal: Element::Ring(value),
            audio: None,
            expanded: None,
        }
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn ease_out_starts_quick_and_lands_exactly() {
        assert_eq!(ease_out(0.0), 0.0);
        assert_eq!(ease_out(1.0), 1.0);
        assert!(ease_out(0.5) > 0.8);
        assert_eq!(ease_out(2.0), 1.0);
    }

    #[test]
    fn a_new_activity_draws_its_first_value_directly() {
        let mut levels = Levels::default();
        let t0 = Instant::now();
        levels.sync(&[volume("volume", 0.4)], t0, true);
        assert_eq!(levels.value(&volume("volume", 0.0).id, t0), Some(0.4));
        assert!(!levels.moving());
    }

    #[test]
    fn a_changed_value_eases_and_settles() {
        let mut levels = Levels::default();
        let id = volume("volume", 0.0).id;
        let t0 = Instant::now();
        levels.sync(&[volume("volume", 0.4)], t0, true);
        levels.sync(&[volume("volume", 0.6)], t0, true);
        assert!(levels.moving());
        assert_eq!(levels.value(&id, t0), Some(0.4));
        let mid = levels.value(&id, t0 + ms(60)).unwrap();
        assert!(mid > 0.5 && mid < 0.6, "{mid}");
        assert!(levels.settle(t0 + ms(60)));
        assert!(!levels.settle(t0 + ms(motion::level::DURATION as u64)));
        assert_eq!(levels.value(&id, t0 + ms(500)), Some(0.6));
    }

    #[test]
    fn a_change_mid_ease_starts_from_what_is_shown() {
        let mut levels = Levels::default();
        let id = volume("volume", 0.0).id;
        let t0 = Instant::now();
        levels.sync(&[volume("volume", 0.0)], t0, true);
        levels.sync(&[volume("volume", 1.0)], t0, true);
        let shown = levels.value(&id, t0 + ms(30)).unwrap();
        levels.sync(&[volume("volume", 0.5)], t0 + ms(30), true);
        assert_eq!(levels.value(&id, t0 + ms(30)), Some(shown));
        // The same value again doesn't restart the easing.
        levels.sync(&[volume("volume", 0.5)], t0 + ms(60), true);
        assert!(levels.value(&id, t0 + ms(60)).unwrap() != 0.5);
    }

    #[test]
    fn reduced_motion_jumps() {
        let mut levels = Levels::default();
        let t0 = Instant::now();
        levels.sync(&[volume("volume", 0.2)], t0, false);
        levels.sync(&[volume("volume", 0.8)], t0, false);
        assert_eq!(levels.value(&volume("volume", 0.0).id, t0), Some(0.8));
        assert!(!levels.moving());
    }

    #[test]
    fn levels_are_per_activity_and_gone_ones_are_forgotten() {
        let mut levels = Levels::default();
        let t0 = Instant::now();
        levels.sync(&[volume("volume", 0.2), volume("brightness", 0.7)], t0, true);
        levels.sync(&[volume("brightness", 0.7)], t0, true);
        assert_eq!(levels.value(&volume("volume", 0.0).id, t0), None);
        assert!(!levels.moving());
        // Back again, it's new: drawn directly.
        levels.sync(&[volume("volume", 0.9), volume("brightness", 0.7)], t0, true);
        assert_eq!(levels.value(&volume("volume", 0.0).id, t0), Some(0.9));
        assert!(!levels.moving());
    }

    #[test]
    fn activities_without_a_level_are_not_tracked() {
        let mut plain = volume("copied", 0.0);
        plain.compact.trailing = Element::Empty;
        plain.minimal = Element::Empty;
        assert_eq!(level_of(&plain), None);
        let mut levels = Levels::default();
        levels.sync(&[plain.clone()], Instant::now(), true);
        assert_eq!(levels.value(&plain.id, Instant::now()), None);
    }
}
