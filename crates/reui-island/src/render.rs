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

use std::collections::HashMap;

use reui_design::tokens::{color, component, font, icon, island, opacity, radius, space, typography};
use reui_design::{Color, TextStyle};
use reui_protocol::{Art, Button, ButtonContent, ButtonRole, CompactView, Element, ExpandedView, Headline, ImageId, Rgb, Row, Tone};
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

/// Widest compact presentation: two maximal sides around the centre gap.
pub const COMPACT_MAX_WIDTH: f32 = island::compact::CENTER_GAP + 2.0 * (island::compact::MAX_SIDE + island::compact::TEXT_MARGIN);
pub const COMPACT_SURFACE: (f32, f32) = (COMPACT_MAX_WIDTH, island::HEIGHT);
pub const EXPANDED_SURFACE: (f32, f32) = (island::expanded::WIDTH, island::expanded::MAX_HEIGHT);
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
}

impl Hit {
    /// The action a click at `x` (card DIPs) sends.
    pub fn action_at(&self, x: f32) -> String {
        match self.seek {
            Some((left, right, length)) => {
                let fraction = ((x - left) / (right - left).max(1.0)).clamp(0.0, 1.0);
                format!("seek:{}", (fraction as f64 * length as f64) as u64)
            }
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

pub struct ExpandedLayout {
    pub height: f32,
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
        let f = |style: TextStyle, tabular: bool| make_font(&dwrite, style, tabular, scale);
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
        };
        Ok(Self { graphics, factory, dwrite, wic, images: HashMap::new(), tabular, round, fonts, scale })
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
    pub fn draw_expanded(&self, surface: &CompositionDrawingSurface, view: Option<&ExpandedView>, accent: Color) -> Result<ExpandedLayout> {
        let mut height = island::expanded::MIN_HEIGHT;
        let mut hits = Vec::new();
        let mut bars = None;
        self.paint(surface, |rt| {
            let Some(view) = view else { return Ok(()) };
            let w = island::expanded::WIDTH;
            let inset = island::expanded::INSET;
            let gap = island::expanded::COLUMN_GAP;

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
            let header_h = lh.max(th).max(text_h);
            let mid = inset + header_h / 2.0;

            self.element(rt, &view.leading, Slot::Leading, inset, mid, Align::Start, accent)?;
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

            // Text column: eyebrow (accent) → hero (primary) → detail (secondary).
            let mut y = mid - text_h / 2.0;
            if let Some(eyebrow) = &view.eyebrow {
                self.text(rt, eyebrow, &self.fonts.eyebrow, text_left, y, text_w, Align::Start, accent)?;
                y += eyebrow_h;
            }
            match &view.headline {
                Headline::Value(s) => self.text(rt, s, &self.fonts.value, text_left, y, text_w, Align::Start, color::label::PRIMARY)?,
                Headline::Text(s) => self.text(rt, s, &self.fonts.headline, text_left, y, text_w, Align::Start, color::label::PRIMARY)?,
                Headline::Code(s) => {
                    let (px, py) = (component::chip::PADDING_X, component::chip::PADDING_Y);
                    let chip_w = (self.text_width(s, &self.fonts.code)? + 2.0 * px).min(text_w);
                    unsafe { self.round_rect(rt, text_left, y, text_left + chip_w, y + hero_h, radius::CHIP, color::fill::TERTIARY)? };
                    self.text(rt, s, &self.fonts.code, text_left + px, y + py, chip_w - 2.0 * px, Align::Start, color::label::PRIMARY)?;
                }
            }
            y += hero_h;
            if let Some(detail) = &view.detail {
                self.text(rt, detail, &self.fonts.detail, text_left, y + line_gap, text_w, Align::Start, color::label::SECONDARY)?;
            }

            // Full-width rows. A progress bar with times followed by icon-only controls shares
            // one line under the bar: times at the ends, controls in the otherwise empty middle.
            let mut y = inset + header_h;
            let mut i = 0;
            while i < rows.len() {
                y += if i == 0 { island::expanded::SECTION_GAP } else { island::expanded::ROW_GAP };
                let progress = match &rows[i] {
                    Row::Progress { fraction, leading, trailing } => Some((*fraction, leading.clone(), trailing.clone())),
                    Row::Timeline { position_ms, length_ms, playing, at_unix_ms, seekable } => {
                        let at = Row::timeline_position(*position_ms, *length_ms, *playing, *at_unix_ms, reui_protocol::unix_ms());
                        if *seekable {
                            // Generous touch target around the thin bar.
                            let (l, r) = (inset, w - inset);
                            let pad = component::seek::HIT_PAD;
                            let bar = component::progress::HEIGHT;
                            hits.push(Hit { left: l, top: y - pad, right: r, bottom: y + bar + pad, action: "seek".into(), seek: Some((l, r, *length_ms)) });
                        }
                        Some((at as f32 / (*length_ms).max(1) as f32, Some(clock(at)), Some(format!("-{}", clock(length_ms - at)))))
                    }
                    Row::Buttons(_) => None,
                };
                let controls = match rows.get(i + 1) {
                    Some(Row::Buttons(b)) if b.iter().all(|b| matches!(b.content, ButtonContent::Glyph(_))) => Some(b),
                    _ => None,
                };
                match (progress, controls) {
                    (Some((fraction, leading, trailing)), Some(buttons)) if leading.is_some() || trailing.is_some() => {
                        y += self.bar(rt, fraction, y, accent)? + space::SM;
                        let row_h = self.button_row(rt, buttons, y, accent, &mut hits)?;
                        self.captions(rt, leading.as_deref(), trailing.as_deref(), y + row_h / 2.0)?;
                        y += row_h;
                        i += 2;
                        continue;
                    }
                    (Some((fraction, leading, trailing)), _) => {
                        y += self.bar(rt, fraction, y, accent)?;
                        if leading.is_some() || trailing.is_some() {
                            y += space::SM;
                            let line = self.fonts.caption.style.line_height;
                            self.captions(rt, leading.as_deref(), trailing.as_deref(), y + line / 2.0)?;
                            y += line;
                        }
                    }
                    (None, _) => {
                        if let Row::Buttons(buttons) = &rows[i] {
                            y += self.button_row(rt, buttons, y, accent, &mut hits)?;
                        }
                    }
                }
                i += 1;
            }
            let natural = y + inset;
            if natural > island::expanded::MAX_HEIGHT {
                // Clamping would eat the bottom padding; make it visible instead.
                eprintln!("island: expanded content is {natural} tall, over max-height {}", island::expanded::MAX_HEIGHT);
            }
            height = natural.clamp(island::expanded::MIN_HEIGHT, island::expanded::MAX_HEIGHT);
            Ok(())
        })?;
        Ok(ExpandedLayout { height, hits, bars })
    }

    /// The full-width progress bar; returns its height.
    fn bar(&self, rt: &ID2D1RenderTarget, fraction: f32, y: f32, accent: Color) -> Result<f32> {
        let inset = island::expanded::INSET;
        let (l, r) = (inset, island::expanded::WIDTH - inset);
        let h = component::progress::HEIGHT;
        unsafe {
            self.round_rect(rt, l, y, r, y + h, h / 2.0, color::fill::TRACK)?;
            self.round_rect(rt, l, y, l + (r - l) * fraction.clamp(0.0, 1.0), y + h, h / 2.0, accent)?;
        }
        Ok(h)
    }

    /// Elapsed / remaining times at the two ends of the bar, centred on `mid`.
    fn captions(&self, rt: &ID2D1RenderTarget, leading: Option<&str>, trailing: Option<&str>, mid: f32) -> Result<()> {
        let inset = island::expanded::INSET;
        let (l, r) = (inset, island::expanded::WIDTH - inset);
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
        let w = island::expanded::WIDTH;
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
        hits.push(Hit { left: x, top: y, right: x + w, bottom: y + h, action: button.action.clone(), seek: None });
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
                    Some(Decoded::Bitmap(image)) => {
                        // Cover-fit: centre-crop to a square, then scale onto the shape.
                        let bitmap = rt.CreateBitmapFromWicBitmap(image, None)?;
                        let D2D_SIZE_F { width: iw, height: ih } = bitmap.GetSize();
                        let side = iw.min(ih);
                        let k = (r.right - r.left) / side;
                        let brush = rt.CreateBitmapBrush(
                            &bitmap,
                            Some(&D2D1_BITMAP_BRUSH_PROPERTIES {
                                extendModeX: D2D1_EXTEND_MODE_CLAMP,
                                extendModeY: D2D1_EXTEND_MODE_CLAMP,
                                interpolationMode: D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                            }),
                            None,
                        )?;
                        brush.SetTransform(&Matrix3x2 {
                            M11: k,
                            M12: 0.0,
                            M21: 0.0,
                            M22: k,
                            M31: r.left - (iw - side) / 2.0 * k,
                            M32: r.top - (ih - side) / 2.0 * k,
                        });
                        rt.FillRoundedRectangle(&shape, &brush);
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

/// 83_000 → "1:23", 3_723_000 → "1:02:03".
fn clock(ms: u64) -> String {
    let s = ms / 1000;
    match s / 3600 {
        0 => format!("{}:{:02}", s / 60, s % 60),
        h => format!("{h}:{:02}:{:02}", s / 60 % 60, s % 60),
    }
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
    Color::from_rgb8(r, g, b)
}

fn d2d(c: Color) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: c.r, g: c.g, b: c.b, a: c.a }
}

fn make_font(dwrite: &IDWriteFactory, style: TextStyle, tabular: bool, scale: f32) -> Result<Font> {
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
        format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
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
        let hit = super::Hit { left: 20.0, top: 0.0, right: 400.0, bottom: 10.0, action: "seek".into(), seek: Some((20.0, 400.0, 200_000)) };
        assert_eq!(hit.action_at(210.0), "seek:100000");
        assert_eq!(hit.action_at(0.0), "seek:0");
        assert_eq!(hit.action_at(999.0), "seek:200000");
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
