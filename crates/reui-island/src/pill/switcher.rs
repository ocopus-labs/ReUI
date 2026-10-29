//! The view switcher under an open place's card (docs/PRD-view-switcher.md): the places
//! the hub offers, whether it shows (`features::switcher`), its tabs drawn and placed, and
//! a click, the wheel or Ctrl+Tab sending a place's own open command. It follows the
//! card's height spring in DWM and appears and goes on a settling spring, so nothing runs
//! while it stands still.

use reui_design::layout::switcher::{self as geometry, Switcher};
use reui_design::tokens::motion;
use reui_protocol::Destination;
use windows::core::{h, Result};
use windows::UI::Composition::{CompositionDrawingSurface, CompositionRoundedRectangleGeometry, ShapeVisual, SpriteVisual};
use windows::Win32::Foundation::POINT;
use windows_numerics::Vector2;

use crate::features::clipboard_keys;
use crate::features::music::WHEEL_NOTCH;
use crate::features::switcher::{self, Open};

use super::{Pill, State};

/// The switcher's visuals (built with the pill, `setup.rs`) and what it shows.
pub(super) struct Tabs {
    pub sprite: SpriteVisual,
    pub clip: CompositionRoundedRectangleGeometry,
    /// The light theme's hairline (none in dark).
    pub edge: Option<ShapeVisual>,
    pub surface: CompositionDrawingSurface,
    /// The places, as the hub last sent them.
    pub places: Vec<Destination>,
    /// Settings → General → View switcher.
    pub enabled: bool,
    /// The selected tab while it shows; `None` while it doesn't.
    pub selected: Option<usize>,
    /// The tab under the pointer.
    pub hover: Option<usize>,
    /// Laid out for this many tabs.
    pub layout: Switcher,
    /// What the surface last showed (glyphs, selected, hover), so nothing is redrawn twice.
    pub drawn: Option<(Vec<char>, Option<usize>, Option<usize>)>,
    /// Wheel travel toward a whole notch.
    pub travel: i32,
}

impl Pill {
    /// The places the hub offers now.
    pub(super) fn set_places(&mut self, places: Vec<Destination>) {
        if places != self.tabs.places {
            self.tabs.places = places;
            self.sync_switcher();
        }
    }

    /// The places, for a pill rebuilt at another scale or look.
    pub fn places(&self) -> Vec<Destination> {
        self.tabs.places.clone()
    }

    /// The `view_switcher` setting.
    pub(super) fn set_switcher_enabled(&mut self, on: bool) {
        if on != self.tabs.enabled {
            self.tabs.enabled = on;
            self.sync_switcher();
        }
    }

    /// Shows, hides or redraws the switcher for what's open now. Cheap when nothing
    /// changed, so it runs after every change of state, snapshot and visibility.
    pub(super) fn sync_switcher(&mut self) {
        let open = self.front().map(|a| Open {
            enabled: self.tabs.enabled,
            expanded: self.state == State::Expanded && self.shown,
            module: a.id.module.as_str(),
            priority: a.priority,
            summoned: self.summoned,
        });
        let selected = switcher::selected(&self.tabs.places, open);
        let was = self.tabs.selected.is_some();
        self.tabs.selected = selected;
        clipboard_keys::offer_views(selected.is_some());
        if selected.is_none() {
            self.tabs.hover = None;
        } else if let Err(e) = self.lay_out_switcher().and_then(|_| self.draw_switcher()) {
            eprintln!("island: switcher failed: {e}");
        }
        if was != selected.is_some() {
            // In on a calm spring under the opening card; out quickly, so it never hangs
            // under a card that's closing.
            let (to, preset) = match selected {
                Some(_) if self.reduced_motion => (1.0, motion::spring::REDUCED),
                Some(_) => (1.0, motion::spring::EXPAND),
                None => (0.0, motion::spring::REDUCED),
            };
            if let Err(e) = self.spring(h!("v"), to, preset) {
                eprintln!("island: spring failed: {e}");
            }
        }
    }

    /// Sizes the capsule for the number of places (rare: a module starting, Now Playing's
    /// card coming or going).
    fn lay_out_switcher(&mut self) -> Result<()> {
        let count = self.tabs.places.len().min(geometry::MAX_TABS);
        if count == self.tabs.layout.tabs.len() {
            return Ok(());
        }
        self.tabs.layout = Switcher::new(&self.look, count);
        let s = self.scale;
        let size = Vector2::new(self.tabs.layout.width * s, self.tabs.layout.height * s);
        self.tabs.sprite.SetSize(size)?;
        self.tabs.clip.SetSize(size)?;
        let r = self.tabs.layout.radius * s;
        self.tabs.clip.SetCornerRadius(Vector2::new(r, r))?;
        if let Some(edge) = &self.tabs.edge {
            edge.SetSize(size)?;
        }
        self.props.InsertScalar(h!("sw"), size.X)?;
        self.tabs.drawn = None;
        Ok(())
    }

