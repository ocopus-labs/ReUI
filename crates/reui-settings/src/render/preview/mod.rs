//! Each page's island, on a stage at the top of its header card. The stage is the top of
//! the user's screen: their own wallpaper's top centre, 1 DIP to 1 DIP ([`crate::wallpaper`]),
//! its top edge the screen's. One island hangs from it where the island really sits
//! (centred, the gap below the edge, or joined to it as a notch), at its real size, in
//! the user's look, showing the one moment the page is about. When a page opens, the
//! island drops in from the edge once; at rest nothing moves.
//!
//! Shapes are in the island's own units (inside [`Painter::scaled`] by the look's size).

mod agents;
mod clipboard;
mod compact;
mod devices;
mod levels;
mod music;
mod parts;
mod timers;

use reui_design::tokens::{motion, prefs as s};
use reui_design::Color;
use windows::core::Result;

use super::Painter;
use crate::ui::layout::Rect;
use crate::ui::{stage, Moment, Preview};
use crate::wallpaper::Backdrop;

pub fn draw(p: &Painter, preview: Preview, r: Rect, backdrop: Option<&Backdrop>, enter: f32) -> Result<()> {
    let dim = Color { r: 0.0, g: 0.0, b: 0.0, a: s::opacity::STAGE_DIM };
    p.backdrop(r, s::group::RADIUS, backdrop, s::color::STAGE, dim)?;
    let look = &preview.look;
    let e = ease_out(enter);
    // Centred on the screen; a second activity hangs beside the pill, not in the middle.
    let moment = stage::shown(preview, r.w());
    let (w, _) = stage::size(moment, look);
    let centred = match moment {
        Moment::Compact => stage::music_pill(),
        Moment::Toast => clipboard::toast_width(p),
        _ => w,
    };
    let x = p.snap((r.l + r.r) / 2.0 - centred * look.size / 2.0);
    // Dropping in from the screen's edge as the page opens.
    let top = p.snap(r.t + stage::top(look)) - (1.0 - e) * motion::show::RISE;
    p.alpha.set(e.min(1.0));
    let drawn = p.clip(r, || {
        p.scaled(x, top, look.size, || match moment {
            Moment::Compact => compact::draw(p, look, e),
            Moment::Music => music::draw(p, look, e),
            Moment::Clipboard => clipboard::draw(p, look),
            Moment::Toast => clipboard::toast(p, look),
            Moment::Meter => levels::draw(p, look),
            Moment::Devices => devices::draw(p, look, e),
            Moment::Timer => timers::draw(p, look, e),
            Moment::Permission => agents::draw(p, look),
        })
    });
    p.alpha.set(1.0);
    drawn
}

pub fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// An `Rgb` an activity sends as its accent.
const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: 1.0 }
}
