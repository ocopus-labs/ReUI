//! The island's component library and layout, drawn with Direct2D/DirectWrite into
//! composition surfaces. Every size, colour and font comes from `reui_design::tokens`;
//! this file only decides how the protocol's semantic view model maps onto them.
//!
//! Layout rules (see DESIGN.md):
//! - Hierarchy: expanded text reads eyebrow (accent) → hero → detail (secondary).
//! - Concentric corners: shapes follow the island's curve. In the compact capsule, round
//!   content is a circle inset so it sits concentric with the pill's ends; in the expanded
//!   island, corner tiles use `radius.concentric` and buttons are capsules.
//!
//! Layout is in DIPs; `scale` converts to pixels at draw time.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};

use reui_design::tokens::{color, component, font, icon, island, opacity, radius, space, typography};
use reui_design::{Color, TextStyle};
use reui_protocol::{Art, Button, ButtonContent, ButtonRole, CompactView, Element, ExpandedView, Headline, ImageId, Item, LyricLine, Menu, Pager, Preview, Rgb, Row, Tab, Tone};
use windows::core::{w, Interface, Result, HSTRING};
use windows::Foundation::Size;
use windows::Graphics::DirectX::{DirectXAlphaMode, DirectXPixelFormat};
use windows::UI::Composition::{CompositionDrawingSurface, CompositionGraphicsDevice, Compositor};
use windows::Win32::Foundation::{HMODULE, POINT};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::Shell::SHCreateMemStream;
use windows::Win32::System::WinRT::Composition::{ICompositionDrawingSurfaceInterop, ICompositorInterop};
use windows_numerics::{Matrix3x2, Vector2};

use crate::music::{self, SeekHold};
use crate::panel::{self, CardRow};

/// Widest compact presentation: two maximal sides around the centre gap.
pub const COMPACT_MAX_WIDTH: f32 = island::compact::CENTER_GAP + 2.0 * (island::compact::MAX_SIDE + island::compact::TEXT_MARGIN);
pub const COMPACT_SURFACE: (f32, f32) = (COMPACT_MAX_WIDTH, island::HEIGHT);
/// Wide enough for the wide expanded island; normal views are drawn centred in it.
pub const EXPANDED_SURFACE: (f32, f32) = (island::expanded::WIDE_WIDTH, island::expanded::MAX_HEIGHT);
pub const MINIMAL_SURFACE: (f32, f32) = (island::minimal::SIZE, island::minimal::SIZE);

pub const BARS: usize = 4;
/// Resting heights: a frozen meter while live, flat otherwise.
pub const BAR_LEVELS_LIVE: [f32; BARS] = [0.45, 0.9, 0.6, 0.75];
pub const BAR_LEVEL_IDLE: f32 = 0.3;

/// Longest side images are kept at after decoding: enough for a 56 DIP tile at 4x.
const IMAGE_MAX_PX: u32 = 256;

/// Where an element sits; decides its size and shape.
#[derive(Clone, Copy, PartialEq)]
enum Slot {
    /// Compact sides. Level bars are left to the pill, which animates them as visuals.
    Compact,
    Minimal,
    Leading,
    Trailing,
}

/// A pressable region of the expanded view, in expanded-surface DIPs.
pub struct Hit {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
    pub action: String,
    /// A seekable bar: its left and right edges and the timeline length. Clicking at `x`
    /// means "seek to (x - left) / (right - left) of the length".
    pub seek: Option<(f32, f32, u64)>,
    /// A card: its index in the row, for Shift+click and its context menu.
    pub card: Option<usize>,
    /// A slider's bar (left and right edges); `action` is its prefix (`volume:`).
    pub slider: Option<(f32, f32)>,
    /// The slider's trailing menu button: a click opens the view's [`Menu`].
    pub menu: bool,
    /// The source switcher: the wheel over it switches source.
    pub pager: bool,
}

impl Hit {
    fn new(left: f32, top: f32, right: f32, bottom: f32, action: &str) -> Self {
        Hit { left, top, right, bottom, action: action.to_string(), seek: None, card: None, slider: None, menu: false, pager: false }
    }

    /// The action a click at `x` (card DIPs) sends.
    pub fn action_at(&self, x: f32) -> String {
        match self.seek {
            Some((left, right, length)) => format!("seek:{}", music::scrub_ms(x, left, right, length)),
            None => self.action.clone(),
        }
    }
}

pub fn hit_at(hits: &[Hit], x: f32, y: f32) -> Option<&Hit> {
    hits.iter().find(|h| x >= h.left && x < h.right && y >= h.top && y < h.bottom)
}

/// Where the compact presentation put things the pill animates itself.
pub struct CompactLayout {
    pub width: f32,
    /// Left edge and vertical centre of the level bars, in compact-surface DIPs.
    pub bars: Option<(f32, f32, bool)>,
    /// An animated logo: its image and the top-left of its tile, in compact-surface DIPs.
    /// The renderer draws only the tile; the pill draws and spins the mark.
    pub logo: Option<(ImageId, f32, f32)>,
}

/// What the user is doing with an open panel, drawn over the module's view.
#[derive(Default)]
pub struct Interaction<'a> {
    /// The search text as typed (the module's query when `None`).
    pub query: Option<&'a str>,
    /// The panel has the keyboard: a caret in the field and a ring on the selected card.
    pub keyboard: bool,
    pub selected: usize,
    /// How far the card row is scrolled, in DIPs.
    pub scroll: f32,
    /// Timeline position shown while dragging the scrubber.
    pub scrub: Option<u64>,
    /// A seek shown before the player confirmed it (the pill drops it once it has).
    pub hold: Option<SeekHold>,
    /// Slider value shown instead of the module's (dragging, or just set).
    pub slider: Option<f32>,
    /// The bar under the pointer (or being dragged) shows its thumb.
    pub thumb: Option<Thumb>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Thumb {
    Seek,
    Slider,
}

pub struct ExpandedLayout {
    pub width: f32,
    pub height: f32,
    /// Corner radius of the expanded shape (the wide panel's is calmer than the pill's).
    pub radius: f32,
    pub hits: Vec<Hit>,
    /// Left edge, vertical centre and liveness of the level bars, in expanded-surface DIPs.
    pub bars: Option<(f32, f32, bool)>,
}

enum Decoded {
    Bitmap(IWICBitmap),
    /// Kept as text: it's re-tinted per draw by setting the root `fill`.
    Svg(String),
}

struct Font {
    format: IDWriteTextFormat,
    style: TextStyle,
    tabular: bool,
}

struct Fonts {
    eyebrow: Font,
    value: Font,
    headline: Font,
    code: Font,
    detail: Font,
    compact: Font,
    button: Font,
    caption: Font,
    icon_compact: Font,
    icon_button: Font,
    icon_tile: Font,
    icon_tile_compact: Font,
    icon_trailing: Font,
    card_text: Font,
    card_code: Font,
    card_label: Font,
    card_title: Font,
    tab: Font,
    tab_count: Font,
    search: Font,
    note: Font,
    icon_search: Font,
    icon_star: Font,
    icon_kind: Font,
    icon_pager: Font,
    lyric: Font,
    pager: Font,
}

pub struct Renderer {
    graphics: CompositionGraphicsDevice,
    factory: ID2D1Factory1,
    dwrite: IDWriteFactory,
    wic: IWICImagingFactory,
    images: HashMap<ImageId, Decoded>,
    tabular: IDWriteTypography,
    round: ID2D1StrokeStyle,
    fonts: Fonts,
    scale: f32,
    /// Width of the expanded view being drawn (normal or wide).
    width: Cell<f32>,
}

