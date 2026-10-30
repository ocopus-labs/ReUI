//! The file shelf on the pill (docs/PRD-file-shelf.md §5.1, §5.4): a drag from elsewhere
//! coming near makes the island hit-testable for it (and only then, and only near), the
//! pill grows into the drop target while Windows says the drag is over it, and a drop is
//! sent to the place that takes drops. The other way, a press on a card that carries
//! something (or on the compact pill, which carries every file its card holds) becomes a
//! drag out once it moves past the system's drag distance. The rules are
//! `features::drop`'s; the OLE side is `dragdrop/`.

use reui_design::tokens::{island, motion};
use reui_protocol::{Activity, ActivityId, Art, ClientMsg, CompactView, Destination, Drag, Element, Priority, Tone};
use windows::Win32::Foundation::POINT;
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SetTimer, SM_CXDRAG, SM_CYDRAG};

use crate::dragdrop::target::Event;
use crate::features::drop;
use crate::render;

use super::timers::TIMER_REVEAL;
use super::{Pill, State};

/// The drop target's face is the island's own, not an activity's.
const DROP_KEY: &str = "\u{0}drop";
const DROP_TEXT: &str = "Drop to hold";

#[derive(Default)]
pub(super) struct Drops {
    /// A drag passing by is near: the island is hit-testable for it.
    near: bool,
    /// The pill is the drop target now.
    pub showing: bool,
    /// A press that may become a drag out: where (screen pixels), and on which card (`None`:
    /// the compact pill).
    press: Option<(POINT, Option<usize>)>,
}

impl Pill {
    /// The place that takes drops, while there is one.
    fn drop_place(&self) -> Option<&Destination> {
        self.tabs.places.iter().find(|d| d.drops)
    }

    /// Where a drag counts as on the island (screen pixels, before the reach), or `None`
    /// when no drop target may appear: no place takes drops, ReUI is paused or hidden, a
    /// fullscreen app is in front, or a card other than the shelf's is open (a drag never
    /// folds a moment).
    fn drop_zone(&self) -> Option<(f32, f32, f32, f32)> {
        let place = self.drop_place()?;
        if self.paused || self.hidden_by_user || self.hidden {
            return None;
        }
        if self.state == State::Expanded {
            return self.front().filter(|a| a.id.module == place.module).map(|_| self.hit_rect());
        }
        Some(self.shape_rect())
    }

    fn in_drop_zone(&self, x: f32, y: f32) -> bool {
        self.drop_zone().is_some_and(|zone| drop::near(zone, island::shelf::REACH * self.scale, x, y))
    }

    /// The pointer moved (screen point): a drag from elsewhere near the island makes it
    /// hit-testable, so Windows finds it as a drop target; anywhere else it's click-through
    /// again at once, so the drop lands on whatever is under the island's empty area.
    pub(super) fn drop_near(&mut self, pt: POINT) {
        let near = self.drag_guard.blocks() && self.in_drop_zone(pt.x as f32, pt.y as f32);
        if near == self.drops.near {
            return;
        }
        self.drops.near = near;
        if near {
            self.set_clickthrough(false);
        } else if !self.inside {
            self.set_clickthrough(true);
        }
    }

    /// Windows' word on a drag over the island; `true` when the island takes it there.
    pub fn on_drop_event(&mut self, event: Event) -> bool {
        match event {
            Event::Enter(pt) | Event::Over(pt) => {
                let here = self.in_drop_zone(pt.x as f32, pt.y as f32);
                match (here, self.drops.showing) {
                    // Over the open shelf card a drop just adds a card; no target face.
                    (true, false) if self.state != State::Expanded => self.show_drop_target(),
                    (false, true) => self.hide_drop_target(),
                    _ => {}
                }
                here
            }
            Event::Leave => {
                self.hide_drop_target();
                false
            }
            Event::Dropped(items) => {
                let Some(module) = self.drop_place().map(|p| p.module.clone()) else { return false };
                _ = self.to_hub.send(ClientMsg::Drop { module, items });
                self.hide_drop_target();
                true
            }
        }
    }