    fn draw_switcher(&mut self) -> Result<()> {
        let glyphs: Vec<char> = self.tabs.places.iter().take(geometry::MAX_TABS).map(|p| p.glyph).collect();
        let now = (glyphs, self.tabs.selected, self.tabs.hover);
        if self.tabs.drawn.as_ref() == Some(&now) {
            return Ok(());
        }
        self.renderer.draw_switcher(&self.tabs.surface, &self.tabs.layout, &now.0, now.1, now.2)?;
        self.tabs.drawn = Some(now);
        Ok(())
    }

    /// The switcher's screen rectangle while it shows (where it's heading: under the card's
    /// target height).
    pub(super) fn switcher_rect(&self) -> Option<(f32, f32, f32, f32)> {
        self.tabs.selected?;
        let s = self.scale;
        let win_w = (self.window.right - self.window.left) as f32;
        let w = self.tabs.layout.width * s;
        let left = self.window.left as f32 + (win_w - w) / 2.0;
        let top = self.window.top as f32 + Switcher::top(self.look.top(), self.expanded_height) * s;
        Some((left, top, left + w, top + self.tabs.layout.height * s))
    }

    /// The switcher and the gap above it: on the island for hover, so moving from the card
    /// to a tab doesn't start the card's collapse.
    pub(super) fn switcher_zone(&self) -> Option<(f32, f32, f32, f32)> {
        let (l, _, r, b) = self.switcher_rect()?;
        Some((l, self.window.top as f32 + (self.look.top() + self.expanded_height) * self.scale, r, b))
    }

    /// The tab at screen point (`x`, `y`).
    fn switcher_tab(&self, x: f32, y: f32) -> Option<usize> {
        let (l, t, ..) = self.switcher_rect()?;
        self.tabs.layout.tab_at((x - l) / self.scale, (y - t) / self.scale)
    }

    /// A click at client point (`x`, `y`): on a tab it goes to that place. `true` when the
    /// click was the switcher's (the gap above it included), so nothing else takes it.
    pub(super) fn click_switcher(&mut self, x: f32, y: f32) -> bool {
        let (sx, sy) = (x + self.window.left as f32, y + self.window.top as f32);
        let Some((l, t, r, b)) = self.switcher_zone() else { return false };
        if !(sx >= l && sx < r && sy >= t && sy < b) {
            return false;
        }
        if let Some(i) = self.switcher_tab(sx, sy) {
            self.open_place(i);
        }
        true
    }

    /// The wheel at screen point `pt`: over the switcher a notch is the next or previous
    /// place. `true` when it was over it.
    pub(super) fn wheel_switcher(&mut self, delta: i32, pt: POINT) -> bool {
        if self.switcher_tab(pt.x as f32, pt.y as f32).is_none() {
            return false;
        }
        self.tabs.travel += delta;
        if self.tabs.travel.abs() >= WHEEL_NOTCH {
            let step = if self.tabs.travel < 0 { 1 } else { -1 };
            self.tabs.travel = 0;
            self.step_place(step);
        }
        true
    }

    /// The tab under the pointer (screen point) shows it; redrawn only when that changes.
    pub(super) fn hover_switcher(&mut self, pt: POINT) {
        let want = if self.shown { self.switcher_tab(pt.x as f32, pt.y as f32) } else { None };
        if want != self.tabs.hover {
            self.tabs.hover = want;
            if let Err(e) = self.draw_switcher() {
                eprintln!("island: switcher failed: {e}");
            }
        }
    }

    /// Ctrl+Tab (`1`) / Ctrl+Shift+Tab (`-1`), and the wheel.
    pub(super) fn step_place(&mut self, step: i32) {
        let count = self.tabs.places.len().min(geometry::MAX_TABS);
        if let Some(i) = geometry::step(count, self.tabs.selected, step) {
            self.open_place(i);
        }
    }

    /// Opens place `i` the way the tray does: its module publishes its card and asks for
    /// focus, and the card opens as summoned. The place already open stays as it is.
    fn open_place(&mut self, i: usize) {
        if self.tabs.selected.is_none() || self.tabs.selected == Some(i) {
            return;
        }
        if let Some(place) = self.tabs.places.get(i) {
            self.command(&place.module.clone(), &place.open.clone());
        }
    }
}