impl Renderer {
    pub fn new(compositor: &Compositor, scale: f32) -> Result<Self> {
        let d3d = create_d3d_device()?;
        let dxgi: IDXGIDevice = d3d.cast()?;
        let factory: ID2D1Factory1 = unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        let device = unsafe { factory.CreateDevice(&dxgi)? };
        let graphics = unsafe { compositor.cast::<ICompositorInterop>()?.CreateGraphicsDevice(&device)? };
        let dwrite: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        let wic: IWICImagingFactory = unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)? };

        let tabular = unsafe {
            let t = dwrite.CreateTypography()?;
            t.AddFontFeature(DWRITE_FONT_FEATURE { nameTag: DWRITE_FONT_FEATURE_TAG_TABULAR_FIGURES, parameter: 1 })?;
            t
        };
        let round = unsafe {
            let base: &ID2D1Factory = &factory;
            base.CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    dashCap: D2D1_CAP_STYLE_ROUND,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    miterLimit: 10.0,
                    dashStyle: D2D1_DASH_STYLE_SOLID,
                    dashOffset: 0.0,
                },
                None,
            )?
        };
        let icons = |size: f32| TextStyle { family: font::ICONS, weight: 400, size, line_height: size };
        let f = |style: TextStyle, tabular: bool| make_font(&dwrite, style, tabular, false, scale);
        let wrapped = |style: TextStyle| make_font(&dwrite, style, false, true, scale);
        let fonts = Fonts {
            eyebrow: f(typography::EYEBROW, false)?,
            value: f(typography::VALUE, true)?,
            headline: f(typography::HEADLINE, false)?,
            code: f(typography::CODE, false)?,
            detail: f(typography::DETAIL, false)?,
            compact: f(typography::COMPACT, true)?,
            button: f(typography::BUTTON, false)?,
            caption: f(typography::CAPTION, true)?,
            icon_compact: f(icons(icon::COMPACT), false)?,
            icon_button: f(icons(icon::BUTTON), false)?,
            icon_tile: f(icons(icon::TILE), false)?,
            icon_tile_compact: f(icons(icon::TILE_COMPACT), false)?,
            icon_trailing: f(icons(icon::TRAILING), false)?,
            card_text: wrapped(typography::CARD_TEXT)?,
            card_code: wrapped(typography::CARD_CODE)?,
            card_label: f(typography::CARD_LABEL, false)?,
            card_title: wrapped(typography::CARD_TITLE)?,
            tab: f(typography::TAB, false)?,
            tab_count: f(typography::TAB, true)?,
            search: f(typography::SEARCH, false)?,
            note: f(typography::NOTE, false)?,
            icon_search: f(icons(icon::SEARCH), false)?,
            icon_star: f(icons(icon::STAR), false)?,
            icon_kind: f(icons(component::card::KIND_ICON), false)?,
            icon_pager: f(icons(icon::PAGER), false)?,
            lyric: f(typography::LYRIC, false)?,
            pager: f(typography::PAGER, true)?,
        };
        Ok(Self { graphics, factory, dwrite, wic, images: HashMap::new(), tabular, round, fonts, scale, width: Cell::new(island::expanded::WIDTH) })
    }

    pub fn surface(&self, dips: (f32, f32)) -> Result<CompositionDrawingSurface> {
        let size = Size { Width: (dips.0 * self.scale).ceil(), Height: (dips.1 * self.scale).ceil() };
        self.graphics.CreateDrawingSurface(size, DirectXPixelFormat::B8G8R8A8UIntNormalized, DirectXAlphaMode::Premultiplied)
    }

    /// Decodes image bytes once: SVG is kept as text, anything else is decoded with WIC and
    /// downscaled so the short side is at most `IMAGE_MAX_PX`.
    pub fn add_image(&mut self, id: ImageId, bytes: &[u8]) -> Result<()> {
        let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]);
        if head.contains("<svg") {
            self.images.insert(id, Decoded::Svg(fit_svg(&String::from_utf8_lossy(bytes))));
            return Ok(());
        }
        unsafe {
            let stream = self.wic.CreateStream()?;
            stream.InitializeFromMemory(bytes)?;
            let decoder = self.wic.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)?;
            let frame = decoder.GetFrame(0)?;
            let (mut w, mut h) = (0u32, 0u32);
            frame.GetSize(&mut w, &mut h)?;
            let k = (IMAGE_MAX_PX as f32 / w.min(h).max(1) as f32).min(1.0);
            let scaler = self.wic.CreateBitmapScaler()?;
            scaler.Initialize(&frame, ((w as f32 * k) as u32).max(1), ((h as f32 * k) as u32).max(1), WICBitmapInterpolationModeFant)?;
            let converter = self.wic.CreateFormatConverter()?;
            converter.Initialize(&scaler, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeMedianCut)?;
            let bitmap = self.wic.CreateBitmapFromSource(&converter, WICBitmapCacheOnLoad)?;
            self.images.insert(id, Decoded::Bitmap(bitmap));
        }
        Ok(())
    }

    /// Drops decoded images the latest snapshot no longer shows; the hub forgets having
    /// sent them by the same rule (`panel::referenced`), so it sends them again if needed.
    pub fn has_image(&self, id: ImageId) -> bool {
        self.images.contains_key(&id)
    }

    pub fn retain_images(&mut self, keep: &HashSet<ImageId>) {
        self.images.retain(|id, _| keep.contains(id));
    }

    /// Draws the compact presentation centred in its surface. Both sides get the same
    /// width (the wider one's, margins included), as in Apple's compact layout.
    pub fn draw_compact(&self, surface: &CompositionDrawingSurface, view: Option<&CompactView>, accent: Color) -> Result<CompactLayout> {
        let mut layout = CompactLayout { width: island::idle::WIDTH, bars: None, logo: None };
        self.paint(surface, |rt| {
            let Some(view) = view else { return Ok(()) };
            let (lw, _) = self.measure(&view.leading, Slot::Compact)?;
            let (tw, _) = self.measure(&view.trailing, Slot::Compact)?;
            let (lm, tm) = (compact_margin(&view.leading), compact_margin(&view.trailing));
            let side = (lw + lm).max(tw + tm);
            let width = island::compact::CENTER_GAP + 2.0 * side;
            let left = (COMPACT_SURFACE.0 - width) / 2.0;
            let mid = COMPACT_SURFACE.1 / 2.0;
            let leading_x = left + lm;
            let trailing_x = left + width - tm;
            self.element(rt, &view.leading, Slot::Compact, leading_x, mid, Align::Start, accent)?;
            self.element(rt, &view.trailing, Slot::Compact, trailing_x, mid, Align::End, accent)?;
            layout.width = width;
            layout.bars = match (&view.leading, &view.trailing) {
                (Element::Bars { live }, _) => Some((leading_x, mid, *live)),
                (_, Element::Bars { live }) => Some((trailing_x - tw, mid, *live)),
                _ => None,
            };
            let tile = component::art::COMPACT;
            layout.logo = match (&view.leading, &view.trailing) {
                (Element::Art(Art::Logo { image, animated: true }), _) => Some((*image, leading_x, mid - tile / 2.0)),
                (_, Element::Art(Art::Logo { image, animated: true })) => Some((*image, trailing_x - tile, mid - tile / 2.0)),
                _ => None,
            };
            Ok(())
        })?;
        Ok(layout)
    }

    /// The mark alone, centred in a square surface, for the pill's animated logo visual.
    pub fn draw_logo(&self, surface: &CompositionDrawingSurface, image: ImageId, size: f32, color: Color) -> Result<()> {
        self.paint(surface, |rt| {
            let mark = size * opacity::LOGO;
            self.svg(rt, image, (size - mark) / 2.0, (size - mark) / 2.0, mark, color)
        })
    }

    /// Draws the minimal bubble's element; returns where live bars go (left, centre, live).
    pub fn draw_minimal(&self, surface: &CompositionDrawingSurface, element: Option<&Element>, accent: Color) -> Result<Option<(f32, f32, bool)>> {
        let (cx, cy) = (MINIMAL_SURFACE.0 / 2.0, MINIMAL_SURFACE.1 / 2.0);
        self.paint(surface, |rt| match element {
            Some(e) => self.element(rt, e, Slot::Minimal, cx, cy, Align::Center, accent),
            None => Ok(()),
        })?;
        Ok(match element {
            Some(Element::Bars { live }) => Some((cx - bars_width() / 2.0, cy, *live)),
            _ => None,
        })
    }

    /// Draws the expanded presentation and returns its layout: height, pressable regions,
    /// and where live bars go (the pill draws those).
    pub fn draw_expanded(&self, surface: &CompositionDrawingSurface, view: Option<&ExpandedView>, accent: Color, with: &Interaction) -> Result<ExpandedLayout> {
        let mut height = island::expanded::MIN_HEIGHT;
        let mut hits = Vec::new();
        let mut bars = None;
        let mut width = island::expanded::WIDTH;
        self.paint(surface, |rt| {
            let Some(view) = view else { return Ok(()) };
            let w = if view.wide { island::expanded::WIDE_WIDTH } else { island::expanded::WIDTH };
            width = w;
            self.width.set(w);
            // Normal views sit centred in the wide surface.
            let dx = (EXPANDED_SURFACE.0 - w) / 2.0;
            unsafe {
                let mut transform = Matrix3x2::default();
                rt.GetTransform(&mut transform);
                rt.SetTransform(&Matrix3x2 { M31: transform.M31 + dx * self.scale, ..transform });
            }
            let inset = island::expanded::INSET;
            let gap = island::expanded::COLUMN_GAP;
            if view.tint {
                self.wash(rt, w, accent)?;
            }

            // A lone row of glyph buttons belongs beside the hero, not under it.
            let inline: Option<&[Button]> = match view.rows.as_slice() {
                [Row::Buttons(b)] if b.iter().all(|b| matches!(b.content, ButtonContent::Glyph(_))) => Some(b),
                _ => None,
            };
            let rows: &[Row] = if inline.is_some() { &[] } else { &view.rows };

            let (lw, lh) = self.measure(&view.leading, Slot::Leading)?;
            let (tw, th) = match inline {
                Some(b) => (inline_width(b.len()), component::button::INLINE),
                None => self.measure(&view.trailing, Slot::Trailing)?,
            };
            let text_left = inset + if lw > 0.0 { lw + gap } else { 0.0 };
            let text_right = w - inset - if tw > 0.0 { tw + gap } else { 0.0 };
            let text_w = text_right - text_left;

            let line_gap = island::expanded::LINE_GAP;
            let eyebrow_h = view.eyebrow.as_ref().map_or(0.0, |_| self.fonts.eyebrow.style.line_height + line_gap);
            let hero_h = match &view.headline {
                Headline::Value(_) => self.fonts.value.style.line_height,
                Headline::Text(_) => self.fonts.headline.style.line_height,
                Headline::Code(_) => self.fonts.code.style.line_height + 2.0 * component::chip::PADDING_Y,
            };
            let detail_h = view.detail.as_ref().map_or(0.0, |_| line_gap + self.fonts.detail.style.line_height);
            let text_h = eyebrow_h + hero_h + detail_h;
            // A view of only rows (e.g. clipboard cards) has no header at all.
            let headless = matches!(view.leading, Element::Empty)
                && matches!(view.trailing, Element::Empty)
                && inline.is_none()
                && view.eyebrow.is_none()
                && view.detail.is_none()
                && view.pager.is_none()
                && matches!(&view.headline, Headline::Text(t) if t.is_empty());
            let header_h = if headless { 0.0 } else { lh.max(th).max(text_h) };
            // The leading tile sits one inset from the top as from the left, and what's at
            // the right shares its centre line. Text is centred on it when it fits beside
            // it, and starts at the inset when it's taller.
            let (mid, text_top) = if lh > 0.0 {
                let mid = inset + lh.max(th) / 2.0;
                (mid, if text_h <= lh { mid - text_h / 2.0 } else { inset })
            } else {
                let mid = inset + header_h / 2.0;
                (mid, mid - text_h / 2.0)
            };

            self.element(rt, &view.leading, Slot::Leading, inset, mid, Align::Start, accent)?;
            if let Some(open) = view.open.as_deref().filter(|_| lw > 0.0) {
                hits.push(Hit::new(inset, mid - lh / 2.0, inset + lw, mid + lh / 2.0, open));
            }
            bars = match (&view.leading, &view.trailing, inline) {
                (Element::Bars { live }, _, _) => Some((inset, mid, *live)),
                (_, Element::Bars { live }, None) => Some((w - inset - tw, mid, *live)),
                _ => None,
            };
            match inline {
                Some(buttons) => {
                    let size = component::button::INLINE;
                    let mut x = w - inset - tw;
                    for button in buttons {
                        self.button(rt, button, x, mid - size / 2.0, size, size, accent, &mut hits)?;
                        x += size + component::button::INLINE_GAP;
                    }
                }
                None => self.element(rt, &view.trailing, Slot::Trailing, w - inset, mid, Align::End, accent)?,
            }

            // The source switcher sits at the top right, on the first line of text, which
            // makes room for it.
            let first_line = if view.eyebrow.is_some() { self.fonts.eyebrow.style.line_height } else { hero_h };
            let first_w = match &view.pager {
                Some(pager) => self.pager(rt, pager, w - inset, text_top + first_line / 2.0, &mut hits)? - space::SM - text_left,
                None => text_w,
            };

            // Text column: eyebrow (accent) → hero (primary) → detail (secondary).
            let mut y = text_top;
            let mut line_w = first_w.min(text_w);
            if let Some(eyebrow) = &view.eyebrow {
                self.text(rt, eyebrow, &self.fonts.eyebrow, text_left, y, line_w, Align::Start, accent)?;
                y += eyebrow_h;
                line_w = text_w;
            }
            match &view.headline {
                Headline::Value(s) => self.text(rt, s, &self.fonts.value, text_left, y, line_w, Align::Start, color::label::PRIMARY)?,
                Headline::Text(s) => self.text(rt, s, &self.fonts.headline, text_left, y, line_w, Align::Start, color::label::PRIMARY)?,
                Headline::Code(s) => {
                    let (px, py) = (component::chip::PADDING_X, component::chip::PADDING_Y);
                    let chip_w = (self.text_width(s, &self.fonts.code)? + 2.0 * px).min(line_w);
                    unsafe { self.round_rect(rt, text_left, y, text_left + chip_w, y + hero_h, radius::CHIP, color::fill::TERTIARY)? };
                    self.text(rt, s, &self.fonts.code, text_left + px, y + py, chip_w - 2.0 * px, Align::Start, color::label::PRIMARY)?;
                }
            }
            y += hero_h;
            if let Some(detail) = &view.detail {
                self.text(rt, detail, &self.fonts.detail, text_left, y + line_gap, text_w, Align::Start, color::label::SECONDARY)?;
            }

            // Where the timeline is now as shown (a drag's preview or a held seek win), for
            // the bar and the lyrics.
            let now = reui_protocol::unix_ms();
            let shown_at = |position_ms: u64, length_ms: u64, playing: bool, at_unix_ms: u64, rate: f32| {
                let reported = || Row::timeline_position(position_ms, length_ms, playing, at_unix_ms, rate, now);
                with.scrub.or_else(|| with.hold.map(|h| h.position(length_ms, playing, rate, now))).unwrap_or_else(reported).min(length_ms)
            };
            let track_at = rows.iter().find_map(|r| match r {
                Row::Timeline { position_ms, length_ms, playing, at_unix_ms, rate, .. } if *length_ms > 0 => Some(shown_at(*position_ms, *length_ms, *playing, *at_unix_ms, *rate)),
                _ => None,
            });

            // Full-width rows. A progress bar with times followed by icon-only controls shares
            // one line under the bar when the controls fit between the times; otherwise the
            // times flank the bar.
            let mut y = inset + header_h;
            let mut i = 0;
            while i < rows.len() {
                y += match i {
                    0 if headless => 0.0,
                    0 => island::expanded::SECTION_GAP,
                    _ => island::expanded::ROW_GAP,
                };
                let meter = match &rows[i] {
                    Row::Progress { fraction, leading, trailing } => Some(Meter { fraction: Some(*fraction), leading: leading.clone(), trailing: trailing.clone(), seek: None }),
                    // Live: no bar, just a quiet fact where the times would be.
                    Row::Timeline { length_ms: 0, .. } => Some(Meter { fraction: None, leading: Some("LIVE".into()), trailing: None, seek: None }),
                    Row::Timeline { position_ms, length_ms, playing, at_unix_ms, seekable, rate } => {
                        let at = shown_at(*position_ms, *length_ms, *playing, *at_unix_ms, *rate);
                        Some(Meter {
                            fraction: Some(at as f32 / *length_ms as f32),
                            leading: Some(clock(at)),
                            trailing: Some(format!("-{}", clock_left(length_ms - at))),
                            seek: seekable.then_some(*length_ms),
                        })
                    }
                    Row::Buttons(_) | Row::Items(_) | Row::Search { .. } | Row::Tabs(_) | Row::Note(_) | Row::Slider { .. } | Row::Lyrics(_) => None,
                };
                let controls = match rows.get(i + 1) {
                    Some(Row::Buttons(b)) if b.iter().all(|b| matches!(b.content, ButtonContent::Glyph(_))) => Some(b),
                    _ => None,
                };
                let thumb = with.thumb == Some(Thumb::Seek);
                match (meter, controls) {
                    (Some(m), Some(buttons)) if m.has_times() && self.fits_between(buttons, &m)? => {
                        if let Some(fraction) = m.fraction {
                            y += self.meter(rt, (inset, w - inset), y, fraction, m.seek, thumb, accent, &mut hits)? + space::SM;
                        }
                        let row_h = self.button_row(rt, buttons, y, accent, &mut hits)?;
                        self.captions(rt, m.leading.as_deref(), m.trailing.as_deref(), y + row_h / 2.0)?;
                        y += row_h;
                        i += 2;
                        continue;
                    }
                    (Some(m @ Meter { fraction: Some(fraction), .. }), Some(_)) if m.has_times() => {
                        // Times at the bar's ends, on its line.
                        let line = self.fonts.caption.style.line_height;
                        let lead = self.caption_width(m.leading.as_deref())?;
                        let trail = self.caption_width(m.trailing.as_deref())?;
                        let span = (inset + lead + space::SM, w - inset - trail - space::SM);
                        self.meter(rt, span, y + (line - component::progress::HEIGHT) / 2.0, fraction, m.seek, thumb, accent, &mut hits)?;
                        self.captions(rt, m.leading.as_deref(), m.trailing.as_deref(), y + line / 2.0)?;
                        y += line;
                    }
                    (Some(m), _) => {
                        if let Some(fraction) = m.fraction {
                            y += self.meter(rt, (inset, w - inset), y, fraction, m.seek, thumb, accent, &mut hits)?;
                            if m.has_times() {
                                y += space::SM;
                            }
                        }
                        if m.has_times() {
                            let line = self.fonts.caption.style.line_height;
                            self.captions(rt, m.leading.as_deref(), m.trailing.as_deref(), y + line / 2.0)?;
                            y += line;
                        }
                    }
                    (None, _) => match &rows[i] {
                        Row::Buttons(buttons) => y += self.button_row(rt, buttons, y, accent, &mut hits)?,
                        Row::Items(items) => y += self.cards(rt, items, y, accent, with, &mut hits)?,
                        Row::Search { query, buttons } => y += self.search(rt, with.query.unwrap_or(query), with.keyboard, buttons, y, accent, &mut hits)?,
                        Row::Tabs(tabs) => y += self.tabs(rt, tabs, y, &mut hits)?,
                        // Standing in for the cards under a search field, it keeps their
                        // height, so the panel doesn't jump while typing.
                        Row::Note(text) => {
                            let searchable = rows.iter().any(|r| matches!(r, Row::Search { .. }));
                            y += self.note(rt, text, y, if searchable { component::card::HEIGHT } else { self.fonts.note.style.line_height })?
                        }
                        Row::Slider { leading, value, action, trailing } => {
                            let value = with.slider.unwrap_or(*value);
                            let thumb = with.thumb == Some(Thumb::Slider);
                            y += self.slider(rt, leading, value, action, trailing.as_ref(), y, thumb, accent, &mut hits)?
                        }
                        Row::Lyrics(lines) => y += self.lyrics(rt, lines, track_at, y)?,
                        Row::Progress { .. } | Row::Timeline { .. } => {}
                    },
                }
                i += 1;
            }
            let natural = y + inset;
            if natural > island::expanded::MAX_HEIGHT {
                // Clamping would eat the bottom padding; make it visible instead.
                eprintln!("island: expanded content is {natural} tall, over max-height {}", island::expanded::MAX_HEIGHT);
            }
            height = natural.clamp(island::expanded::MIN_HEIGHT, island::expanded::MAX_HEIGHT);
            // Hits and bar positions are in surface coordinates.
            for hit in hits.iter_mut() {
                hit.left += dx;
                hit.right += dx;
                if let Some((l, r, length)) = hit.seek {
                    hit.seek = Some((l + dx, r + dx, length));
                }
                if let Some((l, r)) = hit.slider {
                    hit.slider = Some((l + dx, r + dx));
                }
            }
            bars = bars.map(|(x, mid, live)| (x + dx, mid, live));
            Ok(())
        })?;
        let radius = if width == island::expanded::WIDE_WIDTH { island::expanded::PANEL_RADIUS } else { island::expanded::RADIUS };
        Ok(ExpandedLayout { width, height, radius, hits, bars })
    }

    /// A bar from `span.0` to `span.1` at `y`: track, accent fill, and a thumb at the fill's
    /// end while hovered or dragged. A seekable one gets a generous hit around it. Returns
    /// its height.
    #[allow(clippy::too_many_arguments)]
    fn meter(&self, rt: &ID2D1RenderTarget, span: (f32, f32), y: f32, fraction: f32, seek: Option<u64>, thumb: bool, accent: Color, hits: &mut Vec<Hit>) -> Result<f32> {
        let (l, r) = span;
        let h = component::progress::HEIGHT;
        let end = l + (r - l) * fraction.clamp(0.0, 1.0);
        unsafe {
            self.round_rect(rt, l, y, r, y + h, h / 2.0, color::fill::TRACK)?;
            self.round_rect(rt, l, y, end, y + h, h / 2.0, accent)?;
        }
        if let Some(length) = seek {
            if thumb {
                self.thumb(rt, end, y + h / 2.0)?;
            }
            let pad = component::seek::HIT_PAD;
            hits.push(Hit { seek: Some((l, r, length)), ..Hit::new(l, y - pad, r, y + h + pad, "seek") });
        }
        Ok(h)
    }

    /// The round handle of a seekable bar, centred on (`x`, `mid`).
    fn thumb(&self, rt: &ID2D1RenderTarget, x: f32, mid: f32) -> Result<()> {
        let r = component::thumb::SIZE / 2.0 * self.scale;
        let circle = D2D1_ELLIPSE { point: self.point(x, mid), radiusX: r, radiusY: r };
        unsafe { rt.FillEllipse(&circle, &self.brush(rt, color::label::PRIMARY)?) };
        Ok(())
    }

    /// The accent washed in from the top-left corner, fading out by `tint-extent` of the
    /// width (a gradient, no blur). Drawn over the whole surface; the pill's shape clips it.
    fn wash(&self, rt: &ID2D1RenderTarget, w: f32, accent: Color) -> Result<()> {
        let radius = island::expanded::TINT_EXTENT * w * self.scale;
        let a = opacity::TINT_WASH;
        unsafe {
            // A softened falloff, so the wash has no visible edge.
            let stops = [
                D2D1_GRADIENT_STOP { position: 0.0, color: d2d(accent.with_alpha(a)) },
                D2D1_GRADIENT_STOP { position: 0.45, color: d2d(accent.with_alpha(a * 0.5)) },
                D2D1_GRADIENT_STOP { position: 1.0, color: d2d(accent.with_alpha(0.0)) },
            ];
            let collection = rt.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)?;
            let props = D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES { center: self.point(0.0, 0.0), gradientOriginOffset: Vector2::new(0.0, 0.0), radiusX: radius, radiusY: radius };
            let brush = rt.CreateRadialGradientBrush(&props, None, &collection)?;
            rt.FillRectangle(&self.rect(0.0, 0.0, w, EXPANDED_SURFACE.1), &brush);
        }
        Ok(())
    }

    /// "‹ 1/2 ›" ending at `right`, centred on `mid`: quiet tabular text between two chevron
    /// targets, the whole of it a wheel target. Returns its left edge.
    fn pager(&self, rt: &ID2D1RenderTarget, pager: &Pager, right: f32, mid: f32, hits: &mut Vec<Hit>) -> Result<f32> {
        let label = format!("{}/{}", pager.index + 1, pager.count);
        let font = &self.fonts.pager;
        let label_w = self.text_width(&label, font)?;
        let (hit, gap) = (component::pager::HIT, component::pager::GAP);
        let glyph = icon::PAGER;
        let (top, bottom) = (mid - hit / 2.0, mid + hit / 2.0);
        let next_l = right - hit;
        let label_l = next_l - gap - label_w;
        let prev_l = label_l - gap - hit;
        let ink = color::label::SECONDARY;
        self.text(rt, "\u{E76B}", &self.fonts.icon_pager, prev_l, mid - glyph / 2.0, hit, Align::Center, ink)?;
        self.text(rt, &label, font, label_l, mid - font.style.line_height / 2.0, label_w + 1.0, Align::Start, ink)?;
        self.text(rt, "\u{E76C}", &self.fonts.icon_pager, next_l, mid - glyph / 2.0, hit, Align::Center, ink)?;
        hits.push(Hit { pager: true, ..Hit::new(prev_l, top, prev_l + hit, bottom, &pager.prev) });
        hits.push(Hit { pager: true, ..Hit::new(next_l, top, right, bottom, &pager.next) });
        hits.push(Hit { pager: true, ..Hit::new(prev_l, top, right, bottom, "") });
        Ok(prev_l)
    }

    /// Volume and the like: the leading toggle (mute), the bar, and the trailing menu button
    /// (output device). Returns the row's height.
    #[allow(clippy::too_many_arguments)]
    fn slider(&self, rt: &ID2D1RenderTarget, leading: &Button, value: f32, action: &str, trailing: Option<&Menu>, y: f32, thumb: bool, accent: Color, hits: &mut Vec<Hit>) -> Result<f32> {
        let h = component::slider::HEIGHT;
        let gap = component::slider::GAP;
        let inset = island::expanded::INSET;
        let (mut l, mut r) = (inset, self.width.get() - inset);
        self.button(rt, leading, l, y, h, h, accent, hits)?;
        l += h + gap;
        if let Some(menu) = trailing {
            let button = Button { action: String::new(), content: menu.content.clone(), role: ButtonRole::Plain };
            self.button(rt, &button, r - h, y, h, h, accent, hits)?;
            if let Some(hit) = hits.last_mut() {
                hit.menu = true;
            }
            r -= h + gap;
        }
        let bar = component::progress::HEIGHT;
        let top = y + (h - bar) / 2.0;
        let end = l + (r - l) * value.clamp(0.0, 1.0);
        unsafe {
            self.round_rect(rt, l, top, r, top + bar, bar / 2.0, color::fill::TRACK)?;
            self.round_rect(rt, l, top, end, top + bar, bar / 2.0, accent)?;
        }
        if thumb {
            self.thumb(rt, end, y + h / 2.0)?;
        }
        // The whole row height, and a little past the ends so the extremes are easy to hit.
        let reach = component::thumb::SIZE / 2.0;
        hits.push(Hit { slider: Some((l, r)), ..Hit::new(l - reach, y, r + reach, y + h, action) });
        Ok(h)
    }

    /// The line being sung (primary) and the next one (secondary), centred; an empty line
    /// (an instrumental gap) shows nothing. Always the same height, so lines changing never
    /// resize the card.
    fn lyrics(&self, rt: &ID2D1RenderTarget, lines: &[LyricLine], position: Option<u64>, y: f32) -> Result<f32> {
        let font = &self.fonts.lyric;
        let line = font.style.line_height;
        let count = component::lyrics::LINES.max(1.0) as usize;
        let inset = island::expanded::INSET;
        let width = self.width.get() - 2.0 * inset;
        if let Some(position) = position {
            let current = music::current_line(lines, position);
            let next = current.map_or(0, |c| c + 1);
            let shown = [current.map(|c| (c, color::label::PRIMARY)), Some((next, color::label::SECONDARY))];
            for (k, (index, ink)) in shown.into_iter().flatten().take(count).enumerate() {
                if let Some(text) = lines.get(index).map(|l| l.text.trim()).filter(|t| !t.is_empty()) {
                    self.text(rt, text, font, inset, y + k as f32 * line, width, Align::Center, ink)?;
                }
            }
        }
        Ok(line * count as f32)
    }

    /// Do the times fit at the ends of a row of these glyph controls?
    fn fits_between(&self, buttons: &[Button], meter: &Meter) -> Result<bool> {
        let n = buttons.len() as f32;
        let group = n * component::button::TRANSPORT + (n - 1.0).max(0.0) * component::button::TRANSPORT_GAP;
        let free = (self.width.get() - 2.0 * island::expanded::INSET - group) / 2.0;
        let widest = self.caption_width(meter.leading.as_deref())?.max(self.caption_width(meter.trailing.as_deref())?);
        Ok(widest + space::SM <= free)
    }

    /// Width a caption needs, with every digit counted as the widest so a ticking time
    /// doesn't move what's next to it (tabular figures make that every digit).
    fn caption_width(&self, text: Option<&str>) -> Result<f32> {
        match text {
            Some(t) => self.text_width(&t.replace(|c: char| c.is_ascii_digit(), "0"), &self.fonts.caption),
            None => Ok(0.0),
        }
    }

    /// Elapsed / remaining times at the two ends of the bar, centred on `mid`.
    fn captions(&self, rt: &ID2D1RenderTarget, leading: Option<&str>, trailing: Option<&str>, mid: f32) -> Result<()> {
        let inset = island::expanded::INSET;
        let (l, r) = (inset, self.width.get() - inset);
        let top = mid - self.fonts.caption.style.line_height / 2.0;
        if let Some(text) = leading {
            self.text(rt, text, &self.fonts.caption, l, top, (r - l) / 2.0, Align::Start, color::label::TERTIARY)?;
        }
        if let Some(text) = trailing {
            self.text(rt, text, &self.fonts.caption, l + (r - l) / 2.0, top, (r - l) / 2.0, Align::End, color::label::TERTIARY)?;
        }
        Ok(())
    }

    /// All-glyph rows are transport-style circles centred as a group; any text makes the
    /// row equal-width capsules across the full width.
    fn button_row(&self, rt: &ID2D1RenderTarget, buttons: &[Button], y: f32, accent: Color, hits: &mut Vec<Hit>) -> Result<f32> {
        let n = buttons.len() as f32;
        let w = self.width.get();
        let inset = island::expanded::INSET;
        if buttons.iter().all(|b| matches!(b.content, ButtonContent::Glyph(_))) {
            let size = component::button::TRANSPORT;
            let gap = component::button::TRANSPORT_GAP;
            let mut x = (w - (n * size + (n - 1.0) * gap)) / 2.0;
            for button in buttons {
                self.button(rt, button, x, y, size, size, accent, hits)?;
                x += size + gap;
            }
            Ok(size)
        } else {
            let h = component::button::HEIGHT;
            let gap = component::button::GAP;
            let width = (w - 2.0 * inset - gap * (n - 1.0)) / n;
            for (i, button) in buttons.iter().enumerate() {
                self.button(rt, button, inset + i as f32 * (width + gap), y, width, h, accent, hits)?;
            }
            Ok(h)
        }
    }

    /// A capsule (or circle when square) button.
    #[allow(clippy::too_many_arguments)]
    fn button(&self, rt: &ID2D1RenderTarget, button: &Button, x: f32, y: f32, w: f32, h: f32, accent: Color, hits: &mut Vec<Hit>) -> Result<()> {
        let (fill, ink) = button_colors(button.role, accent);
        if let Some(fill) = fill {
            unsafe { self.round_rect(rt, x, y, x + w, y + h, h / 2.0, fill)? };
        }
        match &button.content {
            ButtonContent::Glyph(g) => {
                let size = self.fonts.icon_button.style.size;
                self.text(rt, &g.to_string(), &self.fonts.icon_button, x, y + (h - size) / 2.0, w, Align::Center, ink)?;
            }
            ButtonContent::Text(t) => {
                let line = self.fonts.button.style.line_height;
                self.text(rt, t, &self.fonts.button, x, y + (h - line) / 2.0, w, Align::Center, ink)?;
            }
        }
        hits.push(Hit::new(x, y, x + w, y + h, &button.action));
        Ok(())
    }

    fn measure(&self, element: &Element, slot: Slot) -> Result<(f32, f32)> {
        Ok(match element {
            Element::Empty => (0.0, 0.0),
            Element::Text { text, .. } => {
                let font = self.text_font(slot);
                (self.text_width(text, font)?.min(island::compact::MAX_SIDE), font.style.line_height)
            }
            Element::Glyph { .. } => {
                let s = self.glyph_font(slot).style.size;
                (s, s)
            }
            Element::Art(_) => {
                let s = art_shape(slot).0;
                (s, s)
            }
            Element::Ring(_) => {
                let s = ring_size(slot).0;
                (s, s)
            }
            Element::Bars { .. } => (bars_width(), component::bars::HEIGHT),
            Element::Dot => (component::dot::SIZE, component::dot::SIZE),
        })
    }

    /// Draws `element` anchored at `x` (by `align`) and vertically centred on `mid`.
    #[allow(clippy::too_many_arguments)]
    fn element(&self, rt: &ID2D1RenderTarget, element: &Element, slot: Slot, x: f32, mid: f32, align: Align, accent: Color) -> Result<()> {
        let (w, h) = self.measure(element, slot)?;
        let left = match align {
            Align::Start => x,
            Align::Center => x - w / 2.0,
            Align::End => x - w,
        };
        let top = mid - h / 2.0;
        match element {
            Element::Empty => {}
            Element::Text { text, tone } => self.text(rt, text, self.text_font(slot), left, top, w, Align::Start, tone_color(*tone, accent))?,
            Element::Glyph { glyph, tone } => self.text(rt, &glyph.to_string(), self.glyph_font(slot), left, top, w, Align::Center, tone_color(*tone, accent))?,
            Element::Art(art) => self.art(rt, art, slot, left, top, accent)?,
            Element::Ring(progress) => self.ring(rt, *progress, slot, left + w / 2.0, mid, accent)?,
            // Level bars are always live visuals the pill moves; nothing to draw here.
            Element::Bars { .. } => {}
            Element::Dot => unsafe {
                let r = component::dot::SIZE / 2.0;
                let ellipse = D2D1_ELLIPSE { point: self.point(left + r, mid), radiusX: r * self.scale, radiusY: r * self.scale };
                rt.FillEllipse(&ellipse, &self.brush(rt, accent)?);
            },
        }
        Ok(())
    }

    fn art(&self, rt: &ID2D1RenderTarget, art: &Art, slot: Slot, left: f32, top: f32, accent: Color) -> Result<()> {
        let (size, corner) = art_shape(slot);
        let r = self.rect(left, top, left + size, top + size);
        let shape = D2D1_ROUNDED_RECT { rect: r, radiusX: corner * self.scale, radiusY: corner * self.scale };
        unsafe {
            match art {
                Art::Gradient { from, to } => {
                    let stops = [
                        D2D1_GRADIENT_STOP { position: 0.0, color: d2d(rgb(*from)) },
                        D2D1_GRADIENT_STOP { position: 1.0, color: d2d(rgb(*to)) },
                    ];
                    let collection = rt.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)?;
                    let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: Vector2::new(r.left, r.top), endPoint: Vector2::new(r.right, r.bottom) };
                    rt.FillRoundedRectangle(&shape, &rt.CreateLinearGradientBrush(&props, None, &collection)?);
                }
                Art::Glyph(glyph) => {
                    rt.FillRoundedRectangle(&shape, &self.brush(rt, accent.with_alpha(opacity::TINT_FILL))?);
                    let font = if matches!(slot, Slot::Compact | Slot::Minimal) { &self.fonts.icon_tile_compact } else { &self.fonts.icon_tile };
                    let g = font.style.size;
                    self.text(rt, &glyph.to_string(), font, left, top + (size - g) / 2.0, size, Align::Center, accent)?;
                }
                Art::Logo { image, animated } => {
                    rt.FillRoundedRectangle(&shape, &self.brush(rt, accent.with_alpha(opacity::TINT_FILL))?);
                    // An animated compact logo is drawn by the pill so it can spin.
                    if !(*animated && slot == Slot::Compact) {
                        let mark = size * opacity::LOGO;
                        self.svg(rt, *image, left + (size - mark) / 2.0, top + (size - mark) / 2.0, mark, accent)?;
                    }
                }
                Art::Image(id) => match self.images.get(id) {
                    Some(Decoded::Bitmap(_)) => {
                        self.cover(rt, *id, &shape)?;
                        // A hairline just inside the edge, so a cover on black still reads
                        // as a tile on the black island.
                        let inner = D2D1_ROUNDED_RECT {
                            rect: D2D_RECT_F { left: r.left + 0.5, top: r.top + 0.5, right: r.right - 0.5, bottom: r.bottom - 0.5 },
                            radiusX: (shape.radiusX - 0.5).max(0.0),
                            radiusY: (shape.radiusY - 0.5).max(0.0),
                        };
                        rt.DrawRoundedRectangle(&inner, &self.brush(rt, color::label::PRIMARY.with_alpha(opacity::ART_STROKE))?, 1.0, None);
                    }
                    Some(Decoded::Svg(_)) => {
                        rt.FillRoundedRectangle(&shape, &self.brush(rt, color::fill::TERTIARY)?);
                        let mark = size * opacity::LOGO;
                        self.svg(rt, *id, left + (size - mark) / 2.0, top + (size - mark) / 2.0, mark, color::label::PRIMARY)?;
                    }
                    // Not arrived yet (or undecodable): a quiet placeholder of the same shape.
                    None => rt.FillRoundedRectangle(&shape, &self.brush(rt, color::fill::TERTIARY)?),
                },
            }
        }
        Ok(())
    }

    /// Fills `shape` with an image, cover-fitted (centre-cropped to the shape's aspect).
    fn cover(&self, rt: &ID2D1RenderTarget, id: ImageId, shape: &D2D1_ROUNDED_RECT) -> Result<()> {
        let Some(Decoded::Bitmap(image)) = self.images.get(&id) else { return Ok(()) };
        let r = shape.rect;
        unsafe {
            let bitmap = rt.CreateBitmapFromWicBitmap(image, None)?;
            let D2D_SIZE_F { width: iw, height: ih } = bitmap.GetSize();
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            let k = (w / iw).max(h / ih);
            let brush = rt.CreateBitmapBrush(
                &bitmap,
                Some(&D2D1_BITMAP_BRUSH_PROPERTIES {
                    extendModeX: D2D1_EXTEND_MODE_CLAMP,
                    extendModeY: D2D1_EXTEND_MODE_CLAMP,
                    interpolationMode: D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                }),
                None,
            )?;
            brush.SetTransform(&Matrix3x2 { M11: k, M12: 0.0, M21: 0.0, M22: k, M31: r.left + (w - iw * k) / 2.0, M32: r.top + (h - ih * k) / 2.0 });
            rt.FillRoundedRectangle(shape, &brush);
        }
        Ok(())
    }

    /// A row of rich preview cards; returns its height. Four across when they fit, else four
    /// and a half, scrolled by `with.scroll` and clipped at the panel's edges; only cards in
    /// view are drawn and hit-tested. Each card carries its own footer (source app icon,
    /// age, one fact), so nothing sits under the cards.
    fn cards(&self, rt: &ID2D1RenderTarget, items: &[Item], y: f32, accent: Color, with: &Interaction, hits: &mut Vec<Hit>) -> Result<f32> {
        let w = self.width.get();
        let row = CardRow::new(w, items.len());
        let pad = component::card::PADDING;
        let footer = component::card::FOOTER;
        let cw = row.card_width();
        let ch = component::card::HEIGHT;
        let s = self.scale;
        let scroll = row.clamp(with.scroll);
        let ring = component::card::RING_GAP + component::card::RING_WIDTH;
        // Cut at the content inset, not the surface edge: the pill's corners curve in there.
        let inset = island::expanded::INSET;
        unsafe { rt.PushAxisAlignedClip(&self.rect(inset - ring, y - ring, w - inset + ring, y + ch + ring), D2D1_ANTIALIAS_MODE_ALIASED) };
        let result = (|| -> Result<()> {
            for i in row.visible(scroll) {
                let item = &items[i];
                let l = row.left(i, scroll);
                let (r, b) = (l + cw, y + ch);
                let radius = component::card::RADIUS * s;
                let shape = D2D1_ROUNDED_RECT { rect: self.rect(l, y, r, b), radiusX: radius, radiusY: radius };
                let body = (l + pad, y + pad, cw - 2.0 * pad, ch - footer - 2.0 * pad);
                let selected = with.keyboard && i == with.selected;
                let card_fill = if selected { color::fill::SECONDARY } else { color::fill::TERTIARY };
                let well = |tint: Color| unsafe {
                    rt.FillRoundedRectangle(&shape, &self.brush(rt, tint)?);
                    Ok::<(), windows::core::Error>(())
                };
                // Footer ink: light over images and dark swatches, quiet over wells.
                let mut on_media = false;
                match &item.preview {
                    Preview::Image(id) => {
                        if matches!(self.images.get(id), Some(Decoded::Bitmap(_))) {
                            // Behind transparent images, so the card keeps its shape.
                            well(color::fill::TERTIARY)?;
                            self.cover(rt, *id, &shape)?;
                            self.scrim(rt, &shape, b - footer - pad * 2.0, b)?;
                            on_media = true;
                        } else {
                            // Bytes not here yet: the type's glyph, so the card isn't an empty well.
                            well(color::fill::TERTIARY)?;
                            let g = icon::TILE;
                            self.text(rt, "\u{EB9F}", &self.fonts.icon_tile, l, body.1 + (body.3 - g) / 2.0, cw, Align::Center, color::label::TERTIARY)?;
                        }
                    }
                    Preview::Text(text) => {
                        well(card_fill)?;
                        // Plain text is the default kind: no badge (code gets one).
                        let first_w = body.2;
                        match text.trim().split_once('\n') {
                            // Several lines: the first reads as a title, the rest quieter.
                            Some((first, rest)) if !first.trim().is_empty() => {
                                let line = self.fonts.card_label.style.line_height;
                                self.text(rt, first.trim(), &self.fonts.card_label, body.0, body.1, first_w, Align::Start, color::label::PRIMARY)?;
                                let top = body.1 + line + space::XS;
                                self.text_block(rt, rest.trim_start(), &self.fonts.card_text, body.0, top, body.2, body.1 + body.3 - top, color::label::SECONDARY)?;
                            }
                            _ => self.text_block(rt, text.trim(), &self.fonts.card_text, body.0, body.1, first_w, body.3, color::label::PRIMARY)?,
                        }
                    }
                    Preview::Code(text) => {
                        well(card_fill)?;
                        self.kind_glyph(rt, '\u{E943}', r - pad, body.1)?;
                        self.text_block(rt, text, &self.fonts.card_code, body.0, body.1, body.2 - component::card::KIND_ICON - space::XS, body.3, color::label::PRIMARY)?;
                    }
                    Preview::Color(rgb) => {
                        let swatch = rgb_color(*rgb);
                        well(swatch)?;
                        let light = 0.2126 * swatch.r + 0.7152 * swatch.g + 0.0722 * swatch.b > 0.55;
                        let ink = if light { color::label::ON_ACCENT } else { color::label::PRIMARY };
                        let Rgb(cr, cg, cb) = *rgb;
                        self.text(rt, &format!("#{cr:02X}{cg:02X}{cb:02X}"), &self.fonts.card_label, body.0, body.1, body.2, Align::Start, ink)?;
                        on_media = !light;
                    }
                    Preview::Link { host, path, title, image, .. } => {
                        well(card_fill)?;
                        let mut text_top = body.1;
                        if let Some(id) = image.filter(|id| matches!(self.images.get(id), Some(Decoded::Bitmap(_)))) {
                            // The page image across the top, rounded only where the card is.
                            let media_b = y + ch * component::card::MEDIA;
                            unsafe { rt.PushAxisAlignedClip(&self.rect(l, y, r, media_b), D2D1_ANTIALIAS_MODE_ALIASED) };
                            self.cover(rt, id, &shape)?;
                            unsafe { rt.PopAxisAlignedClip() };
                            text_top = media_b + pad * 0.75;
                        } else {
                            self.text(rt, "\u{E774}", &self.fonts.icon_compact, body.0, body.1, icon::COMPACT, Align::Start, accent)?;
                            text_top += icon::COMPACT + pad * 0.5;
                        }
                        // The site goes in the footer (the module sends it as the card's fact).
                        let heading = title.clone().unwrap_or_else(|| format!("{host}{path}"));
                        self.text_block(rt, &heading, &self.fonts.card_title, body.0, text_top, body.2, b - footer - pad * 0.5 - text_top, color::label::PRIMARY)?;
                    }
                    Preview::Files { name, .. } => {
                        well(card_fill)?;
                        self.text(rt, "\u{E8A5}", &self.fonts.icon_tile, body.0, body.1, icon::TILE, Align::Start, accent)?;
                        let name_top = body.1 + icon::TILE + pad * 0.75;
                        self.text_block(rt, name, &self.fonts.card_title, body.0, name_top, body.2, body.1 + body.3 - name_top, color::label::PRIMARY)?;
                    }
                }
                self.card_footer(rt, item, l, b - footer - pad * 0.25, cw, on_media)?;
                if selected {
                    // Concentric with the card: its radius grows by exactly the ring's offset.
                    let out = component::card::RING_GAP + component::card::RING_WIDTH / 2.0;
                    let rr = (component::card::RADIUS + out) * s;
                    let ring = D2D1_ROUNDED_RECT { rect: self.rect(l - out, y - out, r + out, b + out), radiusX: rr, radiusY: rr };
                    unsafe { rt.DrawRoundedRectangle(&ring, &self.brush(rt, accent)?, component::card::RING_WIDTH * s, None) };
                }
                hits.push(Hit { card: Some(i), ..Hit::new(l.max(0.0), y, r.min(w), b, &item.action) });
            }
            // The row runs on past the edge: fade it out there instead of cutting it.
            let fade = component::card::FADE;
            if scroll < row.max_scroll() - 0.5 {
                self.edge_fade(rt, w - inset + ring - fade, w - inset + ring, y - ring, y + ch + ring, true)?;
            }
            if scroll > 0.5 {
                self.edge_fade(rt, inset - ring, inset - ring + fade, y - ring, y + ch + ring, false)?;
            }
            Ok(())
        })();
        unsafe { rt.PopAxisAlignedClip() };
        result?;
        Ok(ch)
    }

    /// The island's background fading in across `left..right` (towards the right edge when
    /// `rightward`), over the card row.
    #[allow(clippy::too_many_arguments)]
    fn edge_fade(&self, rt: &ID2D1RenderTarget, left: f32, right: f32, top: f32, bottom: f32, rightward: bool) -> Result<()> {
        let s = self.scale;
        let bg = color::island::BACKGROUND;
        let clear = D2D1_COLOR_F { r: bg.r, g: bg.g, b: bg.b, a: 0.0 };
        let solid = D2D1_COLOR_F { r: bg.r, g: bg.g, b: bg.b, a: 1.0 };
        let (from, to) = if rightward { (clear, solid) } else { (solid, clear) };
        unsafe {
            let stops = [D2D1_GRADIENT_STOP { position: 0.0, color: from }, D2D1_GRADIENT_STOP { position: 1.0, color: to }];
            let collection = rt.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)?;
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: Vector2::new(left * s, 0.0), endPoint: Vector2::new(right * s, 0.0) };
            let brush = rt.CreateLinearGradientBrush(&props, None, &collection)?;
            rt.FillRectangle(&self.rect(left, top, right, bottom), &brush);
        }
        Ok(())
    }

    /// A card's small type glyph, right-aligned at `right`, top at `top`.
    fn kind_glyph(&self, rt: &ID2D1RenderTarget, glyph: char, right: f32, top: f32) -> Result<()> {
        let size = component::card::KIND_ICON;
        self.text(rt, &glyph.to_string(), &self.fonts.icon_kind, right - size, top, size, Align::Center, color::label::TERTIARY)
    }

    /// Source app icon, age, a star when pinned, and one fact on the right, inside the
    /// bottom of a card. The age keeps its width; the fact ellipsises in what's left.
    fn card_footer(&self, rt: &ID2D1RenderTarget, item: &Item, l: f32, top: f32, w: f32, on_media: bool) -> Result<()> {
        let pad = component::card::PADDING;
        let icon = component::card::ICON;
        let mid = top + component::card::FOOTER / 2.0;
        let (ink, quiet) = if on_media { (color::label::PRIMARY, color::label::PRIMARY.with_alpha(0.75)) } else { (color::label::SECONDARY, color::label::TERTIARY) };
        let mut x = l + pad;
        let right = l + w - pad;
        if let Some(id) = item.icon.filter(|id| matches!(self.images.get(id), Some(Decoded::Bitmap(_)))) {
            let r = icon * 0.25 * self.scale;
            let shape = D2D1_ROUNDED_RECT { rect: self.rect(x, mid - icon / 2.0, x + icon, mid + icon / 2.0), radiusX: r, radiusY: r };
            self.cover(rt, id, &shape)?;
            x += icon + pad * 0.5;
        }
        let line = self.fonts.caption.style.line_height;
        let age_w = self.text_width(&item.age, &self.fonts.caption)?.min((right - x).max(0.0));
        self.text(rt, &item.age, &self.fonts.caption, x, mid - line / 2.0, age_w, Align::Start, ink)?;
        x += age_w;
        if item.pinned {
            let star = icon::STAR;
            x += pad * 0.5;
            self.text(rt, "\u{E735}", &self.fonts.icon_star, x, mid - star / 2.0, star, Align::Center, ink)?;
            x += star;
        }
        if let Some(meta) = &item.meta {
            let from = x + pad * 0.75;
            if right > from {
                self.text(rt, meta, &self.fonts.caption, from, mid - line / 2.0, right - from, Align::End, quiet)?;
            }
        }
        Ok(())
    }

    /// The search field (magnifier, text or the quiet placeholder, a steady caret while
    /// the panel has the keyboard) with its icon buttons at the right; returns its height.
    #[allow(clippy::too_many_arguments)]
    fn search(&self, rt: &ID2D1RenderTarget, text: &str, caret: bool, buttons: &[Button], y: f32, accent: Color, hits: &mut Vec<Hit>) -> Result<f32> {
        let h = component::search::HEIGHT;
        let pad = component::search::PADDING;
        let inset = island::expanded::INSET;
        let mut right = self.width.get() - inset;
        for button in buttons.iter().rev() {
            let bw = match &button.content {
                ButtonContent::Glyph(_) => h,
                ButtonContent::Text(t) => self.text_width(t, &self.fonts.button)? + 2.0 * pad,
            };
            right -= bw;
            self.button(rt, button, right, y, bw, h, accent, hits)?;
            right -= component::search::BUTTON_GAP;
        }
        unsafe { self.round_rect(rt, inset, y, right, y + h, h / 2.0, color::fill::QUATERNARY)? };
        let glyph = icon::SEARCH;
        self.text(rt, "\u{E721}", &self.fonts.icon_search, inset + pad, y + (h - glyph) / 2.0, glyph, Align::Center, color::label::SECONDARY)?;
        let left = inset + pad + glyph + space::SM;
        let avail = (right - pad - left).max(0.0);
        let line = self.fonts.search.style.line_height;
        let top = y + (h - line) / 2.0;
        let mut end = left;
        if text.is_empty() {
            self.text(rt, "Search", &self.fonts.search, left, top, avail, Align::Start, color::label::TERTIARY)?;
        } else {
            // Long queries show their end, where the typing is.
            let tw = self.text_width(text, &self.fonts.search)?;
            let x = if tw > avail { left + avail - tw } else { left };
            unsafe { rt.PushAxisAlignedClip(&self.rect(left, y, left + avail + component::search::CARET, y + h), D2D1_ANTIALIAS_MODE_ALIASED) };
            let drawn = self.text(rt, text, &self.fonts.search, x, top, tw + 1.0, Align::Start, color::label::PRIMARY);
            unsafe { rt.PopAxisAlignedClip() };
            drawn?;
            end = x + tw;
        }
        if caret {
            let ch = line * 0.9;
            let x = (end * self.scale).round() / self.scale;
            unsafe { self.round_rect(rt, x, y + (h - ch) / 2.0, x + component::search::CARET, y + (h + ch) / 2.0, component::search::CARET / 2.0, accent)? };
        }
        Ok(h)
    }

    /// Filter tabs: "Label count" capsules, the selected one filled. When they're wider than
    /// the panel the row slides to keep the selected one in view, clipped at the edges.
    fn tabs(&self, rt: &ID2D1RenderTarget, tabs: &[Tab], y: f32, hits: &mut Vec<Hit>) -> Result<f32> {
        let h = component::tabs::HEIGHT;
        let pad = component::tabs::PADDING;
        let gap = component::tabs::GAP;
        let inset = island::expanded::INSET;
        let w = self.width.get();
        let mut sizes = Vec::with_capacity(tabs.len());
        for tab in tabs {
            let label = self.text_width(&tab.label, &self.fonts.tab)?;
            let count = self.text_width(&tab.count.to_string(), &self.fonts.tab_count)?;
            sizes.push((label, count, pad + label + component::tabs::COUNT_GAP + count + pad));
        }
        let widths: Vec<f32> = sizes.iter().map(|s| s.2).collect();
        let offset = panel::tabs_offset(&widths, gap, tabs.iter().position(|t| t.selected), w - 2.0 * inset);
        let line = self.fonts.tab.style.line_height;
        let top = y + (h - line) / 2.0;
        unsafe { rt.PushAxisAlignedClip(&self.rect(inset, y, w - inset, y + h), D2D1_ANTIALIAS_MODE_ALIASED) };
        let result = (|| -> Result<()> {
            let mut x = inset - offset;
            for (tab, (label_w, count_w, tw)) in tabs.iter().zip(sizes) {
                if x + tw > 0.0 && x < w {
                    // Plain text; the selected tab sits on a soft fill (no bright pill).
                    let (ink, quiet) = if tab.selected { (color::label::PRIMARY, color::label::SECONDARY) } else { (color::label::SECONDARY, color::label::TERTIARY) };
                    if tab.selected {
                        unsafe { self.round_rect(rt, x, y, x + tw, y + h, h / 2.0, color::fill::SECONDARY)? };
                    }
                    self.text(rt, &tab.label, &self.fonts.tab, x + pad, top, label_w + 1.0, Align::Start, ink)?;
                    self.text(rt, &tab.count.to_string(), &self.fonts.tab_count, x + tw - pad - count_w, top, count_w + 1.0, Align::Start, quiet)?;
                    hits.push(Hit::new(x.max(0.0), y, (x + tw).min(w), y + h, &tab.action));
                }
                x += tw + gap;
            }
            Ok(())
        })();
        unsafe { rt.PopAxisAlignedClip() };
        result?;
        Ok(h)
    }

    /// One quiet centred line in a row `height` tall.
    fn note(&self, rt: &ID2D1RenderTarget, text: &str, y: f32, height: f32) -> Result<f32> {
        let inset = island::expanded::INSET;
        let line = self.fonts.note.style.line_height;
        self.text(rt, text, &self.fonts.note, inset, y + (height - line) / 2.0, self.width.get() - 2.0 * inset, Align::Center, color::label::TERTIARY)?;
        Ok(height)
    }

    /// A dark gradient over the bottom of a card so the footer reads on any image.
    fn scrim(&self, rt: &ID2D1RenderTarget, shape: &D2D1_ROUNDED_RECT, from: f32, to: f32) -> Result<()> {
        let s = self.scale;
        unsafe {
            let stops = [
                D2D1_GRADIENT_STOP { position: 0.0, color: D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 } },
                D2D1_GRADIENT_STOP { position: 1.0, color: D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.7 } },
            ];
            let collection = rt.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)?;
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: Vector2::new(0.0, from * s), endPoint: Vector2::new(0.0, to * s) };
            let brush = rt.CreateLinearGradientBrush(&props, None, &collection)?;
            rt.PushAxisAlignedClip(&D2D_RECT_F { left: shape.rect.left, top: from * s, right: shape.rect.right, bottom: shape.rect.bottom }, D2D1_ANTIALIAS_MODE_ALIASED);
            rt.FillRoundedRectangle(shape, &brush);
            rt.PopAxisAlignedClip();
        }
        Ok(())
    }

    /// Wrapped text in whole lines only, the last one ellipsised if it doesn't all fit.
    #[allow(clippy::too_many_arguments)]
    fn text_block(&self, rt: &ID2D1RenderTarget, text: &str, font: &Font, left: f32, top: f32, width: f32, height: f32, color: Color) -> Result<()> {
        let line = font.style.line_height;
        let height = (height / line).floor() * line;
        if height < line {
            return Ok(());
        }
        let wide = utf16_prefix(text, 600);
        unsafe {
            let layout = self.dwrite.CreateTextLayout(&wide, &font.format, width * self.scale, height * self.scale)?;
            layout.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, font.style.line_height * self.scale, font.style.line_height * 0.8 * self.scale)?;
            rt.DrawTextLayout(self.point(left, top), &layout, &self.brush(rt, color)?, D2D1_DRAW_TEXT_OPTIONS_CLIP | D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT);
        }
        Ok(())
    }

    /// Draws a single-colour SVG mark into a `size` square at (`left`, `top`), filled with
    /// `color`. Missing or non-SVG images draw nothing.
    fn svg(&self, rt: &ID2D1RenderTarget, image: ImageId, left: f32, top: f32, size: f32, color: Color) -> Result<()> {
        let Some(Decoded::Svg(text)) = self.images.get(&image) else { return Ok(()) };
        let [r, g, b, _] = color.to_rgba8();
        let tinted = text.replacen("<svg", &format!("<svg fill=\"#{r:02X}{g:02X}{b:02X}\" fill-opacity=\"{}\"", color.a), 1);
        let s = self.scale;
        unsafe {
            let Some(stream) = SHCreateMemStream(Some(tinted.as_bytes())) else { return Ok(()) };
            let dc: ID2D1DeviceContext5 = rt.cast()?;
            let document = dc.CreateSvgDocument(&stream, D2D_SIZE_F { width: size * s, height: size * s })?;
            let mut previous = Matrix3x2::default();
            rt.GetTransform(&mut previous);
            rt.SetTransform(&Matrix3x2 { M31: previous.M31 + left * s, M32: previous.M32 + top * s, ..previous });
            dc.DrawSvgDocument(&document);
            rt.SetTransform(&previous);
        }
        Ok(())
    }

    fn ring(&self, rt: &ID2D1RenderTarget, progress: f32, slot: Slot, cx: f32, cy: f32, accent: Color) -> Result<()> {
        let (size, stroke) = ring_size(slot);
        let s = self.scale;
        // Centre on a pixel boundary (or pixel centre for odd-width strokes) so the stroke
        // edges land on whole pixels instead of being antialiased across two.
        let odd = ((stroke * s).round() as i32) % 2 == 1;
        let snap = |v: f32| ((v * s).round() + if odd { 0.5 } else { 0.0 }) / s;
        let (cx, cy) = (snap(cx), snap(cy));
        let radius = (size - stroke) / 2.0;
        let progress = progress.clamp(0.0, 1.0);
        unsafe {
            let circle = D2D1_ELLIPSE { point: self.point(cx, cy), radiusX: radius * s, radiusY: radius * s };
            rt.DrawEllipse(&circle, &self.brush(rt, color::fill::TRACK)?, stroke * s, &self.round);
            if progress >= 0.999 {
                rt.DrawEllipse(&circle, &self.brush(rt, accent)?, stroke * s, &self.round);
                return Ok(());
            }
            if progress <= 0.001 {
                return Ok(());
            }
            // Clockwise from 12 o'clock.
            let angle = -std::f32::consts::FRAC_PI_2 + progress * std::f32::consts::TAU;
            let base: &ID2D1Factory = &self.factory;
            let path = base.CreatePathGeometry()?;
            let sink = path.Open()?;
            sink.BeginFigure(self.point(cx, cy - radius), D2D1_FIGURE_BEGIN_HOLLOW);
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: self.point(cx + radius * angle.cos(), cy + radius * angle.sin()),
                size: D2D_SIZE_F { width: radius * s, height: radius * s },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: if progress > 0.5 { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
            });
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
            sink.Close()?;
            if opacity::GLOW > 0.0 && !matches!(slot, Slot::Compact | Slot::Minimal) {
                rt.DrawGeometry(&path, &self.brush(rt, accent.with_alpha(opacity::GLOW))?, stroke * 2.0 * s, &self.round);
            }
            rt.DrawGeometry(&path, &self.brush(rt, accent)?, stroke * s, &self.round);
        }
        Ok(())
    }

    fn text_font(&self, slot: Slot) -> &Font {
        match slot {
            Slot::Compact | Slot::Minimal => &self.fonts.compact,
            Slot::Leading | Slot::Trailing => &self.fonts.detail,
        }
    }

    fn glyph_font(&self, slot: Slot) -> &Font {
        match slot {
            Slot::Compact | Slot::Minimal => &self.fonts.icon_compact,
            Slot::Leading => &self.fonts.icon_tile,
            Slot::Trailing => &self.fonts.icon_trailing,
        }
    }

    fn layout(&self, text: &str, font: &Font, max_width: f32) -> Result<IDWriteTextLayout> {
        let wide: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            let layout = self.dwrite.CreateTextLayout(&wide, &font.format, max_width.max(0.0) * self.scale, font.style.line_height * self.scale)?;
            if font.tabular {
                layout.SetTypography(&self.tabular, DWRITE_TEXT_RANGE { startPosition: 0, length: wide.len() as u32 })?;
            }
            Ok(layout)
        }
    }

    fn text_width(&self, text: &str, font: &Font) -> Result<f32> {
        let layout = self.layout(text, font, 10_000.0)?;
        let mut metrics = DWRITE_TEXT_METRICS::default();
        unsafe { layout.GetMetrics(&mut metrics)? };
        Ok((metrics.widthIncludingTrailingWhitespace / self.scale).ceil())
    }

    /// One line of text in a `width`-wide box whose top is `top`; ellipsised if it doesn't fit.
    #[allow(clippy::too_many_arguments)]
    fn text(&self, rt: &ID2D1RenderTarget, text: &str, font: &Font, left: f32, top: f32, width: f32, align: Align, color: Color) -> Result<()> {
        let layout = self.layout(text, font, width)?;
        unsafe {
            layout.SetTextAlignment(match align {
                Align::Start => DWRITE_TEXT_ALIGNMENT_LEADING,
                Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
                Align::End => DWRITE_TEXT_ALIGNMENT_TRAILING,
            })?;
            rt.DrawTextLayout(self.point(left, top), &layout, &self.brush(rt, color)?, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT);
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn round_rect(&self, rt: &ID2D1RenderTarget, l: f32, t: f32, r: f32, b: f32, radius: f32, color: Color) -> Result<()> {
        let shape = D2D1_ROUNDED_RECT { rect: self.rect(l, t, r, b), radiusX: radius * self.scale, radiusY: radius * self.scale };
        rt.FillRoundedRectangle(&shape, &self.brush(rt, color)?);
        Ok(())
    }

    fn brush(&self, rt: &ID2D1RenderTarget, color: Color) -> Result<ID2D1SolidColorBrush> {
        unsafe { rt.CreateSolidColorBrush(&d2d(color), None) }
    }

    fn rect(&self, l: f32, t: f32, r: f32, b: f32) -> D2D_RECT_F {
        let s = self.scale;
        D2D_RECT_F { left: l * s, top: t * s, right: r * s, bottom: b * s }
    }

    fn point(&self, x: f32, y: f32) -> Vector2 {
        Vector2::new(x * self.scale, y * self.scale)
    }

    /// Clear a composition surface and draw into it with a pixel-unit render target.
    fn paint(&self, surface: &CompositionDrawingSurface, f: impl FnOnce(&ID2D1RenderTarget) -> Result<()>) -> Result<()> {
        let interop: ICompositionDrawingSurfaceInterop = surface.cast()?;
        let mut offset = POINT::default();
        let dc: ID2D1DeviceContext = unsafe { interop.BeginDraw(None, &mut offset)? };
        let rt: &ID2D1RenderTarget = &dc;
        let result = unsafe {
            rt.SetDpi(96.0, 96.0);
            rt.SetTransform(&Matrix3x2::translation(offset.x as f32, offset.y as f32));
            rt.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
            rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            f(rt)
        };
        unsafe { interop.EndDraw()? };
        result
    }
}

/// A bar row as drawn: progress, a timeline, or a live stream (no bar, a fact instead).
struct Meter {
    fraction: Option<f32>,
    leading: Option<String>,
    trailing: Option<String>,
    /// The track length, when the bar seeks.
    seek: Option<u64>,
}

impl Meter {
    fn has_times(&self) -> bool {
        self.leading.is_some() || self.trailing.is_some()
    }
}

#[derive(Clone, Copy)]
enum Align {
    Start,
    Center,
    End,
}

/// Drops `width`/`height` from the root `<svg>` tag so the document fills whatever viewport
/// it's drawn into (icon sets often say `width="1em"`), scaling by its `viewBox`.
fn fit_svg(text: &str) -> String {
    let Some(start) = text.find("<svg") else { return text.to_string() };
    let Some(len) = text[start..].find('>') else { return text.to_string() };
    let tag = &text[start..start + len];
    let mut fitted = tag.to_string();
    for attr in [" width=", " height="] {
        if let Some(at) = fitted.find(attr) {
            let value_start = at + attr.len();
            let quote = fitted[value_start..].chars().next().unwrap_or('"');
            if let Some(end) = fitted[value_start + 1..].find(quote) {
                fitted.replace_range(at..value_start + 1 + end + 1, "");
            }
        }
    }
    format!("{}{}{}", &text[..start], fitted, &text[start + len..])
}

/// At most `max` UTF-16 units of `text`, never ending in half a surrogate pair (an emoji
/// cut in two draws as a box).
fn utf16_prefix(text: &str, max: usize) -> Vec<u16> {
    let mut wide: Vec<u16> = text.encode_utf16().take(max + 1).collect();
    if wide.len() > max {
        wide.truncate(max);
        if wide.last().is_some_and(|u| (0xD800..0xDC00).contains(u)) {
            wide.pop();
        }
    }
    wide
}

/// 83_000 → "1:23", 3_723_000 → "1:02:03".
fn clock(ms: u64) -> String {
    let s = ms / 1000;
    match s / 3600 {
        0 => format!("{}:{:02}", s / 60, s % 60),
        h => format!("{h}:{:02}:{:02}", s / 60 % 60, s % 60),
    }
}

/// Time left, rounded up so elapsed + left add up to the length (0:10 and -3:10 of 3:20).
fn clock_left(ms: u64) -> String {
    clock(ms.div_ceil(1000) * 1000)
}

pub fn bars_width() -> f32 {
    BARS as f32 * component::bars::WIDTH + (BARS as f32 - 1.0) * component::bars::GAP
}

fn inline_width(n: usize) -> f32 {
    let n = n as f32;
    n * component::button::INLINE + (n - 1.0) * component::button::INLINE_GAP
}

/// Round content sits concentric in the capsule's end; text keeps clear of the curve.
fn compact_margin(element: &Element) -> f32 {
    match element {
        Element::Art(_) | Element::Ring(_) => island::compact::SIDE_MARGIN,
        _ => island::compact::TEXT_MARGIN,
    }
}

/// (size, corner radius): circles in the compact capsule, minimal bubble and trailing
/// slot; concentric with the island's corner in the leading slot.
fn art_shape(slot: Slot) -> (f32, f32) {
    match slot {
        Slot::Compact | Slot::Minimal => (component::art::COMPACT, component::art::COMPACT / 2.0),
        Slot::Leading => (component::art::EXPANDED, radius::CONCENTRIC),
        Slot::Trailing => (component::art::TRAILING, component::art::TRAILING / 2.0),
    }
}

/// (diameter, stroke)
fn ring_size(slot: Slot) -> (f32, f32) {
    match slot {
        Slot::Compact | Slot::Minimal => (component::ring::COMPACT, component::ring::STROKE_COMPACT),
        Slot::Leading | Slot::Trailing => (component::ring::EXPANDED, component::ring::STROKE_EXPANDED),
    }
}

/// (fill, ink)
fn button_colors(role: ButtonRole, accent: Color) -> (Option<Color>, Color) {
    match role {
        ButtonRole::Primary => (Some(color::label::PRIMARY), color::label::ON_ACCENT),
        ButtonRole::Secondary => (Some(color::fill::SECONDARY), color::label::PRIMARY),
        ButtonRole::Accent => (Some(accent.with_alpha(opacity::TINT_FILL)), accent),
        ButtonRole::Destructive => (Some(color::system::RED.with_alpha(opacity::TINT_FILL)), color::system::RED),
        ButtonRole::Plain => (None, color::label::PRIMARY),
    }
}

fn tone_color(tone: Tone, accent: Color) -> Color {
    match tone {
        Tone::Primary => color::label::PRIMARY,
        Tone::Secondary => color::label::SECONDARY,
        Tone::Accent => accent,
        Tone::Positive => color::system::GREEN,
        Tone::Warning => color::system::ORANGE,
        Tone::Negative => color::system::RED,
    }
}

pub fn rgb(Rgb(r, g, b): Rgb) -> Color {
    rgb_color(Rgb(r, g, b))
}

fn rgb_color(Rgb(r, g, b): Rgb) -> Color {
    Color::from_rgb8(r, g, b)
}

fn d2d(c: Color) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: c.r, g: c.g, b: c.b, a: c.a }
}

