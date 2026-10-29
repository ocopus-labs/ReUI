//! The idle pill's faces (`features::idle`): nothing, the time, the date and time, or the
//! row of places. Drawn into the compact surface, centred like the compact presentation;
//! where things go is `layout::compact::clock` and `layout::switcher`.

use reui_design::layout::compact;
use reui_design::layout::switcher::Switcher;
use windows::core::Result;
use windows::UI::Composition::CompositionDrawingSurface;

use crate::features::idle::Face;
use crate::render::{Align, Renderer, Slot, COMPACT_SURFACE};

/// What a face needs drawn: the wall clock's text, or the places' glyphs and the tab under
/// the pointer.
pub struct IdleContent<'a> {
    pub date: &'a str,
    pub time: &'a str,
    pub places: &'a [char],
    pub hover: Option<usize>,
}

impl Renderer {
    /// Draws `face` and returns the pill's width for it; `rest` is the resting pill's
    /// width (the floor for the clock). The date is quiet beside a primary time on a peek;
    /// the time alone at rest is quiet too, so the resting island stays calm.
    pub fn draw_idle(&self, surface: &CompositionDrawingSurface, face: &Face, content: &IdleContent<'_>, rest: f32) -> Result<f32> {
        let mut width = rest;
        self.paint(surface, |rt| {
            let font = self.text_font(Slot::Compact);
            let top = (COMPACT_SURFACE.1 - font.style.line_height) / 2.0;
            match face {
                Face::Empty => {}
                Face::Time | Face::DateTime => {
                    let date = if *face == Face::DateTime { content.date } else { "" };
                    let date_w = if date.is_empty() { 0.0 } else { self.text_width(date, font)? };
                    let time_w = self.text_width(content.time, font)?;
                    let clock = compact::clock(date_w, time_w, rest);
                    let left = compact::left(clock.width);
                    if !date.is_empty() {
                        self.text(rt, date, font, left + clock.date_x, top, date_w, Align::Start, self.pal().label_secondary)?;
                    }
                    let ink = if date.is_empty() { self.pal().label_secondary } else { self.pal().label_primary };
                    self.text(rt, content.time, font, left + clock.time_x, top, time_w, Align::Start, ink)?;
                    width = clock.width;
                }
                Face::Places => {
                    let layout = Switcher::new(&self.look, content.places.len());
                    self.place_tabs(rt, &layout, compact::left(layout.width), content.places, None, content.hover)?;
                    width = layout.width;
                }
            }
            Ok(())
        })?;
        Ok(width)
    }
}