    /// The target's face, in front of everything while it shows (`ordered`).
    pub(super) fn drop_face(&self) -> Option<Activity> {
        let place = self.drop_place().filter(|_| self.drops.showing)?;
        Some(Activity {
            id: ActivityId { module: place.module.clone(), key: DROP_KEY.into() },
            priority: Priority::Passive,
            accent: None,
            compact: CompactView { leading: Element::Art(Art::Glyph(place.glyph)), trailing: Element::Text { text: DROP_TEXT.into(), tone: Tone::Secondary } },
            minimal: Element::Empty,
            expanded: None,
            audio: None,
            // The drop target is the island answering a drag, not a moment to quiet.
            quiet: None,
        })
    }

    fn show_drop_target(&mut self) {
        self.drops.showing = true;
        if !self.shown {
            // A hidden visibility mode: the island comes out to take it.
            self.revealed = true;
            self.refresh_visibility();
        }
        self.rerender();
        self.set_state(State::Drop);
    }

    fn hide_drop_target(&mut self) {
        if !std::mem::replace(&mut self.drops.showing, false) {
            return;
        }
        self.rerender();
        self.set_state(State::Compact);
        if self.revealed {
            unsafe { SetTimer(Some(self.hwnd), TIMER_REVEAL, motion::delay::REVEAL_HOLD, None) };
        }
    }

    fn rerender(&mut self) {
        let ordered = self.ordered();
        if let Err(e) = self.render(ordered) {
            eprintln!("island: render failed: {e}");
        }
    }

    /// Debug builds: a drag coming over the pill's middle (1), leaving (0), or letting go
    /// of the files a test script put in `REUI_TEST_DROP` (2, `;`-separated), as Windows would
    /// report them (`WM_TEST_DROP`).
    #[cfg(debug_assertions)]
    pub fn test_drag(&mut self, step: usize) {
        let (l, t, r, b) = self.shape_rect();
        let event = match step {
            1 => Event::Enter(POINT { x: ((l + r) / 2.0) as i32, y: ((t + b) / 2.0) as i32 }),
            2 => {
                let files = std::env::var("REUI_TEST_DROP").unwrap_or_default().split(';').filter(|p| !p.is_empty()).map(str::to_string).collect();
                Event::Dropped(vec![reui_protocol::Dropped::Files(files)])
            }
            _ => Event::Leave,
        };
        self.on_drop_event(event);
    }

    /// A press at client point (`x`, `y`) on something that can be dragged out: an open
    /// card's card that carries something, or the compact pill whose card holds files.
    /// `true` when it was (the caller captures the mouse to see it move).
    pub fn press_drag(&mut self, x: f32, y: f32) -> bool {
        let screen = POINT { x: x as i32 + self.window.left, y: y as i32 + self.window.top };
        let card = match self.state {
            State::Expanded => {
                let (cx, cy) = self.card_point(x, y);
                match render::card_at(&self.hits, cx, cy) {
                    Some(i) if self.items().get(i).is_some_and(|it| it.drag.is_some()) => Some(i),
                    _ => return false,
                }
            }
            State::Compact | State::Peek => {
                let (l, t, r, b) = self.shape_rect();
                let (sx, sy) = (screen.x as f32, screen.y as f32);
                if !(sx >= l && sx < r && sy >= t && sy < b) || drop::drag_all(self.items()).is_none() {
                    return false;
                }
                None
            }
            _ => return false,
        };
        self.drops.press = Some((screen, card));
        true
    }

    /// The pointer moved to client point (`x`, `y`) with the button down: past the drag
    /// distance, what the press carries (and the press is over).
    pub fn drag_out_due(&mut self, x: f32, y: f32) -> Option<Drag> {
        let (from, card) = self.drops.press?;
        let to = (x as i32 + self.window.left, y as i32 + self.window.top);
        let distance = unsafe { (GetSystemMetrics(SM_CXDRAG), GetSystemMetrics(SM_CYDRAG)) };
        if !drop::moved_past((from.x, from.y), to, distance) {
            return None;
        }
        self.drops.press = None;
        match card {
            Some(i) => self.items().get(i).and_then(|it| it.drag.clone()),
            None => drop::drag_all(self.items()),
        }
    }

    /// The button came up before the press became a drag: `true` if there was one (the
    /// caller releases the mouse; the click goes on as usual).
    pub fn release_press(&mut self) -> bool {
        self.drops.press.take().is_some()
    }
}
