//! The horizontally scrolling row of item cards (clipboard history, the shelf): scrolling,
//! clipping, the edge fades, selection and the hover actions. Each card's face is
//! `faces.rs`.


use reui_design::layout::cards::{CardRow, Face};
use reui_design::tokens::{component, opacity};
use reui_design::Color;
use reui_protocol::{Item, ItemKey};
use windows::core::Result;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows_numerics::Vector2;

use crate::render::{Align, Hit, Interaction, Renderer};

impl Renderer {
    /// A row of rich preview cards; returns its height. Three across when they fit, else
    /// three and the edge of a fourth, scrolled by `with.scroll` and clipped at the panel's
    /// edges; only cards in view are drawn and hit-tested.
    pub(in crate::render) fn cards(&self, rt: &ID2D1RenderTarget, items: &[Item], y: f32, accent: Color, with: &Interaction, hits: &mut Vec<Hit>) -> Result<f32> {
        let w = self.width.get();
        let row = CardRow::new(w, items.len(), self.look.inset());
        let cw = row.card_width();
        let ch = component::card::HEIGHT;
        let s = self.scale;
        let scroll = row.clamp(with.scroll);
        let ring = component::card::RING_GAP + component::card::RING_WIDTH;
        // Cut at the content inset, not the surface edge: the pill's corners curve in there.
        let inset = self.look.inset();
        unsafe { rt.PushAxisAlignedClip(&self.rect(inset - ring, y - ring, w - inset + ring, y + ch + ring), D2D1_ANTIALIAS_MODE_ALIASED) };
        let result = (|| -> Result<()> {
            for i in row.visible(scroll) {
                let item = &items[i];
                let l = row.left(i, scroll);
                let (r, b) = (l + cw, y + ch);
                let radius = self.concentric() * s;
                let shape = D2D1_ROUNDED_RECT { rect: self.rect(l, y, r, b), radiusX: radius, radiusY: radius };
                let selected = with.keyboard && i == with.selected;
                self.face(rt, item, &shape, &Face::new(l, y, cw), selected, accent)?;
                if selected {
                    // Concentric with the card: its radius grows by exactly the ring's offset.
                    let out = component::card::RING_GAP + component::card::RING_WIDTH / 2.0;
                    let rr = (self.concentric() + out) * s;
                    let ring = D2D1_ROUNDED_RECT { rect: self.rect(l - out, y - out, r + out, b + out), radiusX: rr, radiusY: rr };
                    unsafe { rt.DrawRoundedRectangle(&ring, &self.brush(rt, accent)?, component::card::RING_WIDTH * s, None) };
                }
                if with.hover_card == Some(i) {
                    self.card_actions(rt, item, r, y, accent, hits)?;
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
        let bg = self.pal().background;
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

    /// Delete and pin on the card under the pointer: circles in its top-right corner over the
    /// island's background, so they read over text, images and swatches alike; a pinned
    /// card's star is filled, in the accent. Their hits come before the card's and carry no
    /// card index, so a click pins or deletes and the panel stays open (a card click pastes
    /// and closes it).
    fn card_actions(&self, rt: &ID2D1RenderTarget, item: &Item, right: f32, top: f32, accent: Color, hits: &mut Vec<Hit>) -> Result<()> {
        let size = component::card::ACTION;
        let gap = component::card::ACTION_GAP;
        let find = |key: ItemKey| item.actions.iter().find(|a| a.key == Some(key));
        let star = if item.pinned { '\u{E735}' } else { '\u{E734}' };
        let fill = self.pal().background.with_alpha(opacity::CARD_ACTION);
        let glyph = component::card::KIND_ICON;
        let y = top + gap;
        let mut x = right - gap - size;
        for (action, g, lit) in [(find(ItemKey::Delete), '\u{E74D}', false), (find(ItemKey::CtrlP), star, item.pinned)] {
            let Some(action) = action else { continue };
            let r = size / 2.0 * self.scale;
            let circle = D2D1_ELLIPSE { point: self.point(x + size / 2.0, y + size / 2.0), radiusX: r, radiusY: r };
            unsafe { rt.FillEllipse(&circle, &self.brush(rt, fill)?) };
            let ink = if lit { accent } else { self.pal().label_primary };
            self.text(rt, &g.to_string(), &self.fonts.icon_kind, x, y + (size - glyph) / 2.0, size, Align::Center, ink)?;
            hits.push(Hit::new(x, y, x + size, y + size, &action.action));
            x -= size + gap;
        }
        Ok(())
    }
}
