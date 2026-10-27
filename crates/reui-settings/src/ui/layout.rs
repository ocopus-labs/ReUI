//! Where everything goes, in DIPs, from the tokens. Pure: text is measured through
//! [`Measure`], so the maths is tested without DirectWrite. Page coordinates: x from the
//! page column's left edge, y from the window's top with the page unscrolled.

use reui_design::tokens::{component, settings as s};

use super::{Block, Part, Row, Target};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Font {
    Title,
    Intro,
    Label,
    RowTitle,
    RowDetail,
    Nav,
    Control,
    Field,
    Mono,
    Figure,
}

pub trait Measure {
    /// One line's width.
    fn width(&self, text: &str, font: Font) -> f32;
    /// Height of `text` wrapped at `width`.
    fn height(&self, text: &str, font: Font, width: f32) -> f32;
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub l: f32,
    pub t: f32,
    pub r: f32,
    pub b: f32,
}

impl Rect {
    pub fn new(l: f32, t: f32, r: f32, b: f32) -> Self {
        Self { l, t, r, b }
    }

    pub fn w(&self) -> f32 {
        self.r - self.l
    }

    pub fn h(&self) -> f32 {
        self.b - self.t
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.l && x < self.r && y >= self.t && y < self.b
    }

    pub fn offset(&self, dx: f32, dy: f32) -> Rect {
        Rect::new(self.l + dx, self.t + dy, self.r + dx, self.b + dy)
    }

    pub fn inflate(&self, by: f32) -> Rect {
        Rect::new(self.l - by, self.t - by, self.r + by, self.b + by)
    }
}

/// A placed block piece, for drawing.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Intro { block: usize, title: Rect, body: Option<Rect> },
    Label { block: usize, rect: Rect },
    /// A row's well: corner radii at the top and bottom (outer or inner), and where its text
    /// may go.
    Row { block: usize, row: usize, rect: Rect, top: f32, bottom: f32, text: Rect },
    Figures { block: usize, rect: Rect },
}

/// A placed control.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub target: Target,
    pub rect: Rect,
    pub radius: f32,
    /// Segmented: each segment. Hotkey: each key cap.
    pub cells: Vec<Rect>,
    pub enabled: bool,
    pub interactive: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    pub shapes: Vec<Shape>,
    pub parts: Vec<Placed>,
    /// Bottom of the content, padding included.
    pub height: f32,
}

impl Layout {
    pub fn part(&self, target: &Target) -> Option<&Placed> {
        self.parts.iter().find(|p| &p.target == target)
    }

    pub fn hit(&self, x: f32, y: f32) -> Option<&Placed> {
        self.parts.iter().find(|p| p.interactive && p.enabled && p.rect.contains(x, y))
    }

    /// Controls in reading order, the ones the keyboard visits.
    pub fn focus_order(&self) -> Vec<Target> {
        self.parts.iter().filter(|p| p.interactive && p.enabled).map(|p| p.target.clone()).collect()
    }
}

/// Width of the reading left of a slider's bar ("10 %").
pub const SLIDER_READING: f32 = 44.0;

pub fn layout(blocks: &[Block], width: f32, m: &impl Measure) -> Layout {
    let mut out = Layout::default();
    let mut y = s::page::TOP;
    for (i, block) in blocks.iter().enumerate() {
        if i > 0 {
            y += if matches!(blocks[i - 1], Block::Intro { .. }) { s::page::INTRO_AFTER } else { s::page::GROUP_GAP };
        }
        match block {
            Block::Intro { title, body } => {
                let title_h = s::typography::TITLE.line_height;
                let title_rect = Rect::new(0.0, y, width, y + title_h);
                let _ = title;
                y += title_h;
                let body = body.as_ref().map(|text| {
                    y += s::page::INTRO_GAP;
                    let h = m.height(text, Font::Intro, width);
                    let rect = Rect::new(0.0, y, width, y + h);
                    y += h;
                    rect
                });
                out.shapes.push(Shape::Intro { block: i, title: title_rect, body });
            }
            Block::Group { label, rows } => {
                if label.is_some() {
                    let h = s::typography::LABEL.line_height;
                    out.shapes.push(Shape::Label { block: i, rect: Rect::new(s::row::PADDING, y, width, y + h) });
                    y += h + s::page::LABEL_GAP;
                }
                for (j, row) in rows.iter().enumerate() {
                    if j > 0 {
                        y += s::row::GAP;
                    }
                    y = place_row(&mut out, i, j, row, rows.len(), y, width, m);
                }
            }
            Block::Figures { label, .. } => {
                if label.is_some() {
                    let h = s::typography::LABEL.line_height;
                    out.shapes.push(Shape::Label { block: i, rect: Rect::new(s::row::PADDING, y, width, y + h) });
                    y += h + s::page::LABEL_GAP;
                }
                let h = s::typography::FIGURE.line_height + s::row::LINE_GAP + s::typography::LABEL.line_height + 2.0 * s::row::PADDING;
                out.shapes.push(Shape::Figures { block: i, rect: Rect::new(0.0, y, width, y + h) });
                y += h;
            }
        }
    }
    out.height = y + s::page::BOTTOM;
    out
}

