//! The island's component library, drawn with Direct2D/DirectWrite into composition
//! surfaces. Every size, colour and font comes from `reui_design::tokens`, and where each
//! piece goes from `reui_design::layout` (pure, shared with Settings' previews, so they
//! can't drift); this module measures text, feeds it the protocol's semantic view model
//! and draws what it lays out. Here:
//! the `Renderer`, surfaces and painting, and the layout/hit types the pill reads back;
//! each presentation, element kind and row type draws in its own child module.
//!
//! Layout rules (see DESIGN.md):
//! - Hierarchy: expanded text reads eyebrow (accent) → hero → detail (secondary).
//! - Concentric corners: shapes follow the island's curve. In the compact capsule, round
//!   content is a circle inset so it sits concentric with the pill's ends; in the expanded
//!   island, corner tiles use `radius.concentric` and buttons are capsules.
//!
//! Layout is in DIPs; `scale` converts to pixels at draw time.

use std::cell::Cell;
use std::collections::HashMap;

use reui_design::layout;
use reui_design::look::{concentric, Look};
use reui_design::tokens::{component, font, icon, island, typography, Palette};
use reui_design::{Color, TextStyle};
use reui_protocol::{CardSize, ImageId, Rgb, Tone};
use windows::core::{Interface, Result};
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
use windows::Win32::System::WinRT::Composition::{ICompositionDrawingSurfaceInterop, ICompositorInterop};
use windows_numerics::{Matrix3x2, Vector2};

use crate::features::music::{self, SeekHold};

use self::fonts::{make_font, Fonts};
use self::images::Decoded;

mod compact;
mod elements;
mod expanded;
mod fonts;
mod images;
mod minimal;
mod notch;
mod rows;
#[cfg(test)]
mod sheet;
mod text;

pub use reui_design::layout::compact::{BARS, BAR_LEVELS as BAR_LEVELS_LIVE, MAX_WIDTH as COMPACT_MAX_WIDTH, MINIMAL_SURFACE, SURFACE as COMPACT_SURFACE};
pub use reui_design::layout::expanded_surface;
pub use self::notch::EAR_SURFACE;

/// Resting height of every bar while not live (flat).
pub const BAR_LEVEL_IDLE: f32 = 0.3;

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
    /// A text block: the wheel over it scrolls it.
    pub block: bool,
    /// A choice: its index in the list, so a click also moves the keyboard cursor there.
    pub choice: Option<usize>,
}

