//! Where the page's pieces go, in DIPs, from the tokens. Pure: text is measured through
//! [`Measure`], so the maths is tested without DirectWrite. Page coordinates: x from the
//! page column's left edge, y from the window's top with the page unscrolled. The window
//! around the page (sidebar, toolbar, scrollbar) is in [`super::chrome`].

use reui_design::tokens::prefs as s;

use super::{Block, Part, Row, Target};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Font {
    Toolbar,
    Hero,
    HeroBody,
    Label,
    RowTitle,
    RowDetail,
    Footnote,
    Nav,
    Control,
    Field,
    Mono,
    Figure,
    Account,
    AccountDetail,
    /// The island's own type, for the previews (drawn scaled).
    Island(IslandFont),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IslandFont {
    Eyebrow,
    Headline,
    Detail,
    Compact,
    Caption,
    Button,
    Code,
    Tab,
    CardText,
    CardLabel,
    Value,
    Context,
    CardTitle,
    CardCode,
    GaugeLabel,
    FactLabel,
    Fact,
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

    /// `w` × `h` at (`l`, `t`).
    pub fn at(l: f32, t: f32, w: f32, h: f32) -> Self {
        Self { l, t, r: l + w, b: t + h }
    }

    pub fn w(&self) -> f32 {
        self.r - self.l
    }

    pub fn h(&self) -> f32 {
        self.b - self.t
    }

    pub fn mid_y(&self) -> f32 {
        (self.t + self.b) / 2.0
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
    /// The header card: the island stage across its top (feature pages), the icon tile, the
    /// eyebrow, title and paragraph beside it, and the on/off row's quiet line.
    Header { block: usize, rect: Rect, stage: Option<Rect>, icon: Rect, eyebrow: Option<Rect>, title: Rect, body: Option<Rect>, note: Option<Rect> },
    /// A group's container.
    Container { block: usize, rect: Rect },
    /// A section header above a container.
    Label { block: usize, rect: Rect },
    /// A footnote under a container.
    Footer { block: usize, rect: Rect },
    /// A row inside its container: where its text and leading icon go, and whether a
    /// hairline separates it from the row above.
    Row { block: usize, row: usize, rect: Rect, text: Rect, icon: Option<Rect>, separator: bool },
    /// A text container: its title line and wrapped body.
    Text { block: usize, rect: Rect, title: Rect, body: Rect },
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

    /// The control at (`x`, `y`); small controls take clicks a little outside their edge.
    pub fn hit(&self, x: f32, y: f32) -> Option<&Placed> {
        let live = || self.parts.iter().filter(|p| p.interactive && p.enabled);
        live().find(|p| p.rect.contains(x, y)).or_else(|| live().find(|p| p.rect.inflate(s::control::HIT_PAD).contains(x, y)))
    }

    /// Controls in reading order, the ones the keyboard visits.
    pub fn focus_order(&self) -> Vec<Target> {
        self.parts.iter().filter(|p| p.interactive && p.enabled).map(|p| p.target.clone()).collect()
    }

    /// The row a control is in (scrolling to a control shows its whole row).
    pub fn row_of(&self, target: &Target) -> Option<Rect> {
        let placed = self.part(target)?;
        self.shapes.iter().find_map(|s| match s {
            Shape::Row { rect, .. } | Shape::Header { rect, .. } if rect.t <= placed.rect.t && rect.b >= placed.rect.b => Some(*rect),
            _ => None,
        })
    }
}

pub fn layout(blocks: &[Block], width: f32, m: &impl Measure) -> Layout {
    let mut out = Layout::default();
    let mut y = s::page::TOP;
    for (i, block) in blocks.iter().enumerate() {
        if i > 0 {
            y += s::page::GROUP_GAP;
        }
        y = match block {
            Block::Intro { .. } => header(&mut out, i, block, y, width, m),
            Block::Group { label, rows, footer } => {
                y = label_above(&mut out, i, label.is_some(), y, width);
                let bare = rows.iter().all(|r| r.bare);
                let top = y;
                for (j, row) in rows.iter().enumerate() {
                    y = place_row(&mut out, i, j, row, y, width, m);
                }
                if !bare {
                    // The container goes under its rows.
                    let at = out.shapes.len() - rows.len();
                    out.shapes.insert(at, Shape::Container { block: i, rect: Rect::new(0.0, top, width, y) });
                }
                match footer {
                    Some(text) => {
                        y += s::page::FOOTER_GAP;
                        let inset = s::page::LABEL_INSET;
                        let h = m.height(text, Font::Footnote, width - 2.0 * inset);
                        out.shapes.push(Shape::Footer { block: i, rect: Rect::new(inset, y, width - inset, y + h) });
                        y + h
                    }
                    None => y,
                }
            }
            Block::Text { label, body, .. } => {
                y = label_above(&mut out, i, label.is_some(), y, width);
                let pad = s::group::PADDING;
                let inner = (width - 2.0 * pad).max(1.0);
                let title_h = s::typography::ROW_TITLE.line_height;
                let body_h = m.height(body, Font::RowDetail, inner);
                let h = 2.0 * pad + title_h + s::group::LINE_GAP + 2.0 + body_h;
                let t = y + pad;
                let title = Rect::new(pad, t, width - pad, t + title_h);
                let bt = title.b + s::group::LINE_GAP + 2.0;
                let body = Rect::new(pad, bt, width - pad, bt + body_h);
                out.shapes.push(Shape::Text { block: i, rect: Rect::new(0.0, y, width, y + h), title, body });
                y + h
            }
            Block::Figures { label, .. } => {
                y = label_above(&mut out, i, label.is_some(), y, width);
                let h = s::typography::FIGURE.line_height + s::group::LINE_GAP + s::typography::LABEL.line_height + 2.0 * s::group::PADDING;
                out.shapes.push(Shape::Figures { block: i, rect: Rect::new(0.0, y, width, y + h) });
                y + h
            }
        };
    }
    out.height = y + s::page::BOTTOM;
    out
}

/// The section header, if there is one; returns where the container starts.
fn label_above(out: &mut Layout, block: usize, label: bool, y: f32, width: f32) -> f32 {
    if !label {
        return y;
    }
    let h = s::typography::LABEL.line_height;
    out.shapes.push(Shape::Label { block, rect: Rect::new(s::page::LABEL_INSET, y, width, y + h) });
    y + h + s::page::LABEL_GAP
}

fn header(out: &mut Layout, i: usize, block: &Block, y: f32, width: f32, m: &impl Measure) -> f32 {
    let Block::Intro { eyebrow, body, preview, row, .. } = block else { return y };
    let pad = s::header::PADDING;
    let stage = preview.map(|kind| Rect::new(0.0, y, width, y + super::stage::height(kind, width)));
    let top = stage.map_or(y, |r| r.b) + pad;
    let icon = Rect::at(pad, top, s::header::ICON, s::header::ICON);
    // The on/off row's controls sit at the right, level with the icon.
    let mut right = width - pad;
    let mut placed = Vec::new();
    if let Some(row) = row {
        for (k, part) in row.parts.iter().enumerate().rev() {
            let (w, h, radius, _) = part_size(part, m);
            let r = Rect::new(right - w, icon.mid_y() - h / 2.0, right, icon.mid_y() + h / 2.0);
            placed.push(Placed { target: Target::new(&row.id, k), rect: r, radius, cells: Vec::new(), enabled: row.enabled, interactive: part.interactive() });
            right = r.l - s::control::GAP;
        }
        right += s::control::GAP - s::group::CONTROL_GAP;
    }
    let left = icon.r + s::header::ICON_GAP;
    let text_w = (right - left).max(1.0);
    let gap = s::header::LINE_GAP;
    let eyebrow_h = eyebrow.as_ref().map_or(0.0, |_| s::typography::LABEL.line_height + gap);
    let title_h = s::typography::HERO.line_height;
    let body_h = body.as_ref().map_or(0.0, |text| m.height(text, Font::HeroBody, text_w) + gap);
    let note_h = row.as_ref().and_then(|r| r.detail.as_ref()).map_or(0.0, |_| s::typography::ROW_DETAIL.line_height + gap);
    let text_h = eyebrow_h + title_h + body_h + note_h;
    // Centred on the icon when it fits beside it; otherwise from the icon's top.
    let mut t = if text_h < icon.h() { icon.mid_y() - text_h / 2.0 } else { icon.t };
    let eyebrow_rect = eyebrow.as_ref().map(|_| {
        let r = Rect::at(left, t, text_w, s::typography::LABEL.line_height);
        t = r.b + gap;
        r
    });
    let title = Rect::at(left, t, text_w, title_h);
    t = title.b;
    let body_rect = body.as_ref().map(|_| {
        let r = Rect::at(left, t + gap, text_w, body_h - gap);
        t = r.b;
        r
    });
    let note = row.as_ref().and_then(|r| r.detail.as_ref()).map(|_| Rect::at(left, t + gap, text_w, s::typography::ROW_DETAIL.line_height));
    let bottom = icon.b.max(top + text_h) + pad;
    out.shapes.push(Shape::Header { block: i, rect: Rect::new(0.0, y, width, bottom), stage, icon, eyebrow: eyebrow_rect, title, body: body_rect, note });
    placed.reverse();
    out.parts.extend(placed);
    bottom
}

fn place_row(out: &mut Layout, block: usize, index: usize, row: &Row, y: f32, width: f32, m: &impl Measure) -> f32 {
    let h = if row.detail.is_some() { s::group::ROW_DETAIL } else { s::group::ROW };
    let rect = Rect::new(0.0, y, width, y + h);
    let mid = y + h / 2.0;
    let pad = s::group::PADDING;
    let mut right = width - pad;
    // Right to left, so the first part ends up leftmost.
    let mut placed = Vec::new();
    for (k, part) in row.parts.iter().enumerate().rev() {
        let (w, ph, radius, cells) = part_size(part, m);
        let left = right - w;
        let r = Rect::new(left, mid - ph / 2.0, right, mid + ph / 2.0);
        let inset = match part {
            Part::Segmented { .. } => s::control::SEGMENT_INSET,
            Part::Hotkey { .. } => (s::control::HEIGHT - s::control::KEY_HEIGHT) / 2.0,
            _ => 0.0,
        };
        let cells = cells.into_iter().map(|(l, cw)| Rect::new(left + l, r.t + inset, left + l + cw, r.b - inset)).collect();
        placed.push(Placed { target: Target::new(&row.id, k), rect: r, radius, cells, enabled: row.enabled, interactive: part.interactive() });
        right = left - s::control::GAP;
    }
    let text_right = if row.parts.is_empty() { width - pad } else { right + s::control::GAP - s::group::CONTROL_GAP };
    let icon = row.icon.map(|_| Rect::at(pad, mid - s::group::ICON / 2.0, s::group::ICON, s::group::ICON));
    let text_left = icon.map_or(pad, |r| r.r + s::group::ICON_GAP);
    let text = Rect::new(text_left, y, text_right.max(text_left), y + h);
    out.shapes.push(Shape::Row { block, row: index, rect, text, icon, separator: index > 0 && !row.bare });
    placed.reverse();
    out.parts.extend(placed);
    y + h
}

/// (width, height, corner radius, cells as (left offset, width)).
pub fn part_size(part: &Part, m: &impl Measure) -> (f32, f32, f32, Vec<(f32, f32)>) {
    let (h, radius, pad) = (s::control::HEIGHT, s::control::RADIUS, s::control::PADDING);
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
            (x + inset, h, radius, cells)
        }
        Part::Slider { .. } => (s::control::SLIDER_READING + s::control::GAP + s::control::SLIDER_WIDTH, h, h / 2.0, Vec::new()),
        Part::Button { label, .. } => ((m.width(label, Font::Control) + 2.0 * pad).max(64.0), h, radius, Vec::new()),
        Part::Hotkey { caps, recording } => {
            let inset = (h - s::control::KEY_HEIGHT) / 2.0;
            if *recording || caps.is_empty() {
                return ((m.width(RECORDING, Font::Control) + 2.0 * pad).max(120.0), h, radius, Vec::new());
            }
            let mut x = inset;
            let mut cells = Vec::new();
            for (i, cap) in caps.iter().enumerate() {
                if i > 0 {
                    x += s::control::KEY_GAP;
                }
                let w = (m.width(cap, Font::Control) + 2.0 * s::control::KEY_PADDING).max(s::control::KEY_HEIGHT);
                cells.push((x, w));
                x += w;
            }
            (x + inset, h, radius, cells)
        }
        Part::Dropdown { label } => (m.width(label, Font::Control) + 2.0 * pad + s::control::GAP + s::control::CHEVRONS, h, radius, Vec::new()),
        Part::Field { .. } => (s::control::FIELD_WIDTH, h, radius, Vec::new()),
        Part::Remove => (s::control::REMOVE, s::control::REMOVE, s::control::REMOVE / 2.0, Vec::new()),
        Part::Value(text) => (m.width(text, Font::RowTitle), h, 0.0, Vec::new()),
    }
}