#[allow(clippy::too_many_arguments)]
fn place_row(out: &mut Layout, block: usize, index: usize, row: &Row, count: usize, y: f32, width: f32, m: &impl Measure) -> f32 {
    let h = if row.detail.is_some() { s::row::HEIGHT } else { s::row::HEIGHT_PLAIN };
    let rect = Rect::new(0.0, y, width, y + h);
    let outer = s::row::RADIUS;
    let inner = s::row::INNER_RADIUS;
    let top = if index == 0 { outer } else { inner };
    let bottom = if index + 1 == count { outer } else { inner };
    let mid = y + h / 2.0;
    let mut right = width - s::row::PADDING;
    // Right to left, so the first part ends up leftmost.
    let mut placed = Vec::new();
    for (k, part) in row.parts.iter().enumerate().rev() {
        let (w, ph, radius, cells) = part_size(part, m);
        let left = right - w;
        let r = Rect::new(left, mid - ph / 2.0, right, mid + ph / 2.0);
        let cells = cells.into_iter().map(|(l, cw)| Rect::new(left + l, r.t, left + l + cw, r.b)).collect::<Vec<_>>();
        let cells = match part {
            // Cells sit inside the track, inset.
            Part::Segmented { .. } | Part::Hotkey { .. } => {
                let inset = if matches!(part, Part::Segmented { .. }) { s::control::SEGMENT_INSET } else { (s::control::HEIGHT - s::control::KEY_HEIGHT) / 2.0 };
                cells.into_iter().map(|c| Rect::new(c.l, c.t + inset, c.r, c.b - inset)).collect()
            }
            _ => cells,
        };
        placed.push(Placed { target: Target::new(&row.id, k), rect: r, radius, cells, enabled: row.enabled, interactive: part.interactive() });
        right = left - s::control::GAP;
    }
    let text_right = if row.parts.is_empty() { width - s::row::PADDING } else { right + s::control::GAP - s::row::CONTROL_GAP };
    let text = Rect::new(s::row::PADDING, y, text_right.max(s::row::PADDING), y + h);
    out.shapes.push(Shape::Row { block, row: index, rect, top, bottom, text });
    placed.reverse();
    out.parts.extend(placed);
    y + h
}