impl Hit {
    fn new(left: f32, top: f32, right: f32, bottom: f32, action: &str) -> Self {
        Hit { left, top, right, bottom, action: action.to_string(), seek: None, card: None, slider: None, menu: false, pager: false, block: false, choice: None }
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

/// The card (in a row of cards) under (`x`, `y`), wherever on it the point is: also over
/// the hover actions drawn on top of it, whose own hits come first.
pub fn card_at(hits: &[Hit], x: f32, y: f32) -> Option<usize> {
    hits.iter().filter(|h| h.card.is_some()).find(|h| x >= h.left && x < h.right && y >= h.top && y < h.bottom)?.card
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
    /// A slider shown at a value other than the module's (dragging, or just set): its
    /// action, the value, and whether it's being dragged (it then shows its percentage).
    pub slider: Option<(&'a str, f32, bool)>,
    /// The bar under the pointer (or being dragged) shows its thumb.
    pub thumb: Option<Thumb>,
    /// The card under the pointer shows its pin and delete actions.
    pub hover_card: Option<usize>,
    /// A lyric line just started: the line before it and how far (0..=1, eased) the column
    /// has slid up from it.
    pub lyric: Option<(usize, f32)>,
    /// The front activity's level as shown right now (eased), over the view's own.
    pub level: Option<f32>,
    /// How far the card's scrolling body (its text block or choice list) is scrolled, in
    /// DIPs (the renderer clamps it).
    pub block_scroll: f32,
    /// The choice under the keyboard cursor, while the card has the keyboard.
    pub choice: Option<usize>,
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
    /// How far the scrolling body (a text block or choice list) can scroll, in DIPs.
    pub block_max: f32,
    /// Each choice's top and bottom in the list (unscrolled) and how tall the list shows,
    /// so the keyboard cursor can scroll its choice into view.
    pub choices: Option<(Vec<(f32, f32)>, f32)>,
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
    /// Pixels per DIP: the display's scale times the look's size.
    scale: f32,
    /// Colours and geometry (Settings > Appearance); see [`Renderer::retarget`].
    look: Look,
    /// Width of the expanded view being drawn (normal or wide).
    width: Cell<f32>,
    /// Size of the expanded view being drawn: a roomy card lays out its buttons and detail
    /// like an alert.
    size: Cell<CardSize>,
}

impl Renderer {
    pub fn new(compositor: &Compositor, scale: f32, look: Look) -> Result<Self> {
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
        let fonts = fonts(&dwrite, scale)?;
        Ok(Self { graphics, factory, dwrite, wic, images: HashMap::new(), tabular, round, fonts, scale, look, width: Cell::new(island::expanded::WIDTH), size: Cell::new(CardSize::Regular) })
    }

    /// A new look, or the same look at another scale (Size, another display): the device,
    /// the factories and the decoded images stay; only the fonts depend on the scale.
    /// Building a renderer from nothing takes ~140 ms, which a style switch shouldn't cost.
    pub fn retarget(&mut self, scale: f32, look: Look) -> Result<()> {
        if scale != self.scale {
            self.fonts = fonts(&self.dwrite, scale)?;
            self.scale = scale;
        }
        self.look = look;
        Ok(())
    }

    pub fn surface(&self, dips: (f32, f32)) -> Result<CompositionDrawingSurface> {
        let size = Size { Width: (dips.0 * self.scale).ceil(), Height: (dips.1 * self.scale).ceil() };
        self.graphics.CreateDrawingSurface(size, DirectXPixelFormat::B8G8R8A8UIntNormalized, DirectXAlphaMode::Premultiplied)
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

impl Renderer {
    /// The theme's colours.
    fn pal(&self) -> &Palette {
        &self.look.palette
    }

    pub fn look(&self) -> &Look {
        &self.look
    }

    fn tone_color(&self, tone: Tone, accent: Color) -> Color {
        match tone {
            Tone::Primary => self.pal().label_primary,
            Tone::Secondary => self.pal().label_secondary,
            Tone::Accent => accent,
            Tone::Positive => self.pal().system_green,
            Tone::Warning => self.pal().system_orange,
            Tone::Negative => self.pal().system_red,
        }
    }

    /// The open card's corner radius: the wide panel's or the card's, from the roundness.
    pub fn card_radius(&self, size: CardSize) -> f32 {
        card(size).radius(&self.look)
    }

    /// Radius of a shape one inset inside the open card's edge (tiles, wells, cards).
    fn concentric(&self) -> f32 {
        concentric(self.card_radius(self.size.get()), self.look.inset())
    }

    /// A control `h` tall at `inset` from the open card's edge: a capsule while the
    /// concentric radius allows one, else concentric.
    fn capsule(&self, h: f32, inset: f32) -> f32 {
        self.look.capsule(h, self.card_radius(self.size.get()), inset)
    }
}

/// The layout's name for a card's size (`reui-design` doesn't see the protocol).
pub fn card(size: CardSize) -> layout::Card {
    match size {
        CardSize::Regular => layout::Card::Regular,
        CardSize::Roomy => layout::Card::Roomy,
        CardSize::Wide => layout::Card::Wide,
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


/// `a` blended towards `b` by `t` (0..=1), channel by channel.
fn mix(a: Color, b: Color, t: f32) -> Color {
    let l = |x: f32, y: f32| x + (y - x) * t.clamp(0.0, 1.0);
    Color { r: l(a.r, b.r), g: l(a.g, b.g), b: l(a.b, b.b), a: l(a.a, b.a) }
}

/// Every text style the island draws, at `scale`.
fn fonts(dwrite: &IDWriteFactory, scale: f32) -> Result<Fonts> {
    let icons = |size: f32| TextStyle { family: font::ICONS, weight: 400, size, line_height: size };
    let f = |style: TextStyle, tabular: bool| make_font(&dwrite, style, tabular, false, scale);
    let wrapped = |style: TextStyle| make_font(&dwrite, style, false, true, scale);
    Ok(Fonts {
        eyebrow: f(typography::EYEBROW, false)?,
        value: f(typography::VALUE, true)?,
        headline: f(typography::HEADLINE, false)?,
        code: f(typography::CODE, false)?,
        detail: f(typography::DETAIL, false)?,
        detail_wrap: wrapped(typography::DETAIL)?,
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
        lyric_wrap: f(typography::LYRIC, true)?,
        pager: f(typography::PAGER, true)?,
        context: f(typography::CONTEXT, false)?,
        choice: f(typography::CHOICE, false)?,
        choice_detail: f(typography::CHOICE_DETAIL, false)?,
        choice_detail_wrap: wrapped(typography::CHOICE_DETAIL)?,
        icon_check: f(icons(component::choice::CHECK), false)?,
        block_code: f(typography::BLOCK_CODE, false)?,
        block_prose: wrapped(typography::BLOCK_PROSE)?,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_cards_hover_actions_take_the_click_and_keep_the_hover() {
        // As the card row pushes them: the actions first, then the card under them.
        let hits = [super::Hit::new(100.0, 4.0, 124.0, 28.0, "delete:7"), super::Hit { card: Some(3), ..super::Hit::new(0.0, 0.0, 128.0, 116.0, "paste:7") }];
        assert_eq!(super::hit_at(&hits, 110.0, 10.0).map(|h| (h.action.as_str(), h.card)), Some(("delete:7", None)), "a click on the bin deletes, without the card's paste");
        assert_eq!(super::card_at(&hits, 110.0, 10.0), Some(3), "the card stays hovered over its own actions");
        assert_eq!(super::card_at(&hits, 50.0, 60.0), Some(3));
        assert_eq!(super::card_at(&hits, 200.0, 60.0), None);
    }

    #[test]
    fn clicking_a_seekable_bar_seeks_proportionally() {
        let hit = super::Hit { seek: Some((20.0, 400.0, 200_000)), ..super::Hit::new(20.0, 0.0, 400.0, 10.0, "seek") };
        assert_eq!(hit.action_at(210.0), "seek:100000");
        assert_eq!(hit.action_at(0.0), "seek:0");
        assert_eq!(hit.action_at(999.0), "seek:200000");
    }
}
