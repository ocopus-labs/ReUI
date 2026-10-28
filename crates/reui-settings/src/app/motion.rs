//! Everything that moves, stepped once per displayed frame: hover fills easing in and out,
//! toggle knobs sliding, the page scrolling to its target, and a page's island preview
//! settling in. Each frame takes the real time since the last one, so motion keeps its
//! speed whatever the refresh rate. When nothing moves, [`Motion::moving`] is false and
//! the window draws nothing more.

use std::collections::HashMap;
use std::time::Instant;

use reui_design::tokens::prefs::motion as m;

use crate::ui::Hover;

/// Hover fills closer than this to their target snap to it.
const SETTLE: f32 = 0.004;

pub struct Motion {
    /// Hover fills and their amount 0..=1; the one under the pointer heads to 1.
    lit: Vec<(Hover, f32)>,
    /// Toggles sliding: row id → (from, to, start).
    knobs: HashMap<String, (f32, f32, Instant)>,
    pub scroll: f32,
    pub scroll_target: f32,
    /// When the page's preview started settling in.
    entered: Option<Instant>,
    last: Instant,
    pub reduced: bool,
}

impl Motion {
    pub fn new(reduced: bool) -> Self {
        Self { lit: Vec::new(), knobs: HashMap::new(), scroll: 0.0, scroll_target: 0.0, entered: None, last: Instant::now(), reduced }
    }

    /// What's still on its way (for `--perf`).
    pub fn why(&self, hover: &Hover) -> &'static str {
        if self.scroll != self.scroll_target {
            "scroll"
        } else if !self.knobs.is_empty() {
            "knob"
        } else if self.entered.is_some() {
            "enter"
        } else if self.lit.iter().any(|(key, v)| *v != if key == hover { 1.0 } else { 0.0 }) {
            "hover"
        } else {
            "change"
        }
    }

    /// Something on the page itself is moving (not just the scroll): a knob, the preview,
    /// a hover fill.
    pub fn restless(&self, hover: &Hover) -> bool {
        !self.knobs.is_empty() || self.entered.is_some() || self.lit.iter().any(|(key, v)| *v != if key == hover { 1.0 } else { 0.0 })
    }

    /// Anything still on its way.
    pub fn moving(&self, hover: &Hover) -> bool {
        self.scroll != self.scroll_target || !self.knobs.is_empty() || self.entered.is_some() || self.lit.iter().any(|(key, v)| *v != if key == hover { 1.0 } else { 0.0 })
    }

    /// The time since the last frame starts now (after a rest, the first step is small).
    pub fn wake(&mut self) {
        self.last = Instant::now();
    }

    /// One frame: everything moves on by the time since the last.
    pub fn step(&mut self, hover: &Hover) {
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f32() * 1000.0;
        self.last = now;
        // Hover: exponential approach, ~120 ms to settle.
        if *hover != Hover::None && !self.lit.iter().any(|(k, _)| k == hover) {
            self.lit.push((hover.clone(), 0.0));
        }
        let k = if self.reduced { 1.0 } else { 1.0 - (-dt / m::HOVER as f32).exp() };
        for (key, v) in &mut self.lit {
            let target = if key == hover { 1.0 } else { 0.0 };
            *v += (target - *v) * k;
            if (target - *v).abs() < SETTLE {
                *v = target;
            }
        }
        self.lit.retain(|(key, v)| *v > 0.0 || key == hover);
        self.knobs.retain(|_, &mut (_, _, start)| now.duration_since(start).as_secs_f32() * 1000.0 < m::TOGGLE as f32);
        let gap = self.scroll_target - self.scroll;
        if gap.abs() < 0.25 || self.reduced {
            self.scroll = self.scroll_target;
        } else {
            self.scroll += gap * (1.0 - (-dt / m::WHEEL as f32).exp());
        }
        if self.enter() >= 1.0 {
            self.entered = None;
        }
    }

    pub fn lit(&self, key: &Hover) -> f32 {
        self.lit.iter().find(|(k, _)| k == key).map_or(0.0, |(_, v)| *v)
    }

    /// A toggle at `row` starts sliding.
    pub fn slide(&mut self, row: &str, on: bool) {
        if self.reduced {
            return;
        }
        let (from, to) = if on { (1.0, 0.0) } else { (0.0, 1.0) };
        self.knobs.insert(row.to_string(), (from, to, Instant::now()));
    }

    /// Knobs mid-slide: row id → position.
    pub fn knobs(&self) -> HashMap<String, f32> {
        let now = Instant::now();
        self.knobs.iter().map(|(id, &(from, to, start))| (id.clone(), knob_at(from, to, start, now))).collect()
    }

    /// A page opened: its preview settles in.
    pub fn enter_page(&mut self) {
        self.entered = (!self.reduced).then(Instant::now);
        self.lit.clear();
        self.knobs.clear();
    }

    /// How far the preview has settled, 0..=1.
    pub fn enter(&self) -> f32 {
        self.entered.map_or(1.0, |start| (start.elapsed().as_secs_f32() * 1000.0 / m::ENTER as f32).min(1.0))
    }

    /// Jumps the scroll (thumb drags, touchpads: the input is already smooth).
    pub fn jump(&mut self, to: f32) {
        self.scroll = to;
        self.scroll_target = to;
    }
}

/// A knob's position `from` → `to`, eased out.
fn knob_at(from: f32, to: f32, start: Instant, now: Instant) -> f32 {
    let t = (now.duration_since(start).as_secs_f32() * 1000.0 / m::TOGGLE as f32).clamp(0.0, 1.0);
    let eased = 1.0 - (1.0 - t).powi(3);
    from + (to - from) * eased
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn knobs_ease_out_to_their_end() {
        let start = Instant::now();
        assert_eq!(knob_at(0.0, 1.0, start, start), 0.0);
        let mid = knob_at(0.0, 1.0, start, start + Duration::from_millis(u64::from(m::TOGGLE / 2)));
        assert!(mid > 0.5 && mid < 1.0, "past halfway at half time: {mid}");
        assert_eq!(knob_at(1.0, 0.0, start, start + Duration::from_secs(1)), 0.0);
    }

    #[test]
    fn hover_fills_ease_in_and_out_then_rest() {
        let mut motion = Motion::new(false);
        let nav = Hover::Nav(1);
        assert!(!motion.moving(&Hover::None));
        motion.wake();
        std::thread::sleep(Duration::from_millis(20));
        motion.step(&nav);
        let a = motion.lit(&nav);
        assert!(a > 0.0 && a < 1.0, "part way after one frame: {a}");
        assert!(motion.moving(&nav));
        // Long frames: settled on, then off, then nothing moves.
        for _ in 0..3 {
            std::thread::sleep(Duration::from_millis(120));
            motion.step(&nav);
        }
        assert_eq!(motion.lit(&nav), 1.0);
        assert!(!motion.moving(&nav));
        for _ in 0..3 {
            std::thread::sleep(Duration::from_millis(120));
            motion.step(&Hover::None);
        }
        assert_eq!(motion.lit(&nav), 0.0);
        assert!(!motion.moving(&Hover::None));
    }

    #[test]
    fn reduced_motion_jumps() {
        let mut motion = Motion::new(true);
        motion.scroll_target = 300.0;
        motion.step(&Hover::Nav(0));
        assert_eq!(motion.scroll, 300.0);
        assert_eq!(motion.lit(&Hover::Nav(0)), 1.0);
        motion.enter_page();
        assert_eq!(motion.enter(), 1.0);
    }
}
