//! Where Windows puts the wallpaper image on the primary monitor, for each fit mode, and
//! which parts of it land in the strip across the monitor's top centre that the previews
//! show. Pure maths in physical pixels; [`super`] does the reading, decoding and copying.

/// Windows' fit modes (`DESKTOP_WALLPAPER_POSITION`), named as Settings names them.
#[allow(clippy::enum_variant_names)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    /// Native size, centred; the background colour around it.
    Center,
    /// Native size, repeated from the monitor's top-left.
    Tile,
    /// Scaled to the monitor, aspect ignored.
    Stretch,
    /// Scaled to fit inside, centred; the background colour in the bars.
    Fit,
    /// Scaled to cover, centred, the overflow cropped.
    Fill,
    /// One image across every monitor: filling the virtual desktop's bounds.
    Span,
}

impl Fit {
    /// From `DESKTOP_WALLPAPER_POSITION`'s value; Fill (Windows' default) for anything else.
    pub fn from_position(value: i32) -> Fit {
        match value {
            0 => Fit::Center,
            1 => Fit::Tile,
            2 => Fit::Stretch,
            3 => Fit::Fit,
            5 => Fit::Span,
            _ => Fit::Fill,
        }
    }

    /// From the registry (`Control Panel\Desktop`: `WallpaperStyle`, `TileWallpaper`), when
    /// IDesktopWallpaper isn't there.
    pub fn from_registry(style: &str, tile: &str) -> Fit {
        match (style.trim(), tile.trim()) {
            ("0", "1") => Fit::Tile,
            ("0", _) => Fit::Center,
            ("2", _) => Fit::Stretch,
            ("6", _) => Fit::Fit,
            ("22", _) => Fit::Span,
            _ => Fit::Fill,
        }
    }
}

/// A rectangle in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Area {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Area { x, y, w, h }
    }

    fn intersect(self, o: Area) -> Option<Area> {
        let (l, t) = (self.x.max(o.x), self.y.max(o.y));
        let (r, b) = ((self.x + self.w).min(o.x + o.w), (self.y + self.h).min(o.y + o.h));
        (r > l && b > t).then(|| Area::new(l, t, r - l, b - t))
    }
}

/// The strip the previews show: `w` × `h` pixels at the top centre of a monitor `monitor`
/// pixels in size, never wider or taller than the monitor.
pub fn strip(monitor: (i32, i32), w: i32, h: i32) -> Area {
    let (w, h) = (w.min(monitor.0).max(1), h.min(monitor.1).max(1));
    Area::new((monitor.0 - w) / 2, 0, w, h)
}

/// The image `image` (pixels) as Windows draws it: each copy's rectangle on the monitor,
/// relative to the monitor's top-left (scaled copies have the scaled size). `monitor` and
/// `desktop` (the virtual desktop, for Span) are in desktop coordinates. Only copies that
/// touch `within` (monitor-relative) are returned, so tiling a small image stays cheap.
pub fn frames(fit: Fit, image: (i32, i32), monitor: Area, desktop: Area, within: Area) -> Vec<Area> {
    let (iw, ih) = (image.0.max(1), image.1.max(1));
    let (mw, mh) = (monitor.w, monitor.h);
    let scaled = |canvas_w: i32, canvas_h: i32, cover: bool| {
        let (sx, sy) = (canvas_w as f64 / iw as f64, canvas_h as f64 / ih as f64);
        let k = if cover { sx.max(sy) } else { sx.min(sy) };
        let (w, h) = ((iw as f64 * k).round() as i32, (ih as f64 * k).round() as i32);
        Area::new((canvas_w - w) / 2, (canvas_h - h) / 2, w, h)
    };
    let frames = match fit {
        Fit::Center => vec![Area::new((mw - iw) / 2, (mh - ih) / 2, iw, ih)],
        Fit::Stretch => vec![Area::new(0, 0, mw, mh)],
        Fit::Fit => vec![scaled(mw, mh, false)],
        Fit::Fill => vec![scaled(mw, mh, true)],
        Fit::Span => {
            let on_desktop = scaled(desktop.w, desktop.h, true);
            vec![Area::new(on_desktop.x + desktop.x - monitor.x, on_desktop.y + desktop.y - monitor.y, on_desktop.w, on_desktop.h)]
        }
        Fit::Tile => {
            let mut tiles = Vec::new();
            let (first_col, first_row) = (within.x.max(0) / iw, within.y.max(0) / ih);
            let (last_col, last_row) = ((within.x + within.w - 1).max(0) / iw, (within.y + within.h - 1).max(0) / ih);
            for row in first_row..=last_row {
                for col in first_col..=last_col {
                    tiles.push(Area::new(col * iw, row * ih, iw, ih));
                }
            }
            tiles
        }
    };
    frames.into_iter().filter(|f| f.intersect(within).is_some()).collect()
}

/// One copy into the strip: `src` within the frame (its scaled pixels, origin at the
/// frame's top-left), to (`x`, `y`) in the strip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Copy {
    pub frame: Area,
    pub src: Area,
    pub x: i32,
    pub y: i32,
}

