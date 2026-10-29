//! The Look: the user's appearance choices (style, theme, roundness, gap from the top,
//! padding, size, accent) resolved into the colours and geometry a surface draws with.
//!
//! Pure, and shared: the island draws with it and Settings' previews draw the same thing,
//! so a preview never disagrees with the island. The settings keys and their spellings
//! (`settings.json`: `style`, `theme`, `roundness`, `top_gap`, `padding`, `scale`,
//! `accent`) live here too, so both programs read and write them the same way.
//!
//! Concentric corners (DESIGN.md) are the rule every combination keeps: a shape one inset
//! from an outer edge takes `outer - inset`, never less than `radius.min`, and a control
//! is a capsule only while that radius allows it.

use crate::tokens::{self, island, look as t, radius, Palette};
use crate::Color;

/// Floating island, or a notch joined to the top edge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Style {
    #[default]
    Island,
    Notch,
}

/// The setting; [`Look::new`] resolves Auto with Windows' app mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Theme {
    Auto,
    #[default]
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Padding {
    Compact,
    #[default]
    Regular,
    Roomy,
}

/// The default accent (for activities that don't send their own).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Accent {
    #[default]
    Reui,
    Windows,
}

macro_rules! names {
    ($ty:ty { $($variant:ident = $name:literal),+ $(,)? }) => {
        impl $ty {
            /// The settings value: [`Self::parse`] reads it back.
            pub const NAMES: &'static [&'static str] = &[$($name),+];
            pub const ALL: &'static [$ty] = &[$(<$ty>::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self { $(<$ty>::$variant => $name),+ }
            }

            /// Lenient about case and spaces, like every other settings value.
            pub fn parse(text: &str) -> Option<Self> {
                let text = text.trim().to_ascii_lowercase();
                match text.as_str() { $($name => Some(<$ty>::$variant),)+ _ => None }
            }

            pub fn index(self) -> usize {
                Self::ALL.iter().position(|v| *v == self).unwrap_or(0)
            }
        }
    };
}

names!(Style { Island = "island", Notch = "notch" });
names!(Theme { Auto = "auto", Dark = "dark", Light = "light" });
names!(Padding { Compact = "compact", Regular = "regular", Roomy = "roomy" });
names!(Accent { Reui = "reui", Windows = "windows" });

impl Padding {
    /// How much it scales the expanded insets and gaps.
    pub fn factor(self) -> f32 {
        match self {
            Padding::Compact => t::padding::COMPACT,
            Padding::Regular => 1.0,
            Padding::Roomy => t::padding::ROOMY,
        }
    }
}

/// What `settings.json` says about the look, each value clamped to what the island can draw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Appearance {
    pub style: Style,
    pub theme: Theme,
    /// 0 (square-ish, `look.roundness.min-radius`) to 1 (capsule, today's island).
    pub roundness: f32,
    /// Gap between the screen's top edge and the floating island, DIPs. A notch has none.
    pub top_gap: f32,
    pub padding: Padding,
    /// Every island dimension and the type, `look.size.min`..=`max`.
    pub size: f32,
    pub accent: Accent,
}

impl Default for Appearance {
    fn default() -> Self {
        Self { style: Style::Island, theme: Theme::Dark, roundness: 1.0, top_gap: island::TOP, padding: Padding::Regular, size: 1.0, accent: Accent::Reui }
    }
}

impl Appearance {
    /// Reads the keys through `text` and `number` (the caller's JSON), falling back to the
    /// default for anything missing, unknown or out of range.
    pub fn read(text: impl Fn(&str) -> Option<String>, number: impl Fn(&str) -> Option<f64>) -> Self {
        let d = Self::default();
        let num = |key: &str, lo: f32, hi: f32, fallback: f32| number(key).map(|n| n as f32).filter(|n| n.is_finite()).map_or(fallback, |n| n.clamp(lo, hi));
        Self {
            style: text("style").and_then(|s| Style::parse(&s)).unwrap_or(d.style),
            theme: text("theme").and_then(|s| Theme::parse(&s)).unwrap_or(d.theme),
            roundness: num("roundness", 0.0, 1.0, d.roundness),
            top_gap: num("top_gap", 0.0, t::top_gap::MAX, d.top_gap),
            padding: text("padding").and_then(|s| Padding::parse(&s)).unwrap_or(d.padding),
            size: num("scale", t::size::MIN, t::size::MAX, d.size),
            accent: text("accent").and_then(|s| Accent::parse(&s)).unwrap_or(d.accent),
        }
    }
}

