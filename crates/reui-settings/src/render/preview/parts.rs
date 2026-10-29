//! The island's pieces as the previews draw them: the shape (floating, or a notch with its
//! ears), glyph tiles, rings, level bars, the header's text column, buttons, the timeline
//! and the volume slider. Where each piece goes is `reui_design::layout`, the maths the
//! island's renderer (`reui-island/src/render`) lays out with, so a preview is where the
//! island draws; only the drawing is this program's own (its Painter, its text). Island
//! units, inside [`Painter::scaled`].

use reui_design::layout::header::{self, Text};
use reui_design::layout::{self, compact, rows};
use reui_design::look::{concentric, Look};
use reui_design::tokens::{component, icon, notch, opacity, typography};
use reui_design::Color;
use windows::core::Result;

use crate::render::{Align, Painter};
use crate::ui::layout::{Font, IslandFont, Rect};

/// An open card being drawn: the look, the activity's accent and the card's own geometry.
pub struct Card<'a, 'p> {
    pub p: &'a Painter<'p>,
    pub look: &'a Look,
    pub accent: Color,
    /// Its size: a roomy card's (agent moments) buttons sit like an alert's.
    pub size: layout::Card,
    pub width: f32,
    /// The card's corner radius (the wide panel's is calmer).
    pub radius: f32,
}

impl<'a, 'p> Card<'a, 'p> {
    pub fn new(p: &'a Painter<'p>, look: &'a Look, accent: Color, size: layout::Card) -> Self {
        Card { p, look, accent, size, width: size.width(), radius: size.radius(look) }
    }
}

