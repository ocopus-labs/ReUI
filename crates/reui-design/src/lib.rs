//! ReUI design tokens. Edit `design/tokens.json`, not this crate: the build script turns
//! every token into a typed constant under [`tokens`], e.g. `tokens::color::label::PRIMARY`.
//! See `DESIGN.md` for what each token is for. Built on them: [`look`] (the user's
//! appearance resolved) and [`layout`] (the island's geometry), shared by every surface.
//! Also [`devices`]: the device drawings' bytes, one copy for the hub and Settings.

/// Straight (non-premultiplied) sRGB, channels 0..=1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const fn with_alpha(self, a: f32) -> Self {
        Self { a, ..self }
    }

    pub fn from_rgb8(r: u8, g: u8, b: u8) -> Self {
        Self { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: 1.0 }
    }

    /// This colour laid over an opaque `base`, as one opaque colour: what blending would
    /// show, worked out once. For fills over the island's own black, so no see-through
    /// colour is left for the display path to blend (some blend it lighter on screen than
    /// a capture shows).
    pub fn over(self, base: Color) -> Color {
        let a = self.a.clamp(0.0, 1.0);
        let mix = |top: f32, bottom: f32| top * a + bottom * (1.0 - a);
        Color { r: mix(self.r, base.r), g: mix(self.g, base.g), b: mix(self.b, base.b), a: 1.0 }
    }

    /// Channels as 0..=255 bytes (alpha included), for APIs that take 8-bit colour.
    pub fn to_rgba8(self) -> [u8; 4] {
        [self.r, self.g, self.b, self.a].map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    pub family: &'static str,
    pub weight: u16,
    /// DIPs
    pub size: f32,
    /// DIPs
    pub line_height: f32,
}

/// A composition natural-motion spring.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    pub damping_ratio: f32,
    pub period_ms: f32,
}

pub mod devices;
pub mod icon;
pub mod layout;
pub mod look;

#[allow(clippy::excessive_precision, clippy::approx_constant)]
pub mod tokens {
    include!(concat!(env!("OUT_DIR"), "/tokens.rs"));
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_see_through_fill_over_black_becomes_one_solid_grey() {
        // The dark theme's quietest well: #767680 at 16 % over black is about RGB 19.
        let well = Color::from_rgb8(0x76, 0x76, 0x80).with_alpha(0.16).over(Color::from_rgb8(0, 0, 0));
        assert_eq!(well.to_rgba8(), [19, 19, 20, 255]);
        // Opaque stays itself; fully clear is the base.
        let red = Color::from_rgb8(255, 0, 0);
        assert_eq!(red.over(Color::from_rgb8(0, 0, 255)), red);
        assert_eq!(red.with_alpha(0.0).over(Color::from_rgb8(0, 0, 255)).to_rgba8(), [0, 0, 255, 255]);
    }

    use super::tokens::*;
    use super::*;

    #[test]
    fn aliases_and_inherited_types_resolve() {
        assert_eq!(color::accent::DEFAULT, color::system::BLUE);
        assert_eq!(typography::VALUE.family, font::DISPLAY);
        assert_eq!(typography::VALUE.size, 34.0);
        assert_eq!(radius::CONCENTRIC, island::expanded::RADIUS - island::expanded::INSET);
        assert_eq!(color::label::PRIMARY.to_rgba8(), [0xEB, 0xEB, 0xF0, 0xFF]);
        assert!((color::fill::TRACK.a - 0.16).abs() < 1e-6);
        assert_eq!(motion::delay::COLLAPSE, 450);
        assert_eq!(motion::spring::EXPAND, Spring { damping_ratio: 1.0, period_ms: 60.0 });
    }
}
