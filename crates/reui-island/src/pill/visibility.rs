//! Size and presence: the spring morphs between states, the hit and shape rects,
//! click-through, visibility modes, reveal, fullscreen hiding and staying topmost.


use reui_design::tokens::{island, motion};
use reui_design::Spring;
use windows::core::{h, Interface, Result, HSTRING};
use windows::Foundation::{IReference, PropertyValue, TimeSpan};
use windows::Win32::Foundation::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows_numerics::{Vector2, Vector3};

use crate::desktop;
use crate::fullscreen;
use crate::place::{self, MonitorChoice, Placement};
use crate::settings::{self, Settings};
use reui_design::look::{Appearance, Look};

/// Posted to the island window when the look changed: it rebuilds the pill.
pub const WM_LOOK: u32 = WM_APP + 6;

use super::{Pill, State};
use super::timers::*;

impl Pill {
    fn state_size(&self, state: State) -> (f32, f32) {
        let core_h = island::HEIGHT;
        let grow = island::compact::PEEK_GROW;
        match state {
            State::Idle => (island::idle::WIDTH, core_h),
            State::Compact => (self.compact_width, core_h),
            State::Peek => (self.compact_width + grow, core_h + grow),
            State::Expanded => (self.expanded_width, self.expanded_height),
        }
    }

    /// Spring the pill to the current state's size, if it isn't already heading there.
    pub(super) fn retarget(&mut self) {
        let size = self.state_size(self.state);
        if size == self.target {
            return;
        }
        self.target = size;
        let preset = match self.state {
            _ if self.reduced_motion => motion::spring::REDUCED,
            State::Idle => motion::spring::SETTLE,
            State::Compact => motion::spring::COLLAPSE,
            State::Peek => motion::spring::PEEK,
            State::Expanded => motion::spring::EXPAND,
        };
        let height = Spring {
            damping_ratio: (preset.damping_ratio + motion::height::DAMPING_OFFSET).min(1.0),
            period_ms: preset.period_ms * motion::height::PERIOD_FACTOR,
        };
        let result = self.spring(h!("w"), size.0 * self.scale, preset).and_then(|_| self.spring(h!("h"), size.1 * self.scale, height));
        if let Err(e) = result {
            eprintln!("island: spring failed: {e}");
        }
    }

    pub(super) fn set_minimal(&mut self, on: bool) {
        if self.minimal == on {
            return;
        }
        self.minimal = on;
        self.update_bars();
        let preset = if self.reduced_motion { motion::spring::REDUCED } else { motion::spring::SETTLE };
        if let Err(e) = self.spring(h!("m"), if on { 1.0 } else { 0.0 }, preset) {
            eprintln!("island: spring failed: {e}");
        }
    }

    fn spring(&self, key: &HSTRING, to: f32, preset: Spring) -> Result<()> {
        let spring = self.compositor.CreateSpringScalarAnimation()?;
        spring.SetFinalValue(&PropertyValue::CreateSingle(to)?.cast::<IReference<f32>>()?)?;
        spring.SetDampingRatio(preset.damping_ratio)?;
        spring.SetPeriod(TimeSpan { Duration: (preset.period_ms * 10_000.0) as i64 })?;
        self.props.StartAnimation(key, &spring)
    }

