//! A pop-up button's menu: an opaque rounded panel over the page with a hairline edge and a
//! soft stepped shadow (no blur), items highlighted in the accent, the checked one marked.

use reui_design::tokens::{color, prefs as s};
use reui_design::Color;
use windows::core::Result;

use super::frame::View;
use super::{Align, Painter};
use crate::ui::layout::{Font, Rect};
use crate::ui::{icons, Menu};

const WHITE: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };

pub fn draw(p: &Painter, _v: &View, menu: &Menu) -> Result<()> {
    let r = menu.rect;
    for (grow, alpha) in [(10.0, 0.05), (6.0, 0.08), (3.0, 0.12), (1.0, 0.2)] {
        p.fill(r.offset(0.0, grow / 2.0).inflate(grow), s::menu::RADIUS + grow, Color { r: 0.0, g: 0.0, b: 0.0, a: alpha })?;
    }
    p.fill(r, s::menu::RADIUS, s::color::MENU)?;
    p.edge(r, s::menu::RADIUS, s::color::MENU_EDGE)?;
    let pad = s::menu::PADDING;
    let h = s::menu::ITEM_HEIGHT;
    let inner = Rect::new(r.l + pad, r.t + pad, r.r - pad, r.b - pad);
    p.clip(inner, || {
        for (i, item) in menu.items.iter().enumerate() {
            let top = inner.t + i as f32 * h - menu.scroll;
            if top + h < inner.t || top > inner.b {
                continue;
            }
            let row = Rect::new(inner.l, top, inner.r, top + h);
            let hot = menu.hot == Some(i);
            if hot {
                p.fill(row, s::menu::ITEM_RADIUS, s::color::ACCENT)?;
            }
            let ink = if hot { WHITE } else { color::label::PRIMARY };
            let mark = Rect::at(row.l + 6.0, row.t, s::control::GLYPH, row.h());
            if menu.checked == Some(i) {
                p.glyph(icons::CHECK, s::control::GLYPH, mark, ink)?;
            }
            let text = Rect::new(mark.r + 7.0, row.t, row.r - s::control::PADDING, row.b);
            p.text(item, Font::Control, text, Align::Start, ink)?;
        }
        Ok(())
    })
}

/// Where a menu for a control at `anchor` (window coordinates) goes: below it, or above
/// when there's no room, right-aligned with it, at least `min-width` wide.
pub fn place(anchor: Rect, items: usize, widest: f32, window: (f32, f32)) -> Rect {
    let pad = s::menu::PADDING;
    let content = items as f32 * s::menu::ITEM_HEIGHT + 2.0 * pad;
    let h = content.min(s::menu::MAX_HEIGHT);
    let w = (widest + 6.0 + s::control::GLYPH + 7.0 + s::control::PADDING + 2.0 * pad).max(s::menu::MIN_WIDTH).max(anchor.w());
    let r = anchor.r.min(window.0 - pad);
    let l = (r - w).max(pad);
    let below = anchor.b + s::menu::OFFSET;
    let t = if below + h <= window.1 - pad { below } else { (anchor.t - s::menu::OFFSET - h).max(s::toolbar::HEIGHT) };
    Rect::new(l, t, l + w, t + h)
}

/// The entry at window point (`x`, `y`).
pub fn item_at(menu: &Menu, x: f32, y: f32) -> Option<usize> {
    let pad = s::menu::PADDING;
    let inner = Rect::new(menu.rect.l + pad, menu.rect.t + pad, menu.rect.r - pad, menu.rect.b - pad);
    if !inner.contains(x, y) {
        return None;
    }
    let i = ((y - inner.t + menu.scroll) / s::menu::ITEM_HEIGHT).floor() as usize;
    (i < menu.items.len()).then_some(i)
}

/// How far a menu can scroll.
pub fn max_scroll(menu: &Menu) -> f32 {
    let content = menu.items.len() as f32 * s::menu::ITEM_HEIGHT;
    (content - (menu.rect.h() - 2.0 * s::menu::PADDING)).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::Target;

    #[test]
    fn menus_open_below_or_above_and_hit_their_items() {
        let anchor = Rect::new(500.0, 100.0, 620.0, 124.0);
        let below = place(anchor, 3, 60.0, (1000.0, 700.0));
        assert_eq!(below.t, anchor.b + s::menu::OFFSET);
        assert_eq!(below.r, anchor.r);
        assert!(below.w() >= s::menu::MIN_WIDTH);
        let low = Rect::new(500.0, 650.0, 620.0, 674.0);
        let above = place(low, 3, 60.0, (1000.0, 700.0));
        assert_eq!(above.b, low.t - s::menu::OFFSET);
        let menu = Menu { target: Target::new("x", 0), items: vec!["a".into(), "b".into(), "c".into()], checked: None, rect: below, hot: None, scroll: 0.0 };
        let y = below.t + s::menu::PADDING + s::menu::ITEM_HEIGHT * 1.5;
        assert_eq!(item_at(&menu, below.l + 20.0, y), Some(1));
        assert_eq!(item_at(&menu, below.l - 5.0, y), None);
        assert_eq!(max_scroll(&menu), 0.0);
        let long = place(anchor, 40, 60.0, (1000.0, 700.0));
        assert_eq!(long.h(), s::menu::MAX_HEIGHT);
    }
}
