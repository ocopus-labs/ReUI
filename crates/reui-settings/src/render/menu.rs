//! A dropdown's menu: an opaque rounded panel over the page, items as capsules
//! concentric with it, the checked one marked in the accent.

use reui_design::tokens::{color, settings as s};
use windows::core::Result;

use super::{Align, Painter};
use crate::ui::layout::{Font, Rect};
use crate::ui::Menu;

pub fn draw(p: &Painter, menu: &Menu) -> Result<()> {
    let r = menu.rect;
    p.fill(r, s::menu::RADIUS, s::color::MENU)?;
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
            if menu.hot == Some(i) {
                p.fill(row, h / 2.0, color::fill::SECONDARY)?;
            }
            let icon = s::icon::CONTROL;
            let mark = Rect::new(row.l + s::control::PADDING - 4.0, row.t, row.l + s::control::PADDING - 4.0 + icon, row.b);
            if menu.checked == Some(i) {
                p.glyph('\u{E73E}', icon, mark, color::accent::DEFAULT)?;
            }
            let text = Rect::new(mark.r + s::control::GAP, row.t, row.r - s::control::PADDING, row.b);
            p.text(item, Font::Nav, text, Align::Start, color::label::PRIMARY)?;
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
    let w = (widest + 2.0 * s::control::PADDING + s::icon::CONTROL + s::control::GAP + 2.0 * pad).max(s::menu::MIN_WIDTH).max(anchor.w());
    let r = anchor.r.min(window.0 - pad);
    let l = (r - w).max(pad);
    let below = anchor.b + s::menu::OFFSET;
    let t = if below + h <= window.1 - pad { below } else { (anchor.t - s::menu::OFFSET - h).max(s::caption::HEIGHT) };
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
        let anchor = Rect::new(500.0, 100.0, 620.0, 132.0);
        let below = place(anchor, 3, 60.0, (1000.0, 700.0));
        assert_eq!(below.t, anchor.b + s::menu::OFFSET);
        assert_eq!(below.r, anchor.r);
        assert!(below.w() >= s::menu::MIN_WIDTH);
        let low = Rect::new(500.0, 650.0, 620.0, 682.0);
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