/// What a header's hero is.
pub enum Hero<'t> {
    Text(&'t str),
    /// A number that changes in place (a timer's clock): the value font, tabular.
    Value(&'t str),
    /// A command in a chip.
    Code(&'t str),
}

/// What a button does to its colours (`ButtonRole`).
#[derive(Clone, Copy)]
pub enum Role {
    Primary,
    Secondary,
    Accent,
    Plain,
}

/// One line of island text with its top at `top` (the island's convention).
#[allow(clippy::too_many_arguments)]
pub fn line(p: &Painter, text: &str, font: IslandFont, x: f32, top: f32, w: f32, align: Align, ink: Color) -> Result<()> {
    let h = crate::render::fonts::style(Font::Island(font)).line_height;
    let r = Rect::at(x, top, w, h);
    match font {
        // The island's tabular formats.
        IslandFont::Value | IslandFont::Compact | IslandFont::Caption | IslandFont::Fact => p.figures(text, Font::Island(font), r, align, ink),
        _ => p.text(text, Font::Island(font), r, align, ink),
    }
}

/// A Segoe Fluent Icons glyph, `size` square, its top-left at (`x`, `top`).
pub fn glyph(p: &Painter, glyph: char, size: f32, x: f32, top: f32, ink: Color) -> Result<()> {
    p.glyph(glyph, size, Rect::at(x, top, size, size), ink)
}

/// The island's shape `r`: a rounded rectangle with the theme's hairline, or in the Notch
/// style one whose top is flush and square, with ears where it meets the edge (`ears`:
/// left, right).
pub fn shape(p: &Painter, look: &Look, r: Rect, radius: f32, ears: (bool, bool)) -> Result<()> {
    let pal = &look.palette;
    p.fill(r, radius, pal.background)?;
    if !look.notch() {
        return if pal.hairline.a > 0.0 { p.edge(r, radius, pal.hairline) } else { Ok(()) };
    }
    p.fill(Rect::new(r.l, r.t, r.r, r.t + radius.min(r.h())), 0.0, pal.background)?;
    if pal.hairline.a > 0.0 {
        // Along the sides and bottom only: the top is the screen's edge.
        p.clip(Rect::new(r.l - 1.0, r.t + radius.min(r.h() / 2.0), r.r + 1.0, r.b + 1.0), || p.edge(r, radius, pal.hairline))?;
        let side = |x: f32| p.fill(Rect::new(x, r.t, x + p.pixel(), r.t + radius.min(r.h() / 2.0)), 0.0, pal.hairline);
        side(r.l)?;
        side(r.r - p.pixel())?;
    }
    if ears.0 {
        ear(p, look, r.l, r.t, -1.0)?;
    }
    if ears.1 {
        ear(p, look, r.r, r.t, 1.0)?;
    }
    Ok(())
}

/// A concave ear at the top of a notch's side at `x`: the square beside it minus a quarter
/// circle, so the side curves out into the screen's edge (`notch.rs`). `dir` -1 is the
/// left. The theme's hairline follows the curve.
fn ear(p: &Painter, look: &Look, x: f32, top: f32, dir: f32) -> Result<()> {
    let e = notch::EAR;
    let (cx, cy) = (x + dir * e, top + e);
    let curve = |radius: f32| -> Vec<(f32, f32)> {
        const STEPS: usize = 16;
        (0..=STEPS)
            .map(|i| {
                // From the notch's side (angle 0 or 180°) up to the edge (270°).
                let t = i as f32 / STEPS as f32 * std::f32::consts::FRAC_PI_2;
                (cx - dir * t.cos() * radius, cy - t.sin() * radius)
            })
            .collect()
    };
    // With one DIP of material over the notch's side, hiding its hairline where they join.
    let mut points = vec![(x + dir * e, top), (x - dir, top), (x - dir, top + e), (x, top + e)];
    points.extend(curve(e));
    p.polygon(&points, look.palette.background)?;
    let hairline = look.palette.hairline;
    if hairline.a > 0.0 {
        let half = p.pixel() / 2.0;
        let along = curve(e + half);
        for pair in along.windows(2) {
            p.segment(pair[0], pair[1], p.pixel(), hairline)?;
        }
    }
    Ok(())
}

/// Artwork's stand-in, as the island draws `Art::Glyph`: the accent-tinted tile with the
/// glyph in the accent. `compact`: the pill's 24 circle, else the card's 56 tile.
#[allow(clippy::too_many_arguments)]
pub fn glyph_tile(p: &Painter, look: &Look, card_radius: f32, compact: bool, g: char, x: f32, top: f32, accent: Color) -> Result<()> {
    let (size, corner, glyph_size) = if compact {
        (component::art::COMPACT, compact::art_corner(look), icon::TILE_COMPACT)
    } else {
        (component::art::EXPANDED, concentric(card_radius, look.inset()), icon::TILE)
    };
    p.fill(Rect::at(x, top, size, size), corner, accent.with_alpha(opacity::TINT_FILL))?;
    glyph(p, g, glyph_size, x + (size - glyph_size) / 2.0, top + (size - glyph_size) / 2.0, accent)
}

/// A ring on the theme's track, the accent over `value` of it, clockwise from 12.
#[allow(clippy::too_many_arguments)]
pub fn ring(p: &Painter, look: &Look, cx: f32, cy: f32, size: f32, stroke: f32, value: f32, accent: Color) -> Result<()> {
    let radius = (size - stroke) / 2.0;
    p.stroke(Rect::new(cx - radius, cy - radius, cx + radius, cy + radius), radius, stroke, look.palette.fill_track)?;
    p.arc(cx, cy, radius, stroke, value, accent)
}

/// Four level bars from `x`, centred on `mid`, at their resting heights (`e` grows them
/// in as the page opens).
pub fn bars(p: &Painter, x: f32, mid: f32, accent: Color, e: f32) -> Result<()> {
    let (w, gap, h) = (component::bars::WIDTH, component::bars::GAP, component::bars::HEIGHT);
    for (i, level) in compact::BAR_LEVELS.iter().enumerate() {
        let bh = h * level * (0.4 + 0.6 * e);
        let l = x + i as f32 * (w + gap);
        p.fill(Rect::new(l, mid - bh / 2.0, l + w, mid + bh / 2.0), w / 2.0, accent)?;
    }
    Ok(())
}

impl Card<'_, '_> {
    /// The card's shape, from the top-left; a notch has both ears.
    pub fn shape(&self, h: f32) -> Result<()> {
        shape(self.p, self.look, Rect::at(0.0, 0.0, self.width, h), self.radius, (true, true))
    }

    /// Radius of a shape one inset inside the card's edge.
    pub fn concentric(&self) -> f32 {
        concentric(self.radius, self.look.inset())
    }

    /// The text column: eyebrow (accent) → hero (primary) → detail (secondary), laid out
    /// as `lines` (which has the eyebrow and detail the strings do), from `top`, `w` wide.
    #[allow(clippy::too_many_arguments)]
    pub fn text(&self, lines: &Text, x: f32, top: f32, w: f32, eyebrow: &str, hero: Hero, detail: &str) -> Result<()> {
        let (p, pal) = (self.p, &self.look.palette);
        let mut y = top;
        if !eyebrow.is_empty() {
            line(p, eyebrow, IslandFont::Eyebrow, x, y, w, Align::Start, self.accent)?;
            y += lines.eyebrow;
        }
        match hero {
            Hero::Text(t) => line(p, t, IslandFont::Headline, x, y, w, Align::Start, pal.label_primary)?,
            Hero::Value(t) => line(p, t, IslandFont::Value, x, y, w, Align::Start, pal.label_primary)?,
            Hero::Code(t) => {
                let (px, py) = (component::chip::PADDING_X, component::chip::PADDING_Y);
                let chip_w = (p.measure(t, Font::Island(IslandFont::Code)) + 2.0 * px).min(w);
                p.fill(Rect::at(x, y, chip_w, lines.hero), reui_design::tokens::radius::CHIP.min(self.concentric()), pal.fill_tertiary)?;
                line(p, t, IslandFont::Code, x + px, y + py, chip_w - 2.0 * px, Align::Start, pal.label_primary)?;
            }
        }
        y += lines.hero;
        if !detail.is_empty() {
            line(p, detail, IslandFont::Detail, x, y + reui_design::tokens::island::expanded::LINE_GAP, w, Align::Start, pal.label_secondary)?;
        }
        Ok(())
    }

    /// A button's fill and ink for its role.
    fn colors(&self, role: Role) -> (Option<Color>, Color) {
        let pal = &self.look.palette;
        match role {
            Role::Primary => (Some(pal.label_primary), pal.label_on_accent),
            Role::Secondary => (Some(pal.fill_secondary), pal.label_primary),
            Role::Accent => (Some(self.accent.with_alpha(opacity::TINT_FILL)), self.accent),
            Role::Plain => (None, pal.label_primary),
        }
    }

    /// A glyph button: a circle (or the concentric shape the roundness allows).
    pub fn glyph_button(&self, g: char, role: Role, x: f32, y: f32, size: f32) -> Result<()> {
        let (fill, ink) = self.colors(role);
        if let Some(fill) = fill {
            self.p.fill(Rect::at(x, y, size, size), self.look.capsule(size, self.radius, rows::button_inset(self.look, self.size)), fill)?;
        }
        let s = icon::BUTTON;
        glyph(self.p, g, s, x + (size - s) / 2.0, y + (size - s) / 2.0, ink)
    }

    /// A row of text buttons, equal capsules across the card at `y`.
    pub fn text_buttons(&self, buttons: &[(&str, Role)], y: f32) -> Result<()> {
        let row = rows::text_buttons(self.look, self.size, self.width, buttons.len());
        let (h, w, inset) = (row.height, row.width, rows::button_inset(self.look, self.size));
        for (i, (label, role)) in buttons.iter().enumerate() {
            let x = row.x(i);
            let (fill, ink) = self.colors(*role);
            if let Some(fill) = fill {
                self.p.fill(Rect::at(x, y, w, h), self.look.capsule(h, self.radius, inset), fill)?;
            }
            let lh = typography::BUTTON.line_height;
            line(self.p, label, IslandFont::Button, x, y + (h - lh) / 2.0, w, Align::Center, ink)?;
        }
        Ok(())
    }

    /// A bar on the theme's track from `l` to `r`, filled to `fraction` in the accent.
    pub fn meter(&self, l: f32, r: f32, y: f32, fraction: f32) -> Result<()> {
        let h = component::progress::HEIGHT;
        self.p.fill(Rect::new(l, y, r, y + h), h / 2.0, self.look.palette.fill_track)?;
        self.p.fill(Rect::new(l, y, l + ((r - l) * fraction).max(h), y + h), h / 2.0, self.accent)
    }

    /// A timeline's times at the card's insets, centred on `mid` (tertiary).
    pub fn captions(&self, leading: &str, trailing: &str, mid: f32) -> Result<()> {
        let (l, half) = rows::captions(self.look, self.width);
        let top = mid - typography::CAPTION.line_height / 2.0;
        let ink = self.look.palette.label_tertiary;
        line(self.p, leading, IslandFont::Caption, l, top, half, Align::Start, ink)?;
        line(self.p, trailing, IslandFont::Caption, l + half, top, half, Align::End, ink)
    }

    /// Glyph buttons as transport circles, centred as a group at `y`; returns their height.
    pub fn transport(&self, buttons: &[(char, Role)], y: f32) -> Result<f32> {
        let t = rows::transport(self.width, buttons.len());
        let mut x = t.left;
        for (g, role) in buttons {
            self.glyph_button(*g, *role, x, y, t.size)?;
            x += t.size + t.gap;
        }
        Ok(t.size)
    }

    /// The volume row: the mute button, the bar, the output menu's button.
    pub fn slider(&self, leading: char, value: f32, trailing: char, y: f32) -> Result<()> {
        let row = rows::slider(self.look, self.width, true);
        let h = row.size;
        self.glyph_button(leading, Role::Plain, row.button, y, h)?;
        if let Some(x) = row.menu {
            self.glyph_button(trailing, Role::Plain, x, y, h)?;
        }
        self.meter(row.bar.0, row.bar.1, y + row.bar_top, value)
    }

    /// Where the header's text starts and how wide it is, beside a leading `lead` wide and
    /// a trailing `trail` wide.
    pub fn columns(&self, lead: f32, trail: f32) -> (f32, f32) {
        header::columns(self.look, self.width, lead, trail)
    }
}