    pub(super) fn set_clickthrough(&mut self, on: bool) {
        if self.clickthrough == on {
            return;
        }
        self.clickthrough = on;
        unsafe {
            let ex = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE);
            let flag = WS_EX_TRANSPARENT.0 as isize;
            SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, if on { ex | flag } else { ex & !flag });
        }
    }

    /// Screen rectangle that counts as "on the pill". Compact uses its peek size so the
    /// edge doesn't flicker as the pill grows, and it reaches the top of the screen.
    pub(super) fn hit_rect(&self) -> (f32, f32, f32, f32) {
        let state = if self.state == State::Compact { State::Peek } else { self.state };
        let (w, h) = self.state_size(state);
        let s = self.scale;
        let win_w = (self.window.right - self.window.left) as f32;
        let left = self.window.left as f32 + (win_w - w * s) / 2.0;
        let top = self.window.top as f32;
        (left, top, left + w * s, top + (self.look.top() + h) * s)
    }

    /// Screen rectangle of the pill as drawn (without the band above it).
    pub(super) fn shape_rect(&self) -> (f32, f32, f32, f32) {
        let (l, _, r, b) = self.hit_rect();
        (l, self.window.top as f32 + self.look.top() * self.scale, r, b)
    }

    /// Screen rectangle of the minimal bubble, when it's there to be clicked.
    pub(super) fn bubble_rect(&self) -> Option<(f32, f32, f32, f32)> {
        if !self.minimal || self.state == State::Expanded {
            return None;
        }
        let s = self.scale;
        let win_w = (self.window.right - self.window.left) as f32;
        let pill_w = self.state_size(self.state).0;
        let left = self.window.left as f32 + (win_w + pill_w * s) / 2.0 + island::minimal::GAP * s;
        let top = self.window.top as f32 + self.look.top() * s;
        let size = island::minimal::SIZE * s;
        Some((left, top, left + size, top + size))
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    /// Applies settings.json as it is now (at start, when the Settings window or the tray
    /// changed it, and when a pause runs out). Pausing also pauses clipboard recording, so
    /// nothing is captured while ReUI is out of the way.
    pub fn apply_settings(&mut self, settings: Settings) {
        let left = settings.pause_left(settings::now_ms());
        if left.is_some() != self.paused {
            self.paused = left.is_some();
            eprintln!("island: {}", if self.paused { "paused" } else { "resumed" });
            self.command("clipboard", if self.paused { "pause" } else { "resume" });
        }
        unsafe {
            match left {
                Some(ms) => _ = SetTimer(Some(self.hwnd), TIMER_PAUSE, ms.min(u64::from(u32::MAX)) as u32, None),
                None => _ = KillTimer(Some(self.hwnd), TIMER_PAUSE),
            }
        }
        if self.visibility != settings.visibility {
            eprintln!("island: showing {:?}", settings.visibility);
        }
        self.visibility = settings.visibility;
        self.show_on_desktop = settings.show_on_desktop;
        if self.hide_in_fullscreen != settings.hide_in_fullscreen {
            self.hide_in_fullscreen = settings.hide_in_fullscreen;
            self.update_fullscreen(unsafe { GetForegroundWindow() });
        }
        if self.appearance != settings.appearance {
            // A new look is a new composition tree and window size: main.rs rebuilds.
            unsafe { _ = PostMessageW(Some(self.hwnd), WM_LOOK, WPARAM(0), LPARAM(0)) };
        }
        if self.monitor_choice != settings.monitor {
            eprintln!("island: on {:?}", settings.monitor);
            self.monitor_choice = settings.monitor;
            unsafe { _ = PostMessageW(Some(self.hwnd), place::WM_PLACE, WPARAM(0), LPARAM(0)) };
        }
        self.refresh_visibility();
    }

    /// The look the pill was built with, and the settings it came from.
    pub fn look(&self) -> (&Look, &Appearance) {
        (&self.look, &self.appearance)
    }

    /// The `monitor` setting.
    pub fn monitor_choice(&self) -> &MonitorChoice {
        &self.monitor_choice
    }

    /// Moves the island to `placement` (another display, a changed work area or
    /// resolution) and puts it back on top. `false` when the display's scale differs: the
    /// pill is built for one scale, so the caller rebuilds it there instead.
    pub fn move_to(&mut self, placement: &Placement) -> bool {
        if (placement.scale - self.dpi).abs() > 0.001 {
            return false;
        }
        if placement.window != self.window || placement.monitor != self.monitor {
            let w = placement.window;
            unsafe {
                _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), w.left, w.top, w.right - w.left, w.bottom - w.top, SWP_NOACTIVATE | SWP_NOOWNERZORDER);
            }
            self.window = w;
            self.monitor = placement.monitor;
        }
        self.raise();
        self.update_fullscreen(unsafe { GetForegroundWindow() });
        true
    }

    /// Pause ReUI for an hour (the tray), or resume it.
    pub fn set_paused(&mut self, paused: bool) {
        let mut settings = Settings::load();
        settings.paused_until = paused.then(|| settings::now_ms() + settings::PAUSE_MS);
        settings.save();
        self.apply_settings(settings);
    }

    fn wants_shown(&self) -> bool {
        if self.hidden {
            return false;
        }
        // A pause quiets everything except questions only the user can answer (an agent
        // asking for permission); those still come through.
        if self.paused {
            return self.snapshot.activities.iter().any(|a| a.priority == reui_protocol::Priority::Decision);
        }
        self.revealed || self.desktop_shows() || self.visibility.shows(&self.snapshot.activities)
    }

    /// On the desktop with "Always show on the desktop": the island shows as if Always.
    pub(super) fn desktop_shows(&self) -> bool {
        self.show_on_desktop && self.on_desktop
    }

    /// Fades the island in or out to match the visibility mode, reveal and fullscreen.
    pub fn refresh_visibility(&mut self) {
        let show = self.wants_shown();
        if show == self.shown {
            return;
        }
        self.shown = show;
        if show {
            self.raise();
        }
        self.animate_presence(show);
        if !show {
            self.inside = false;
            self.hovered = false;
            self.set_clickthrough(true);
            unsafe { _ = KillTimer(Some(self.hwnd), TIMER_HOVER) };
            if self.state == State::Expanded {
                self.set_state(State::Compact);
            }
        }
        self.update_bars();
        self.update_logo();
        self.update_capture();
        self.update_clock_tick();
    }

    /// Emerges from the top edge (slide down, scale up with a little overshoot, fade in) or
    /// retracts into it (quicker, accelerating). Starts from wherever the island is now, so
    /// a quick show/hide/show reverses smoothly.
    fn animate_presence(&self, show: bool) {
        let from_scale = motion::show::FROM_SCALE;
        let (opacity, scale, rise) = if show { (1.0, 1.0, 0.0) } else { (0.0, from_scale, -motion::show::RISE * self.scale) };
        if self.reduced_motion {
            for property in [h!("Opacity"), h!("Scale"), h!("Offset")] {
                _ = self.root.StopAnimation(property);
            }
            _ = self.root.SetOpacity(opacity);
            _ = self.root.SetScale(Vector3::new(1.0, 1.0, 1.0));
            _ = self.root.SetOffset(Vector3::new(0.0, 0.0, 0.0));
            return;
        }
        let c = &self.compositor;
        let result = (|| -> Result<()> {
            let (duration, ease) = if show {
                // Decelerate with a slight overshoot: arrives, then settles.
                (motion::show::IN, c.CreateCubicBezierEasingFunction(Vector2::new(0.34, 1.4), Vector2::new(0.64, 1.0))?)
            } else {
                (motion::show::OUT, c.CreateCubicBezierEasingFunction(Vector2::new(0.4, 0.0), Vector2::new(1.0, 1.0))?)
            };
            let span = |ms: u32| TimeSpan { Duration: ms as i64 * 10_000 };

            let shape = c.CreateVector3KeyFrameAnimation()?;
            shape.InsertKeyFrameWithEasingFunction(1.0, Vector3::new(scale, scale, 1.0), &ease)?;
            shape.SetDuration(span(duration))?;
            self.root.StartAnimation(h!("Scale"), &shape)?;

            let slide = c.CreateVector3KeyFrameAnimation()?;
            slide.InsertKeyFrameWithEasingFunction(1.0, Vector3::new(0.0, rise, 0.0), &ease)?;
            slide.SetDuration(span(duration))?;
            self.root.StartAnimation(h!("Offset"), &slide)?;

            let fade = c.CreateScalarKeyFrameAnimation()?;
            fade.InsertKeyFrame(1.0, opacity)?;
            fade.SetDuration(span(motion::delay::FADE))?;
            // Fading out waits until the shape has mostly retracted.
            if !show {
                fade.SetDelayTime(span(duration.saturating_sub(motion::delay::FADE)))?;
            }
            self.root.StartAnimation(h!("Opacity"), &fade)
        })();
        if let Err(e) = result {
            eprintln!("island: presence animation failed: {e}");
            _ = self.root.SetOpacity(opacity);
        }
    }

    pub fn on_foreground(&mut self, fg: HWND) {
        if self.ignore_foreground.take() == Some(fg) {
            return;
        }
        // Switching apps means the summoned card is done with.
        self.dismiss_summoned();
        let on_desktop = desktop::on_desktop(desktop::front(fg), self.on_desktop);
        if on_desktop != self.on_desktop {
            self.on_desktop = on_desktop;
            self.refresh_visibility();
        }
        if !self.update_fullscreen(fg) {
            self.raise();
        }
    }

    /// The foreground window moved or resized (a browser video going fullscreen keeps the
    /// same window), or the setting changed: hide or show for fullscreen.
    pub fn recheck_fullscreen(&mut self) {
        self.update_fullscreen(unsafe { GetForegroundWindow() });
    }

    /// Hides while something fullscreen is in front of the island's display. Returns
    /// whether it's hidden.
    fn update_fullscreen(&mut self, fg: HWND) -> bool {
        let (notify, foreground, monitor) = fullscreen::read(fg, self.monitor);
        let hide = fullscreen::hides(self.hide_in_fullscreen, notify, foreground.as_ref(), &monitor);
        if hide != self.hidden {
            self.hidden = hide;
            self.refresh_visibility();
        }
        hide
    }

    /// Puts the island back at the top of the always-on-top band. Being topmost once isn't
    /// enough: the taskbar, Start, pop-ups and apps that make themselves topmost later all
    /// land above it, so this runs whenever the foreground changes and when it shows.
    fn raise(&self) {
        unsafe {
            _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER);
        }
    }
}
