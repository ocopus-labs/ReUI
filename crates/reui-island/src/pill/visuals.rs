//! Visuals the pill animates itself rather than the renderer: the level bars and the
//! spinning logo mark.


use reui_design::tokens::{component, motion};
use reui_design::Color;
use reui_protocol::ImageId;
use windows::core::{h, Result};
use windows::Foundation::TimeSpan;
use windows::UI::Composition::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows_numerics::{Vector2, Vector3};

use crate::render::{BARS, BAR_LEVELS_LIVE, BAR_LEVEL_IDLE};

use super::{Pill, State};
use super::timers::*;

impl Pill {
    /// Draws and positions the animated compact logo (or hides it).
    pub(super) fn place_logo(&mut self, spot: Option<(ImageId, f32, f32)>, accent: Color) -> Result<()> {
        let s = self.scale;
        match spot {
            Some((image, left, top)) => {
                self.renderer.draw_logo(&self.logo_surface, image, component::art::COMPACT, accent)?;
                self.logo.SetOffset(Vector3::new((left * s).round(), (top * s).round(), 0.0))?;
                self.logo.SetIsVisible(true)?;
                let spinning = self.logo_state.is_some_and(|(_, spinning)| spinning);
                self.logo_state = Some((image, spinning));
            }
            None => {
                self.stop_logo();
                self.logo.SetIsVisible(false)?;
                self.logo_state = None;
            }
        }
        self.update_logo();
        Ok(())
    }

    /// Spins the logo while it's showing in compact; never when hidden or with reduced motion.
    pub(super) fn update_logo(&mut self) {
        let Some((image, spinning)) = self.logo_state else { return };
        let want = matches!(self.state, State::Compact | State::Peek) && self.shown && !self.reduced_motion;
        if want == spinning {
            return;
        }
        if want {
            // Ticks, not a continuous spin: DWM only works during each short turn.
            unsafe { SetTimer(Some(self.hwnd), TIMER_SPIN, motion::logo::INTERVAL, None) };
            self.tick_logo();
        } else {
            self.stop_logo();
        }
        self.logo_state = Some((image, want));
    }

    /// One eased quarter turn from wherever the last tick stopped.
    pub(super) fn tick_logo(&mut self) {
        let from = self.logo_angle;
        let to = from + 90.0;
        self.logo_angle = to % 360.0;
        let result = self.compositor.CreateScalarKeyFrameAnimation().and_then(|a| {
            let ease = self.compositor.CreateCubicBezierEasingFunction(Vector2::new(0.4, 0.0), Vector2::new(0.2, 1.0))?;
            a.InsertKeyFrame(0.0, from)?;
            a.InsertKeyFrameWithEasingFunction(1.0, to, &ease)?;
            a.SetDuration(TimeSpan { Duration: motion::logo::TURN as i64 * 10_000 })?;
            self.logo.StartAnimation(h!("RotationAngleInDegrees"), &a)
        });
        if let Err(e) = result {
            eprintln!("island: logo animation failed: {e}");
        }
    }

    fn stop_logo(&mut self) {
        unsafe { _ = KillTimer(Some(self.hwnd), TIMER_SPIN) };
        _ = self.logo.StopAnimation(h!("RotationAngleInDegrees"));
        _ = self.logo.SetRotationAngleInDegrees(0.0);
        self.logo_angle = 0.0;
    }

    /// Positions every set of bars where the renderer laid them out (or hides them).
    pub(super) fn place_bars(
        &mut self,
        compact: Option<(f32, f32, bool)>,
        card: Option<(f32, f32, bool)>,
        minimal: Option<(f32, f32, bool)>,
        front_accent: Color,
        second_accent: Color,
    ) -> Result<()> {
        let s = self.scale;
        for (set, spot) in [(&self.bars, compact), (&self.card_bars, card), (&self.minimal_bars, minimal)] {
            for (i, bar) in set.iter().enumerate() {
                bar.SetIsVisible(spot.is_some())?;
                if let Some((left, mid, _)) = spot {
                    let x = left + i as f32 * (component::bars::WIDTH + component::bars::GAP);
                    bar.SetOffset(Vector3::new((x * s).round(), ((mid - component::bars::HEIGHT / 2.0) * s).round(), 0.0))?;
                }
            }
        }
        for (brush, accent) in [(&self.bar_brush, front_accent), (&self.minimal_brush, second_accent)] {
            let [r, g, b, a] = accent.to_rgba8();
            brush.SetColor(windows::UI::Color { A: a, R: r, G: g, B: b })?;
        }
        self.front_bars = compact.or(card).map(|(_, _, live)| live);
        self.minimal_bars_live = minimal.map(|(_, _, live)| live);
        self.rest_bars();
        self.update_bars();
        Ok(())
    }

