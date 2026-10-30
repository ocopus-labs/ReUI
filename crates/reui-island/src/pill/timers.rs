//! Timer ids and `on_timer` dispatch, plus the clocks they drive: the timeline tick,
//! lyric slides, level easing, card-row scrolling and held seeks.

use std::time::Instant;

use reui_design::tokens::motion;
use reui_protocol::{
    Activity, Row,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::features::clock;
use crate::features::level;
use crate::features::music;
use crate::render;
use crate::settings::Settings;

use super::{Pill, State};

pub(super) const TIMER_COLLAPSE: usize = 1;

pub(super) const TIMER_HOVER: usize = 2;

pub(super) const TIMER_REVEAL: usize = 3;

pub(super) const TIMER_SPIN: usize = 4;

pub(super) const TIMER_TICK: usize = 5;

pub(super) const TIMER_BARS: usize = 6;

pub(super) const TIMER_REORDER: usize = 7;

pub(super) const TIMER_SCROLL: usize = 8;

pub(super) const TIMER_SEARCH: usize = 9;

pub(super) const TIMER_AWAIT: usize = 10;

pub(super) const TIMER_HOLD: usize = 11;

pub(super) const TIMER_LYRIC: usize = 12;

pub(super) const TIMER_LEVEL: usize = 13;
/// "Pause ReUI" ends (one-shot, set for the time left).
pub(super) const TIMER_PAUSE: usize = 14;
/// A clock on screen (a timer, a stopwatch) reads differently: one-shot, just after it does.
pub(super) const TIMER_CLOCK: usize = 15;
/// The next snooze ends (one-shot, only while something is snoozed).
pub(super) const TIMER_SNOOZE: usize = 16;

/// How often an open card with a playing timeline redraws its position.
const TICK_MS: u32 = 1000;

impl Pill {
    pub fn on_timer(&mut self, id: usize) {
        // The bar and scroll frame timers are periodic; every other timer here is one-shot.
        if id == TIMER_BARS {
            self.tick_bars();
            return;
        }
        if id == TIMER_SCROLL {
            self.tick_scroll();
            return;
        }
        if id == TIMER_LYRIC {
            self.tick_lyric();
            return;
        }
        if id == TIMER_LEVEL {
            self.tick_level();
            return;
        }
        unsafe { _ = KillTimer(Some(self.hwnd), id) };
        match id {
            TIMER_HOVER => self.on_hover_intent(),
            TIMER_PAUSE => self.apply_settings(Settings::load()),
            TIMER_SNOOZE => self.gate_changed(),
            TIMER_HOLD if self.state == State::Expanded => {
                self.redraw_expanded();
                self.update_tick();
            }
            TIMER_TICK => {
                self.redraw_expanded();
                self.update_tick();
            }
            TIMER_CLOCK => self.tick_clock(),
            TIMER_SPIN if self.logo_state.is_some_and(|(_, ticking)| ticking) => {
                unsafe { SetTimer(Some(self.hwnd), TIMER_SPIN, motion::logo::INTERVAL, None) };
                self.tick_logo();
            }
            // Revealed stays revealed while the pointer is on it or a summoned card is open.
            TIMER_REVEAL if self.inside || (self.summoned && self.state == State::Expanded) => unsafe {
                SetTimer(Some(self.hwnd), TIMER_REVEAL, motion::delay::REVEAL_HOLD, None);
            },
            TIMER_REVEAL => {
                self.revealed = false;
                self.refresh_visibility();
            }
            TIMER_COLLAPSE if !self.hovered && matches!(self.state, State::Expanded | State::Places) => self.set_state(State::Compact),
            TIMER_SEARCH => self.flush_search(),
            TIMER_AWAIT => {
                // The results never came back to match; act on what's showing.
                self.run_awaiting();
                if self.state == State::Expanded {
                    self.redraw_expanded();
                }
            }
            TIMER_REORDER => {
                unsafe { _ = KillTimer(Some(self.hwnd), TIMER_REORDER) };
                let ordered = self.ordered();
                if let Err(e) = self.render(ordered) {
                    eprintln!("island: render failed: {e}");
                }
            }
            _ => {}
        }
    }

    /// An open card with a playing timeline is the one thing that redraws on a clock, once a
    /// second, and only while it's open; compact never ticks.
    /// Redraws the playing timeline just after each whole second of its position, so the
    /// times step 0:09 → 0:10 → 0:11 instead of skipping when the ticks drift.
    /// Lyrics move on at their own lines too: the tick comes early when a line starts
    /// before the next whole second.
    pub(super) fn update_tick(&self) {
        self.update_clock_tick();
        let playing = self.timeline_now().filter(|(_, _, playing, _)| *playing);
        unsafe {
            match playing {
                Some((now, _, _, rate)) if self.state == State::Expanded && self.shown => {
                    let lyrics = self.front_view().and_then(|v| v.rows.iter().find_map(|r| if let Row::Lyrics(l) = r { Some(l.as_slice()) } else { None }));
                    let line = lyrics.and_then(|l| music::next_line_in(l, now)).map(|ms| (ms as f32 / rate.max(0.01)) as u32 + 5);
                    let delay = line.map_or(next_tick(now), |line| line.clamp(16, next_tick(now)));
                    SetTimer(Some(self.hwnd), TIMER_TICK, delay, None);
                }
                _ => _ = KillTimer(Some(self.hwnd), TIMER_TICK),
            }
        }
    }

    /// Running clocks (timers, stopwatches) redraw just after what they read changes, and
    /// only while they're on screen: the pill's compact sides, the bubble, an open card.
    /// A hidden island, or one whose clocks are all stopped, has no timer at all.
    pub(super) fn update_clock_tick(&self) {
        let now = reui_protocol::unix_ms();
        let open = self.state == State::Expanded;
        let compact = matches!(self.state, State::Compact | State::Peek);
        let front = self.front().and_then(|a| clock::next_change(a, compact, false, open, now));
        let second = self.snapshot.activities.get(1).and_then(|a| clock::next_change(a, false, self.minimal, false, now));
        unsafe {
            match front.into_iter().chain(second).chain(self.idle_clock_due()).min().filter(|_| self.shown) {
                // A little after the change, so the redraw lands on the new reading.
                Some(ms) => _ = SetTimer(Some(self.hwnd), TIMER_CLOCK, (ms as u32).saturating_add(15), None),
                None => _ = KillTimer(Some(self.hwnd), TIMER_CLOCK),
            }
        }
    }

    /// Redraws the surfaces that show a clock. The compact side is redrawn even under an
    /// open card, so the pill's width is right when it folds back; a width change (9:59
    /// after 10:00) re-springs the pill only when compact is what's showing.
    fn tick_clock(&mut self) {
        self.draw_idle();
        let default_accent = self.look.accent;
        let accent = |a: &Activity| a.accent.map(render::rgb).unwrap_or(default_accent);
        if let Some(a) = self.snapshot.activities.first().filter(|a| clock::in_compact(a)) {
            match self.renderer.draw_compact(&self.compact_surface, Some(&a.compact), accent(a), None) {
                Ok(layout) if layout.width != self.compact_width => {
                    self.compact_width = layout.width;
                    self.retarget();
                }
                Ok(_) => {}
                Err(e) => eprintln!("island: redraw failed: {e}"),
            }
        }
        if let Some(a) = self.snapshot.activities.get(1).filter(|a| matches!(a.minimal, reui_protocol::Element::Clock(_) | reui_protocol::Element::ClockRing { .. })) {
            if let Err(e) = self.renderer.draw_minimal(&self.minimal_surface, Some(&a.minimal), accent(a), None) {
                eprintln!("island: redraw failed: {e}");
            }
        }
        let open_clock = self.front().is_some_and(clock::in_expanded);
        if self.state == State::Expanded && open_clock {
            self.redraw_expanded();
        }
        self.update_clock_tick();
    }

    /// The open card's timeline as (position now, length, playing, rate), a held seek
    /// counting as the position.
    fn timeline_now(&self) -> Option<(u64, u64, bool, f32)> {
        let now = reui_protocol::unix_ms();
        self.front_view()?.rows.iter().find_map(|r| match r {
            Row::Timeline { position_ms, length_ms, playing, at_unix_ms, rate, .. } if *length_ms > 0 => {
                let reported = Row::timeline_position(*position_ms, *length_ms, *playing, *at_unix_ms, *rate, now);
                let shown = self.seek_hold.map_or(reported, |h| h.position(*length_ms, *playing, *rate, now));
                Some((shown, *length_ms, *playing, *rate))
            }
            _ => None,
        })
    }

    /// Lets go of a held seek once the player agrees with it, or it has held long enough.
    pub(super) fn prune_hold(&mut self) {
        let Some(hold) = self.seek_hold else { return };
        let now = reui_protocol::unix_ms();
        let confirmed = self.front_view().and_then(|v| {
            v.rows.iter().find_map(|r| match r {
                Row::Timeline { position_ms, length_ms, playing, at_unix_ms, rate, .. } if *length_ms > 0 => {
                    let reported = Row::timeline_position(*position_ms, *length_ms, *playing, *at_unix_ms, *rate, now);
                    Some(!hold.holds(reported, *length_ms, *playing, *rate, now))
                }
                _ => None,
            })
        });
        if confirmed.unwrap_or(true) {
            self.seek_hold = None;
        }
    }

    /// Notices a new lyric line and starts the slide (only for the next line in order: a
    /// seek just jumps). The slide timer stops when it lands.
    pub(super) fn note_lyric(&mut self) {
        let lyrics = self.front_view().and_then(|v| v.rows.iter().find_map(|r| if let Row::Lyrics(l) = r { Some(l.as_slice()) } else { None }));
        let now = self.timeline_now().map(|(ms, ..)| ms);
        let line = lyrics.zip(now).and_then(|(l, ms)| music::current_line(l, ms));
        if line == self.lyric_line {
            return;
        }
        let next_in_order = matches!((self.lyric_line, line), (Some(old), Some(new)) if new == old + 1);
        if next_in_order && !self.reduced_motion && self.state == State::Expanded && self.shown {
            self.lyric_change = Some((self.lyric_line.unwrap(), Instant::now()));
            unsafe { SetTimer(Some(self.hwnd), TIMER_LYRIC, 1000 / motion::lyrics::FPS as u32, None) };
        } else {
            self.lyric_change = None;
        }
        self.lyric_line = line;
    }

    fn tick_lyric(&mut self) {
        let done = self.lyric_change.is_none_or(|(_, at)| at.elapsed().as_millis() as u64 >= motion::lyrics::DURATION as u64);
        if done {
            self.lyric_change = None;
            unsafe { _ = KillTimer(Some(self.hwnd), TIMER_LYRIC) };
        }
        self.redraw_expanded();
    }

    /// Starts the level frame timer when a level begins to ease; it stops itself once
    /// every level has landed.
    pub(super) fn update_levels(&mut self) {
        if self.levels.moving() && !self.level_ticking {
            self.level_ticking = true;
            unsafe { SetTimer(Some(self.hwnd), TIMER_LEVEL, frame_interval(motion::level::FPS), None) };
        }
    }

    /// One easing frame: redraws only the surfaces that show a level (the pill, the bubble,
    /// an open card's header). Their size doesn't depend on the value, so nothing re-springs.
    fn tick_level(&mut self) {
        let now = Instant::now();
        let moving = self.levels.settle(now);
        let default_accent = self.look.accent;
        let accent = |a: &Activity| a.accent.map(render::rgb).unwrap_or(default_accent);
        let front = self.snapshot.activities.first();
        if let Some(a) = front.filter(|a| level::is_level(&a.compact.leading) || level::is_level(&a.compact.trailing)) {
            if let Err(e) = self.renderer.draw_compact(&self.compact_surface, Some(&a.compact), accent(a), self.levels.value(&a.id, now)) {
                eprintln!("island: redraw failed: {e}");
            }
        }
        let second = self.snapshot.activities.get(1);
        if let Some((a, value)) = second.and_then(|a| Some((a, self.levels.value(&a.id, now)?))) {
            if let Err(e) = self.renderer.draw_minimal(&self.minimal_surface, Some(&a.minimal), accent(a), Some(value)) {
                eprintln!("island: redraw failed: {e}");
            }
        }
        let in_card = self.front_view().is_some_and(|v| level::is_level(&v.leading) || level::is_level(&v.trailing));
        if in_card && self.state == State::Expanded {
            self.redraw_expanded();
        }
        if !moving {
            self.level_ticking = false;
            unsafe { _ = KillTimer(Some(self.hwnd), TIMER_LEVEL) };
        }
    }

    /// Eases the card row toward its target on a short frame timer that stops when it
    /// settles; with reduced motion it jumps.
    pub(super) fn update_scroll(&mut self) {
        if !self.panel.moving() {
            return;
        }
        if self.reduced_motion || self.state != State::Expanded {
            self.panel.scroll = self.panel.target;
            self.redraw_expanded();
            return;
        }
        if self.scroll_clock.is_none() {
            self.scroll_clock = Some(Instant::now());
            unsafe { SetTimer(Some(self.hwnd), TIMER_SCROLL, frame_interval(motion::scroll::FPS), None) };
        }
    }

    fn tick_scroll(&mut self) {
        let now = Instant::now();
        let dt = self.scroll_clock.replace(now).map_or(16.0, |t| (now - t).as_secs_f32() * 1000.0);
        let moving = self.panel.tick(dt);
        self.redraw_expanded();
        if !moving {
            self.scroll_clock = None;
            unsafe { _ = KillTimer(Some(self.hwnd), TIMER_SCROLL) };
        }
    }
}

/// A frame timer's interval for `fps`, rounded down to the system timer's 15.625 ms grain
/// so it doesn't land on the next tick and drop to a lower rate.
pub(super) fn frame_interval(fps: f32) -> u32 {
    ((1000.0 / fps / 15.625).floor().max(1.0) * 15.625) as u32
}

/// Milliseconds until just after the next whole second of a position.
fn next_tick(position_ms: u64) -> u32 {
    (TICK_MS - (position_ms % u64::from(TICK_MS)) as u32 + 15).max(16)
}

#[cfg(test)]
mod tick_tests {
    #[test]
    fn ticks_land_just_after_whole_seconds() {
        assert_eq!(super::next_tick(9_400), 615);
        assert_eq!(super::next_tick(10_000), 1_015);
        assert_eq!(super::next_tick(10_999), 16);
    }
}