/// What the shortcut recorder says while it listens.
pub const RECORDING: &str = "Press keys";

/// A slider's value for a pointer at `x` over its bar (`rect` is the whole part).
pub fn slider_value(rect: Rect, x: f32) -> f32 {
    let (l, r) = slider_bar(rect);
    ((x - l) / (r - l)).clamp(0.0, 1.0)
}

/// The slider's bar, left and right: inset by half a knob so the knob stays inside.
pub fn slider_bar(rect: Rect) -> (f32, f32) {
    let knob = s::control::SLIDER_KNOB / 2.0;
    (rect.l + s::control::SLIDER_READING + s::control::GAP + knob, rect.r - knob)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{button, footed, group, Icon, Role};

    /// Every character 7 DIPs wide; text wraps at whole lines of 20.
    pub struct Fixed;

    impl Measure for Fixed {
        fn width(&self, text: &str, _: Font) -> f32 {
            text.chars().count() as f32 * 7.0
        }
        fn height(&self, text: &str, _: Font, width: f32) -> f32 {
            ((self.width(text, Font::HeroBody) / width).ceil().max(1.0)) * 20.0
        }
    }

    fn page() -> Vec<Block> {
        vec![
            Block::Intro { eyebrow: None, title: "Clipboard".into(), body: Some("x".repeat(200)), icon: None, preview: Some(crate::ui::Preview { moment: crate::ui::Moment::Clipboard, look: reui_design::look::Look::default() }), row: Some(Row::new("clip:enabled", "Clipboard").part(Part::Toggle { on: true })) },
            group("History", vec![
                Row::new("a", "One").detail("quiet").part(Part::Toggle { on: true }),
                Row::new("b", "Two").icon(Icon::Mark).part(button("Open", Role::Secondary)).part(Part::Remove),
                Row::new("c", "Three").part(Part::Value("12 MB".into())).enabled(false).part(button("Go", Role::Primary)),
            ]),
            footed("", vec![Row::new("d", "Four").part(Part::Segmented { options: vec!["A".into(), "BB".into()], selected: 0 })], "A footnote"),
        ]
    }

    fn rows(l: &Layout) -> Vec<(Rect, Rect, Option<Rect>, bool)> {
        l.shapes.iter().filter_map(|s| if let Shape::Row { rect, text, icon, separator, .. } = s { Some((*rect, *text, *icon, *separator)) } else { None }).collect()
    }

    #[test]
    fn the_header_card_holds_the_stage_icon_text_and_toggle() {
        let l = layout(&page(), 600.0, &Fixed);
        let Shape::Header { rect, stage: Some(stage), icon, title, body: Some(body), .. } = &l.shapes[0] else { panic!("header first") };
        assert_eq!(rect.t, s::page::TOP);
        assert_eq!(stage.h(), crate::ui::stage::height(crate::ui::Preview { moment: crate::ui::Moment::Clipboard, look: reui_design::look::Look::default() }, 600.0));
        assert_eq!(icon.t, stage.b + s::header::PADDING);
        assert_eq!(title.l, icon.r + s::header::ICON_GAP);
        assert_eq!(body.t, title.b + s::header::LINE_GAP);
        let toggle = l.part(&Target::new("clip:enabled", 0)).unwrap();
        assert_eq!(toggle.rect.r, 600.0 - s::header::PADDING);
        assert_eq!(toggle.rect.mid_y(), icon.mid_y());
        assert!(title.r <= toggle.rect.l - s::group::CONTROL_GAP);
        assert_eq!(rect.b, body.b.max(icon.b) + s::header::PADDING);
    }

    #[test]
    fn rows_share_one_container_with_hairlines_between_them() {
        let l = layout(&page(), 600.0, &Fixed);
        let rows = rows(&l);
        assert_eq!(rows.len(), 4);
        let containers: Vec<Rect> = l.shapes.iter().filter_map(|s| if let Shape::Container { rect, .. } = s { Some(*rect) } else { None }).collect();
        assert_eq!(containers.len(), 2);
        assert_eq!((containers[0].t, containers[0].b), (rows[0].0.t, rows[2].0.b));
        assert_eq!(rows[0].0.h(), s::group::ROW_DETAIL);
        assert_eq!(rows[1].0.t, rows[0].0.b, "no gaps: hairlines separate rows");
        assert_eq!(rows[1].0.h(), s::group::ROW);
        assert_eq!(rows.iter().map(|r| r.3).collect::<Vec<_>>(), [false, true, true, false]);
        // A leading icon pushes the text in.
        let icon = rows[1].2.unwrap();
        assert_eq!(icon.l, s::group::PADDING);
        assert_eq!(rows[1].1.l, icon.r + s::group::ICON_GAP);
        assert_eq!(rows[0].1.l, s::group::PADDING);
        let footer = l.shapes.iter().find_map(|s| if let Shape::Footer { rect, .. } = s { Some(*rect) } else { None }).unwrap();
        assert_eq!(footer.t, containers[1].b + s::page::FOOTER_GAP);
        assert_eq!(l.height, footer.b + s::page::BOTTOM);
    }

    #[test]
    fn controls_sit_right_aligned_and_centred_in_their_row() {
        let l = layout(&page(), 600.0, &Fixed);
        let toggle = l.part(&Target::new("a", 0)).unwrap();
        assert_eq!(toggle.rect.r, 600.0 - s::group::PADDING);
        assert_eq!(toggle.rect.w(), s::control::TOGGLE_WIDTH);
        let open = l.part(&Target::new("b", 0)).unwrap();
        let remove = l.part(&Target::new("b", 1)).unwrap();
        assert_eq!(remove.rect.r, 600.0 - s::group::PADDING);
        assert_eq!(open.rect.r, remove.rect.l - s::control::GAP);
        assert_eq!(open.rect.mid_y(), remove.rect.mid_y());
        let seg = l.part(&Target::new("d", 0)).unwrap();
        assert_eq!(seg.cells.len(), 2);
        assert_eq!(seg.cells[0].l, seg.rect.l + s::control::SEGMENT_INSET);
        assert_eq!(seg.cells[1].r, seg.rect.r - s::control::SEGMENT_INSET);
        let text = rows(&l)[1].1;
        assert_eq!(text.r, open.rect.l - s::group::CONTROL_GAP);
    }

    #[test]
    fn the_keyboard_visits_enabled_controls_in_reading_order() {
        let l = layout(&page(), 600.0, &Fixed);
        let order: Vec<(String, usize)> = l.focus_order().into_iter().map(|t| (t.row, t.part)).collect();
        assert_eq!(order, [("clip:enabled".into(), 0), ("a".into(), 0), ("b".into(), 0), ("b".into(), 1), ("d".into(), 0)]);
        let toggle = l.part(&Target::new("a", 0)).unwrap().rect;
        assert_eq!(l.hit(toggle.l + 1.0, toggle.t + 1.0).map(|p| p.target.clone()), Some(Target::new("a", 0)));
        // Small controls take clicks just outside their edge.
        assert_eq!(l.hit(toggle.r + 2.0, toggle.t - 2.0).map(|p| p.target.clone()), Some(Target::new("a", 0)));
        let disabled = l.part(&Target::new("c", 1)).unwrap().rect;
        assert!(l.hit(disabled.l + 1.0, disabled.t + 1.0).is_none());
        assert_eq!(l.row_of(&Target::new("a", 0)), Some(rows(&l)[0].0));
    }

    #[test]
    fn bare_rows_have_no_container_and_sliders_keep_their_knob_inside() {
        let blocks = vec![group("", vec![Row::new("next", "").part(button("Next", Role::Primary)).bare()])];
        let l = layout(&blocks, 600.0, &Fixed);
        assert!(!l.shapes.iter().any(|s| matches!(s, Shape::Container { .. })));
        let slider = Rect::new(100.0, 0.0, 100.0 + s::control::SLIDER_READING + s::control::GAP + 200.0, 24.0);
        let (bar_l, bar_r) = slider_bar(slider);
        assert_eq!(slider_value(slider, 0.0), 0.0);
        assert_eq!(slider_value(slider, (bar_l + bar_r) / 2.0), 0.5);
        assert_eq!(bar_r, slider.r - s::control::SLIDER_KNOB / 2.0);
    }
}