    /// Which bar sets should be moving right now: the front's (pill or open card) and the
    /// minimal bubble's (hidden while the card is open).
    fn moving_sets(&self) -> (bool, bool) {
        if !self.shown || self.reduced_motion {
            return (false, false);
        }
        let front = self.front_bars == Some(true) && self.state != State::Idle;
        let minimal = self.minimal_bars_live == Some(true) && self.minimal && self.state != State::Expanded;
        (front, minimal)
    }

    /// Runs the frame clock while any set is moving; rests them otherwise.
    pub(super) fn update_bars(&mut self) {
        let (front, minimal) = self.moving_sets();
        // Capture follows the moving bars: one process at a time, the front's first. It
        // stops (within its poll) when they rest, hide or switch to another process.
        let pid = if front { self.front_audio } else { None }.or(if minimal { self.minimal_audio } else { None });
        self.visualizer.follow(pid);
        let want = front || minimal;
        if want == self.bars_moving {
            return;
        }
        self.bars_moving = want;
        unsafe {
            if want {
                // A fixed frame rate (see `frame_interval`; 30 fps lands on 31.25 ms).
                SetTimer(Some(self.hwnd), TIMER_BARS, frame_interval(motion::bars::FPS), None);
            } else {
                _ = KillTimer(Some(self.hwnd), TIMER_BARS);
                self.rest_bars();
            }
        }
    }

    /// One frame. Bars whose process is being captured show its bands (bass on the left);
    /// the rest (or all, until the capture hears something) follow their own smooth
    /// two-sine waves. Only visible sets move.
    pub(super) fn tick_bars(&self) {
        const WAVES: [(f32, f32, f32, f32); BARS] = [(1.7, 0.0, 3.1, 1.2), (2.3, 2.1, 1.3, 0.4), (1.1, 4.0, 2.9, 2.6), (2.9, 1.0, 1.9, 5.1)];
        let t = self.bars_clock.elapsed().as_secs_f32() * std::f32::consts::TAU;
        let (front, minimal) = self.moving_sets();
        let front_set = if self.state == State::Expanded { &self.card_bars } else { &self.bars };
        let real = self.visualizer.levels();
        let captured = self.visualizer.pid();
        let sets = [(front_set, front, 0.0, self.front_audio), (&self.minimal_bars, minimal, 0.9, self.minimal_audio)];
        for (set, moving, phase, pid) in sets {
            if !moving {
                continue;
            }
            let bands = real.filter(|_| pid.is_some() && pid == captured);
            for (i, (bar, (f1, p1, f2, p2))) in set.iter().zip(WAVES).enumerate() {
                let level = match bands {
                    Some(bands) => BAR_LEVEL_IDLE + (1.0 - BAR_LEVEL_IDLE) * bands[i.min(bands.len() - 1)],
                    None => (0.6 + 0.25 * (t * f1 + p1 + phase).sin() + 0.15 * (t * f2 + p2).sin()).clamp(BAR_LEVEL_IDLE, 1.0),
                };
                _ = bar.SetScale(Vector3::new(1.0, level, 1.0));
            }
        }
    }

    fn rest_bars(&self) {
        for (set, live) in [(&self.bars, self.front_bars), (&self.card_bars, self.front_bars), (&self.minimal_bars, self.minimal_bars_live)] {
            for (bar, level) in set.iter().zip(BAR_LEVELS_LIVE) {
                _ = bar.SetScale(Vector3::new(1.0, if live == Some(true) { level } else { BAR_LEVEL_IDLE }, 1.0));
            }
        }
    }
}

/// Four rounded level bars, hidden until placed, scaling around their vertical centre.
pub(super) fn bar_visuals(c: &Compositor, parent: &VisualCollection, brush: &CompositionColorBrush, s: f32) -> Result<Vec<SpriteVisual>> {
    let (bw, bh) = (component::bars::WIDTH * s, component::bars::HEIGHT * s);
    let mut bars = Vec::with_capacity(BARS);
    for _ in 0..BARS {
        let bar = c.CreateSpriteVisual()?;
        bar.SetBrush(brush)?;
        bar.SetSize(Vector2::new(bw, bh))?;
        bar.SetCenterPoint(Vector3::new(bw / 2.0, bh / 2.0, 0.0))?;
        let rounded = c.CreateRoundedRectangleGeometry()?;
        rounded.SetSize(Vector2::new(bw, bh))?;
        rounded.SetCornerRadius(Vector2::new(bw / 2.0, bw / 2.0))?;
        bar.SetClip(&c.CreateGeometricClipWithGeometry(&rounded)?)?;
        bar.SetIsVisible(false)?;
        parent.InsertAtTop(&bar)?;
        bars.push(bar);
    }
    Ok(bars)
}