/// (width, height, corner radius, cells as (left offset, width)).
fn part_size(part: &Part, m: &impl Measure) -> (f32, f32, f32, Vec<(f32, f32)>) {
    let h = s::control::HEIGHT;
    let pad = s::control::PADDING;
    match part {
        Part::Toggle { .. } => (s::control::TOGGLE_WIDTH, s::control::TOGGLE_HEIGHT, s::control::TOGGLE_HEIGHT / 2.0, Vec::new()),
        Part::Segmented { options, .. } => {
            let inset = s::control::SEGMENT_INSET;
            let mut x = inset;
            let mut cells = Vec::new();
            for option in options {
                let w = m.width(option, Font::Control) + 2.0 * pad;
                cells.push((x, w));
                x += w;
            }
            (x + inset, h, h / 2.0, cells)
        }
        Part::Slider { .. } => (SLIDER_READING + s::control::GAP + s::control::SLIDER_WIDTH, h, h / 2.0, Vec::new()),
        Part::Button { label, .. } => (m.width(label, Font::Control) + 2.0 * pad, h, h / 2.0, Vec::new()),
        Part::Hotkey { caps, recording } => {
            let inset = (h - s::control::KEY_HEIGHT) / 2.0;
            if *recording || caps.is_empty() {
                return ((m.width("Press keys", Font::Control) + 2.0 * pad).max(120.0), h, h / 2.0, Vec::new());
            }
            let mut x = inset;
            let mut cells = Vec::new();
            for (i, cap) in caps.iter().enumerate() {
                if i > 0 {
                    x += s::control::KEY_GAP;
                }
                let w = (m.width(cap, Font::Control) + 2.0 * component::chip::PADDING_X).max(s::control::KEY_HEIGHT);
                cells.push((x, w));
                x += w;
            }
            (x + inset, h, h / 2.0, cells)
        }
        Part::Dropdown { label } => (m.width(label, Font::Control) + 2.0 * pad + s::control::GAP + s::icon::CONTROL, h, h / 2.0, Vec::new()),
        Part::Field { .. } => (200.0, h, h / 2.0, Vec::new()),
        Part::Remove => (s::control::REMOVE, s::control::REMOVE, s::control::REMOVE / 2.0, Vec::new()),
        Part::Value(text) => (m.width(text, Font::RowTitle), h, 0.0, Vec::new()),
    }
}

/// The page column's width in a window `window_w` wide.
pub fn column_width(window_w: f32) -> f32 {
    (window_w - s::nav::WIDTH - 2.0 * s::page::PADDING_X).clamp(0.0, s::page::MAX_WIDTH)
}

/// The page column's left edge in a window `window_w` wide: after the nav, centred in
/// the space right of it once the column is at its widest.
pub fn page_left(window_w: f32) -> f32 {
    let area = window_w - s::nav::WIDTH;
    s::nav::WIDTH + ((area - column_width(window_w)) / 2.0).max(s::page::PADDING_X)
}

/// The nav item capsules, top to bottom, in window coordinates.
pub fn nav_items(count: usize) -> Vec<Rect> {
    (0..count)
        .map(|i| {
            let t = s::nav::TOP + i as f32 * (s::nav::ITEM_HEIGHT + s::nav::ITEM_GAP);
            Rect::new(s::nav::INSET, t, s::nav::WIDTH - s::nav::INSET, t + s::nav::ITEM_HEIGHT)
        })
        .collect()
}

/// Minimise and close, in window coordinates.
pub fn caption_buttons(window_w: f32) -> [Rect; 2] {
    let size = s::caption::BUTTON;
    let top = s::caption::INSET;
    let close_l = window_w - s::caption::INSET - size;
    let min_l = close_l - s::caption::BUTTON_GAP - size;
    [Rect::new(min_l, top, min_l + size, top + size), Rect::new(close_l, top, close_l + size, top + size)]
}

/// How far the page can scroll.
pub fn max_scroll(content_h: f32, window_h: f32) -> f32 {
    (content_h - window_h).max(0.0)
}

/// The scroll that keeps `rect` (page coordinates) in view below the caption, moving as
/// little as possible.
pub fn scroll_to_show(scroll: f32, rect: Rect, window_h: f32, max: f32) -> f32 {
    let margin = s::page::GROUP_GAP;
    let top = s::caption::HEIGHT + margin;
    let bottom = window_h - margin;
    let target = if rect.t - scroll < top {
        rect.t - top
    } else if rect.b - scroll > bottom {
        rect.b - bottom
    } else {
        scroll
    };
    target.clamp(0.0, max)
}

/// The scrollbar's thumb (top, length) in window coordinates; `None` when nothing scrolls.
pub fn scrollbar(scroll: f32, content_h: f32, window_h: f32) -> Option<(f32, f32)> {
    let max = max_scroll(content_h, window_h);
    if max <= 0.0 {
        return None;
    }
    let (track_t, track_h) = track(window_h);
    let len = (track_h * window_h / content_h).max(s::scrollbar::MIN_LENGTH).min(track_h);
    Some((track_t + (track_h - len) * (scroll / max), len))
}

/// The scroll for a thumb dragged to `thumb_top`.
pub fn scroll_for_thumb(thumb_top: f32, content_h: f32, window_h: f32) -> f32 {
    let max = max_scroll(content_h, window_h);
    let Some((_, len)) = scrollbar(0.0, content_h, window_h) else { return 0.0 };
    let (track_t, track_h) = track(window_h);
    let travel = (track_h - len).max(1.0);
    ((thumb_top - track_t) / travel * max).clamp(0.0, max)
}