/// Concentric radius for a shape `inset` inside an edge rounded `outer`, never under
/// `radius.min` (and never rounder than the edge itself).
pub fn concentric(outer: f32, inset: f32) -> f32 {
    (outer - inset).max(radius::MIN).min(outer.max(radius::MIN))
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// The resolved look: colours for one theme, and geometry from the shape settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub palette: Palette,
    pub light: bool,
    pub style: Style,
    pub roundness: f32,
    pub top_gap: f32,
    pub padding: Padding,
    pub size: f32,
    /// The accent of an activity that doesn't send one.
    pub accent: Color,
}

impl Default for Look {
    fn default() -> Self {
        Look::new(&Appearance::default(), false, None)
    }
}

impl Look {
    /// `apps_light`: Windows' app mode is light (for Auto). `windows_accent`: the Windows
    /// accent colour, when it could be read (for the Windows accent; ReUI blue otherwise).
    pub fn new(a: &Appearance, apps_light: bool, windows_accent: Option<Color>) -> Self {
        let light = match a.theme {
            Theme::Auto => apps_light,
            Theme::Dark => false,
            Theme::Light => true,
        };
        let accent = match a.accent {
            Accent::Windows => windows_accent.unwrap_or(tokens::color::accent::DEFAULT),
            Accent::Reui => tokens::color::accent::DEFAULT,
        };
        Look {
            palette: if light { tokens::palette::LIGHT } else { tokens::palette::DARK },
            light,
            style: a.style,
            roundness: a.roundness.clamp(0.0, 1.0),
            top_gap: a.top_gap,
            padding: a.padding,
            size: a.size,
            accent,
        }
    }

    pub fn notch(&self) -> bool {
        self.style == Style::Notch
    }

    /// Corner radius of a regular or roomy expanded card: `look.roundness.min-radius` up to
    /// the island's 44.
    pub fn radius(&self) -> f32 {
        lerp(t::roundness::MIN_RADIUS, island::expanded::RADIUS, self.roundness)
    }

    /// The wide panel's corners (clipboard), calmer than the card's in the same proportion.
    pub fn panel_radius(&self) -> f32 {
        lerp(t::roundness::MIN_RADIUS, island::expanded::PANEL_RADIUS, self.roundness)
    }

    /// Corner radius of a shape `h` tall that is a capsule at full roundness (the compact
    /// pill, the minimal bubble).
    pub fn rounded(&self, h: f32) -> f32 {
        let capsule = h / 2.0;
        lerp(t::roundness::MIN_RADIUS.min(capsule), capsule, self.roundness)
    }

    /// A control `h` tall one `inset` inside an edge rounded `outer`: a capsule while the
    /// concentric radius allows one (within `radius.capsule-slack`), else concentric.
    pub fn capsule(&self, h: f32, outer: f32, inset: f32) -> f32 {
        let c = concentric(outer, inset);
        if c + radius::CAPSULE_SLACK >= h / 2.0 {
            h / 2.0
        } else {
            c
        }
    }

    /// `v` scaled by the padding, in whole DIPs.
    pub fn padded(&self, v: f32) -> f32 {
        (v * self.padding.factor()).round()
    }

    /// Expanded padding on every side.
    pub fn inset(&self) -> f32 {
        self.padded(island::expanded::INSET)
    }

    pub fn column_gap(&self) -> f32 {
        self.padded(island::expanded::COLUMN_GAP)
    }