/// What of each frame lands in `strip` (all monitor-relative), and where.
pub fn copies(frames: &[Area], strip: Area) -> Vec<Copy> {
    frames
        .iter()
        .filter_map(|&frame| {
            let hit = frame.intersect(strip)?;
            Some(Copy { frame, src: Area::new(hit.x - frame.x, hit.y - frame.y, hit.w, hit.h), x: hit.x - strip.x, y: hit.y - strip.y })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONITOR: Area = Area::new(0, 0, 2560, 1440);

    fn one(fit: Fit, image: (i32, i32)) -> Area {
        let all = frames(fit, image, MONITOR, MONITOR, Area::new(0, 0, 2560, 1440));
        assert_eq!(all.len(), 1, "{fit:?}");
        all[0]
    }

    #[test]
    fn the_strip_is_the_top_centre_and_never_wider_than_the_monitor() {
        assert_eq!(strip((2560, 1440), 1080, 540), Area::new(740, 0, 1080, 540));
        assert_eq!(strip((800, 600), 1080, 900), Area::new(0, 0, 800, 600));
    }

    #[test]
    fn fill_covers_and_crops_the_overflow_evenly() {
        // A 4:3 image on 16:9: as wide as the monitor, cropped top and bottom.
        assert_eq!(one(Fit::Fill, (1600, 1200)), Area::new(0, -240, 2560, 1920));
        // A wider image: as tall as the monitor, cropped at the sides.
        assert_eq!(one(Fit::Fill, (3840, 1080)), Area::new(-1280, 0, 5120, 1440));
    }

    #[test]
    fn fit_letterboxes_and_stretch_ignores_the_aspect() {
        assert_eq!(one(Fit::Fit, (1600, 1200)), Area::new(320, 0, 1920, 1440));
        assert_eq!(one(Fit::Fit, (3840, 1080)), Area::new(0, 360, 2560, 720));
        assert_eq!(one(Fit::Stretch, (1600, 1200)), MONITOR);
    }

    #[test]
    fn centre_keeps_the_native_size_even_when_larger() {
        assert_eq!(one(Fit::Center, (1000, 500)), Area::new(780, 470, 1000, 500));
        assert_eq!(one(Fit::Center, (3000, 2000)), Area::new(-220, -280, 3000, 2000));
    }

    #[test]
    fn tiles_start_at_the_monitors_corner_and_only_where_asked() {
        let top = Area::new(740, 0, 1080, 540);
        let tiles = frames(Fit::Tile, (500, 400), MONITOR, MONITOR, top);
        // Columns 1..=3 (500..2000) and rows 0..=1 (0..800) touch the strip.
        assert_eq!(tiles.len(), 6);
        assert!(tiles.contains(&Area::new(500, 0, 500, 400)) && tiles.contains(&Area::new(1500, 400, 500, 400)));
    }

    #[test]
    fn span_fills_the_desktop_and_shows_this_monitors_part() {
        // Two 1920×1080 monitors side by side; the primary is the right one.
        let desktop = Area::new(-1920, 0, 3840, 1080);
        let primary = Area::new(0, 0, 1920, 1080);
        let all = frames(Fit::Span, (3840, 1080), primary, desktop, Area::new(0, 0, 1920, 1080));
        assert_eq!(all, vec![Area::new(-1920, 0, 3840, 1080)]);
    }

    #[test]
    fn copies_take_the_part_inside_the_strip() {
        let strip = Area::new(740, 0, 1080, 540);
        let fill = one(Fit::Fill, (1600, 1200));
        assert_eq!(copies(&[fill], strip), vec![Copy { frame: fill, src: Area::new(740, 240, 1080, 540), x: 0, y: 0 }]);
        // Letterboxed: the image starts below the strip's top, so the bar shows above it.
        let fit = one(Fit::Fit, (3840, 1080));
        assert_eq!(copies(&[fit], strip), vec![Copy { frame: fit, src: Area::new(740, 0, 1080, 180), x: 0, y: 360 }]);
        // A centred image narrower than the strip lands in its middle.
        let small = one(Fit::Center, (400, 1600));
        assert_eq!(copies(&[small], strip), vec![Copy { frame: small, src: Area::new(0, 80, 400, 540), x: 340, y: 0 }]);
        assert!(copies(&[Area::new(0, 600, 10, 10)], strip).is_empty());
    }

    #[test]
    fn modes_read_from_both_sources() {
        assert_eq!(Fit::from_position(4), Fit::Fill);
        assert_eq!(Fit::from_position(3), Fit::Fit);
        assert_eq!(Fit::from_position(99), Fit::Fill);
        assert_eq!(Fit::from_registry("0", "1"), Fit::Tile);
        assert_eq!(Fit::from_registry("0", "0"), Fit::Center);
        assert_eq!(Fit::from_registry("10", "0"), Fit::Fill);
        assert_eq!(Fit::from_registry("22", ""), Fit::Span);
    }
}