/// `wrap`: multi-line text, top-aligned, ellipsised on the last line that fits.
fn make_font(dwrite: &IDWriteFactory, style: TextStyle, tabular: bool, wrap: bool, scale: f32) -> Result<Font> {
    unsafe {
        let format = dwrite.CreateTextFormat(
            &HSTRING::from(style.family),
            None,
            DWRITE_FONT_WEIGHT(style.weight as i32),
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            style.size * scale,
            w!("en-us"),
        )?;
        if wrap {
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
            format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)?;
        } else {
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        }
        let sign = dwrite.CreateEllipsisTrimmingSign(&format)?;
        let trimming = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
        format.SetTrimming(&trimming, &sign)?;
        Ok(Font { format, style, tabular })
    }
}

fn create_d3d_device() -> Result<ID3D11Device> {
    let mut result = Err(windows::core::Error::empty());
    for driver in [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP] {
        let mut device = None;
        result = unsafe { D3D11CreateDevice(None, driver, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut device), None, None) }
            .map(|_| device.unwrap());
        if result.is_ok() {
            break;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::fit_svg;

    #[test]
    fn clicking_a_seekable_bar_seeks_proportionally() {
        let hit = super::Hit { seek: Some((20.0, 400.0, 200_000)), ..super::Hit::new(20.0, 0.0, 400.0, 10.0, "seek") };
        assert_eq!(hit.action_at(210.0), "seek:100000");
        assert_eq!(hit.action_at(0.0), "seek:0");
        assert_eq!(hit.action_at(999.0), "seek:200000");
    }

    #[test]
    fn truncation_never_splits_a_surrogate_pair() {
        let text = "ab\u{1F600}c";
        assert_eq!(super::utf16_prefix(text, 3), vec![0x61, 0x62]);
        assert_eq!(super::utf16_prefix(text, 4).len(), 4);
        assert_eq!(super::utf16_prefix(text, 99).len(), 5);
    }

    #[test]
    fn clocks_read_like_players_show_them() {
        assert_eq!(super::clock(83_400), "1:23");
        assert_eq!(super::clock(5_000), "0:05");
        assert_eq!(super::clock(3_723_000), "1:02:03");
    }

    #[test]
    fn svg_root_size_is_dropped_so_it_fills_its_slot() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="1em" height='1em' viewBox="0 0 128 128"><path width="5"/></svg>"#;
        assert_eq!(fit_svg(svg), r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 128 128"><path width="5"/></svg>"#);
        assert_eq!(fit_svg("<svg viewBox=\"0 0 24 24\"/>"), "<svg viewBox=\"0 0 24 24\"/>");
    }
}