fn track(window_h: f32) -> (f32, f32) {
    let t = s::caption::HEIGHT;
    (t, (window_h - t - s::scrollbar::MARGIN).max(0.0))
}

/// A slider's value for a pointer at `x` over its bar (`rect` is the whole part).
pub fn slider_value(rect: Rect, x: f32) -> f32 {
    let (l, r) = slider_bar(rect);
    ((x - l) / (r - l)).clamp(0.0, 1.0)
}

/// The slider's bar, left and right.
pub fn slider_bar(rect: Rect) -> (f32, f32) {
    let l = rect.l + SLIDER_READING + s::control::GAP;
    (l, rect.r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{button, group, Role};

    /// Every character 7 DIPs wide; text wraps at whole lines of 20.
    struct Fixed;

    impl Measure for Fixed {
        fn width(&self, text: &str, _: Font) -> f32 {
            text.chars().count() as f32 * 7.0
        }
        fn height(&self, text: &str, _: Font, width: f32) -> f32 {
            ((self.width(text, Font::Intro) / width).ceil().max(1.0)) * 20.0
        }
    }

    fn page() -> Vec<Block> {
        vec![
            Block::Intro { title: "Clipboard".into(), body: Some("x".repeat(200)) },
            group("History", vec![
                Row::new("a", "One").detail("quiet").part(Part::Toggle { on: true }),
                Row::new("b", "Two").part(button("Open", Role::Secondary)).part(Part::Remove),
                Row::new("c", "Three").part(Part::Value("12 MB".into())).enabled(false).part(button("Go", Role::Primary)),
            ]),
            group("", vec![Row::new("d", "Four").part(Part::Segmented { options: vec!["A".into(), "BB".into()], selected: 0 })]),
        ]
    }

    #[test]
    fn rows_stack_in_wells_with_outer_and_inner_corners() {
        let l = layout(&page(), 600.0, &Fixed);
        let rows: Vec<_> = l.shapes.iter().filter_map(|s| if let Shape::Row { rect, top, bottom, .. } = s { Some((*rect, *top, *bottom)) } else { None }).collect();
        assert_eq!(rows.len(), 4);
        // Title 34, gap 8, body: 1400 DIPs of text at 600 wraps to 3 lines of 20.
        let intro_bottom = s::page::TOP + 34.0 + s::page::INTRO_GAP + 60.0;
        let first = intro_bottom + s::page::INTRO_AFTER + s::typography::LABEL.line_height + s::page::LABEL_GAP;
        assert_eq!(rows[0].0.t, first);
        assert_eq!(rows[0].0.h(), s::row::HEIGHT);
        assert_eq!(rows[1].0.t, rows[0].0.b + s::row::GAP);
        assert_eq!(rows[1].0.h(), s::row::HEIGHT_PLAIN);
        assert_eq!((rows[0].1, rows[0].2), (s::row::RADIUS, s::row::INNER_RADIUS));
        assert_eq!((rows[1].1, rows[1].2), (s::row::INNER_RADIUS, s::row::INNER_RADIUS));
        assert_eq!((rows[2].1, rows[2].2), (s::row::INNER_RADIUS, s::row::RADIUS));
        // A group without a label starts right after the gap; alone, it's round all over.
        assert_eq!(rows[3].0.t, rows[2].0.b + s::page::GROUP_GAP);
        assert_eq!((rows[3].1, rows[3].2), (s::row::RADIUS, s::row::RADIUS));
        assert_eq!(l.height, rows[3].0.b + s::page::BOTTOM);
    }

    #[test]
    fn controls_sit_right_aligned_and_centred_in_their_row() {
        let l = layout(&page(), 600.0, &Fixed);
        let toggle = l.part(&Target::new("a", 0)).unwrap();
        assert_eq!(toggle.rect.r, 600.0 - s::row::PADDING);
        assert_eq!(toggle.rect.w(), s::control::TOGGLE_WIDTH);
        let open = l.part(&Target::new("b", 0)).unwrap();
        let remove = l.part(&Target::new("b", 1)).unwrap();
        assert_eq!(remove.rect.r, 600.0 - s::row::PADDING);
        assert_eq!(open.rect.r, remove.rect.l - s::control::GAP);
        assert_eq!(open.rect.w(), 4.0 * 7.0 + 2.0 * s::control::PADDING);
        assert_eq!((open.rect.t + open.rect.b) / 2.0, (remove.rect.t + remove.rect.b) / 2.0);
        let seg = l.part(&Target::new("d", 0)).unwrap();
        assert_eq!(seg.cells.len(), 2);
        assert_eq!(seg.cells[0].l, seg.rect.l + s::control::SEGMENT_INSET);
        assert_eq!(seg.cells[1].r, seg.rect.r - s::control::SEGMENT_INSET);
        // The row's text stops short of its first control.
        let text = l.shapes.iter().find_map(|s| if let Shape::Row { row: 1, text, .. } = s { Some(*text) } else { None }).unwrap();
        assert_eq!(text.r, open.rect.l - s::row::CONTROL_GAP);
    }

    #[test]
    fn the_keyboard_visits_enabled_controls_in_reading_order() {
        let l = layout(&page(), 600.0, &Fixed);
        let order: Vec<(String, usize)> = l.focus_order().into_iter().map(|t| (t.row, t.part)).collect();
        assert_eq!(order, [("a".into(), 0), ("b".into(), 0), ("b".into(), 1), ("d".into(), 0)]);
        let toggle = l.part(&Target::new("a", 0)).unwrap().rect;
        assert_eq!(l.hit(toggle.l + 1.0, toggle.t + 1.0).map(|p| p.target.clone()), Some(Target::new("a", 0)));
        let disabled = l.part(&Target::new("c", 1)).unwrap().rect;
        assert!(l.hit(disabled.l + 1.0, disabled.t + 1.0).is_none());
    }

    #[test]
    fn scrolling_keeps_things_in_view_and_the_thumb_tracks_it() {
        let (content, window) = (2000.0, 600.0);
        let max = max_scroll(content, window);
        assert_eq!(max, 1400.0);
        assert_eq!(max_scroll(300.0, 600.0), 0.0);
        // Already visible: unchanged.
        assert_eq!(scroll_to_show(0.0, Rect::new(0.0, 200.0, 10.0, 260.0), window, max), 0.0);
        // Below the fold: just enough to bring its bottom in.
        assert_eq!(scroll_to_show(0.0, Rect::new(0.0, 900.0, 10.0, 964.0), window, max), 964.0 - (window - s::page::GROUP_GAP));
        // Above, under the caption: its top comes back below it.
        assert_eq!(scroll_to_show(500.0, Rect::new(0.0, 520.0, 10.0, 584.0), window, max), 520.0 - s::caption::HEIGHT - s::page::GROUP_GAP);
        let (top0, len) = scrollbar(0.0, content, window).unwrap();
        let (top1, _) = scrollbar(max, content, window).unwrap();
        assert_eq!(top0, s::caption::HEIGHT);
        assert!((top1 + len - (window - s::scrollbar::MARGIN)).abs() < 1e-3);
        assert!((scroll_for_thumb(top1, content, window) - max).abs() < 1e-3);
        assert_eq!(scroll_for_thumb(top0 - 50.0, content, window), 0.0);
        assert!(scrollbar(0.0, 300.0, window).is_none());
    }

    #[test]
    fn nav_and_caption_geometry() {
        let items = nav_items(3);
        assert_eq!(items[1].t, items[0].b + s::nav::ITEM_GAP);
        let [min, close] = caption_buttons(1000.0);
        assert_eq!(close.r, 1000.0 - s::caption::INSET);
        assert_eq!(min.r, close.l - s::caption::BUTTON_GAP);
        assert_eq!(column_width(10_000.0), s::page::MAX_WIDTH);
        assert_eq!(page_left(900.0), s::nav::WIDTH + s::page::PADDING_X);
        let wide = page_left(2000.0);
        assert_eq!(wide - s::nav::WIDTH, 2000.0 - wide - s::page::MAX_WIDTH);
        let slider = Rect::new(100.0, 0.0, 100.0 + SLIDER_READING + s::control::GAP + 200.0, 32.0);
        assert_eq!(slider_value(slider, 0.0), 0.0);
        assert_eq!(slider_value(slider, slider.r - 100.0), 0.5);
    }
}