    pub fn section_gap(&self) -> f32 {
        self.padded(island::expanded::SECTION_GAP)
    }

    pub fn row_gap(&self) -> f32 {
        self.padded(island::expanded::ROW_GAP)
    }

    /// A card's tallest, grown with roomier padding so the same content still fits.
    pub fn max_height(&self, base: f32) -> f32 {
        (base * self.padding.factor().max(1.0)).ceil()
    }

    /// The gap above the island in DIPs at the look's size (the pill is drawn at
    /// `dpi * size`; the gap is set in DIPs at the display's scale). A notch has none.
    pub fn top(&self) -> f32 {
        if self.notch() {
            0.0
        } else {
            self.top_gap / self.size
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn look(roundness: f32, padding: Padding) -> Look {
        Look::new(&Appearance { roundness, padding, ..Appearance::default() }, false, None)
    }

    #[test]
    fn the_default_look_is_todays_island() {
        let l = Look::default();
        assert_eq!(l.palette, tokens::palette::DARK);
        assert_eq!(l.palette.background, tokens::color::island::BACKGROUND);
        assert_eq!(l.palette.label_primary, tokens::color::label::PRIMARY);
        assert_eq!((l.radius(), l.panel_radius(), l.inset()), (island::expanded::RADIUS, island::expanded::PANEL_RADIUS, island::expanded::INSET));
        assert_eq!((l.column_gap(), l.section_gap(), l.row_gap()), (island::expanded::COLUMN_GAP, island::expanded::SECTION_GAP, island::expanded::ROW_GAP));
        assert_eq!(concentric(l.radius(), l.inset()), radius::CONCENTRIC);
        assert_eq!(concentric(l.panel_radius(), l.inset()), tokens::component::card::RADIUS);
        assert_eq!(l.rounded(island::HEIGHT), island::HEIGHT / 2.0);
        assert_eq!(l.top(), island::TOP);
        assert_eq!(l.accent, tokens::color::accent::DEFAULT);
        assert_eq!(l.max_height(island::expanded::ROOMY_MAX_HEIGHT), island::expanded::ROOMY_MAX_HEIGHT);
    }

    #[test]
    fn todays_capsules_stay_capsules() {
        let l = Look::default();
        let (card, panel, inset) = (l.radius(), l.panel_radius(), l.inset());
        let b = tokens::component::button::HEIGHT;
        assert_eq!(l.capsule(b, card, inset), b / 2.0, "text buttons");
        assert_eq!(l.capsule(b, card, tokens::component::button::ROOMY_INSET), b / 2.0, "a roomy card's buttons");
        let field = tokens::component::search::HEIGHT;
        assert_eq!(l.capsule(field, panel, inset), field / 2.0, "the search field in the panel");
        let tab = tokens::component::tabs::HEIGHT;
        assert_eq!(l.capsule(tab, panel, inset), tab / 2.0);
        let circle = tokens::component::button::INLINE;
        assert_eq!(l.capsule(circle, card, inset), circle / 2.0);
    }

    #[test]
    fn every_combination_keeps_concentric_corners() {
        for r in [0.0, 0.25, 0.5, 0.75, 1.0] {
            for p in Padding::ALL.iter().copied() {
                let l = look(r, p);
                for outer in [l.radius(), l.panel_radius()] {
                    let inner = concentric(outer, l.inset());
                    assert!(inner >= radius::MIN && inner <= outer, "r {r} {p:?}: {inner} in {outer}");
                    if outer - l.inset() >= radius::MIN {
                        assert!((outer - l.inset() - inner).abs() < 1e-4, "parallel curves");
                    }
                    for h in [28.0, 32.0, 40.0, 44.0] {
                        let c = l.capsule(h, outer, l.inset());
                        assert!(c <= h / 2.0 && c >= radius::MIN.min(h / 2.0));
                        // Never rounder than concentric, except a capsule within the slack.
                        assert!(c <= inner + radius::CAPSULE_SLACK + 1e-4);
                    }
                }
                let compact = l.rounded(island::HEIGHT);
                assert!(compact <= island::HEIGHT / 2.0 && compact >= t::roundness::MIN_RADIUS.min(island::HEIGHT / 2.0));
                // Round content inset in the compact pill stays inside its end's curve.
                let art = concentric(compact, island::compact::SIDE_MARGIN).min(tokens::component::art::COMPACT / 2.0);
                assert!(art <= compact);
            }
        }
        // Square-ish: corners collapse to the minimum, never negative.
        let square = look(0.0, Padding::Roomy);
        assert_eq!(square.radius(), t::roundness::MIN_RADIUS);
        assert_eq!(concentric(square.radius(), square.inset()), radius::MIN);
    }

    #[test]
    fn padding_scales_insets_and_gaps_in_whole_dips() {
        let (c, r) = (look(1.0, Padding::Compact), look(1.0, Padding::Roomy));
        assert_eq!((c.inset(), r.inset()), (16.0, 24.0));
        assert_eq!((c.row_gap(), r.row_gap()), (10.0, 14.0));
        assert!(r.max_height(island::expanded::MAX_HEIGHT) > island::expanded::MAX_HEIGHT);
        assert_eq!(c.max_height(island::expanded::MAX_HEIGHT), island::expanded::MAX_HEIGHT);
    }

    #[test]
    fn auto_follows_the_app_mode_and_accent_falls_back_to_blue() {
        let auto = Appearance { theme: Theme::Auto, ..Appearance::default() };
        assert!(Look::new(&auto, true, None).light);
        assert!(!Look::new(&auto, false, None).light);
        assert_eq!(Look::new(&auto, true, None).palette, tokens::palette::LIGHT);
        let dark = Appearance { theme: Theme::Dark, ..Appearance::default() };
        assert!(!Look::new(&dark, true, None).light, "Dark ignores the app mode");
        let red = Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 };
        let windows = Appearance { accent: Accent::Windows, ..Appearance::default() };
        assert_eq!(Look::new(&windows, false, Some(red)).accent, red);
        assert_eq!(Look::new(&windows, false, None).accent, tokens::color::accent::DEFAULT);
        assert_eq!(Look::new(&Appearance::default(), false, Some(red)).accent, tokens::color::accent::DEFAULT);
    }

    #[test]
    fn settings_values_parse_leniently_and_clamp() {
        let json = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| pairs.iter().find(|(key, _)| *key == k).map(|(_, v)| v.to_string())
        };
        let nums = |pairs: &'static [(&'static str, f64)]| move |k: &str| pairs.iter().find(|(key, _)| *key == k).map(|(_, v)| *v);
        let a = Appearance::read(json(&[("style", " Notch "), ("theme", "LIGHT"), ("padding", "roomy"), ("accent", "windows")]), nums(&[("roundness", 3.0), ("top_gap", -5.0), ("scale", 2.0)]));
        assert_eq!((a.style, a.theme, a.padding, a.accent), (Style::Notch, Theme::Light, Padding::Roomy, Accent::Windows));
        assert_eq!((a.roundness, a.top_gap, a.size), (1.0, 0.0, t::size::MAX));
        let junk = Appearance::read(json(&[("style", "bar"), ("theme", "")]), nums(&[("roundness", f64::NAN)]));
        assert_eq!(junk, Appearance::default());
        for name in Theme::NAMES {
            assert_eq!(Theme::parse(name).unwrap().as_str(), *name);
        }
        let notch = Look::new(&Appearance { style: Style::Notch, top_gap: 20.0, ..Appearance::default() }, false, None);
        assert_eq!(notch.top(), 0.0, "a notch is flush with the edge");
        let big = Look::new(&Appearance { size: 1.25, top_gap: 10.0, ..Appearance::default() }, false, None);
        assert_eq!(big.top() * big.size, 10.0, "the gap stays in display DIPs at any size");
    }
}
