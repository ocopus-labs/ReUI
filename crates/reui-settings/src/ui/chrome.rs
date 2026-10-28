//! The window around the page, in window DIPs: the sidebar (search, the ReUI row, the nav
//! in sections), the toolbar (back, forward, title, window buttons), the page column, the
//! scrollbar and the scroll maths. Pure and tested.

use reui_design::tokens::prefs as s;

use super::layout::Rect;

/// The page column's width in a window `window_w` wide.
pub fn column_width(window_w: f32) -> f32 {
    (window_w - s::sidebar::WIDTH - 2.0 * s::page::PADDING_X).clamp(0.0, s::page::MAX_WIDTH)
}

/// The page column's left edge: after the sidebar, centred in the space right of it once
/// the column is at its widest.
pub fn page_left(window_w: f32) -> f32 {
    let area = window_w - s::sidebar::WIDTH;
    s::sidebar::WIDTH + ((area - column_width(window_w)) / 2.0).max(s::page::PADDING_X)
}

pub fn search() -> Rect {
    let inset = s::sidebar::INSET;
    Rect::new(inset, s::sidebar::SEARCH_TOP, s::sidebar::WIDTH - inset, s::sidebar::SEARCH_TOP + s::sidebar::SEARCH_HEIGHT)
}

/// The ReUI row under the search field.
pub fn account() -> Rect {
    let inset = s::sidebar::INSET;
    Rect::new(inset, s::sidebar::ACCOUNT_TOP, s::sidebar::WIDTH - inset, s::sidebar::ACCOUNT_TOP + s::sidebar::ACCOUNT_HEIGHT)
}

/// The nav items, top to bottom; `sections[i]` is the section of the ith item, and a gap
/// opens where the section changes.
pub fn nav_items(sections: &[usize]) -> Vec<Rect> {
    let inset = s::sidebar::INSET;
    let mut y = account().b + s::sidebar::SECTION_GAP;
    let mut out = Vec::with_capacity(sections.len());
    for (i, section) in sections.iter().enumerate() {
        if i > 0 && sections[i - 1] != *section {
            y += s::sidebar::SECTION_GAP;
        }
        out.push(Rect::new(inset, y, s::sidebar::WIDTH - inset, y + s::sidebar::ITEM_HEIGHT));
        y += s::sidebar::ITEM_HEIGHT;
    }
    out
}

/// Back and forward, at the toolbar's start.
pub fn history_buttons() -> [Rect; 2] {
    let size = s::toolbar::BUTTON;
    let top = (s::toolbar::HEIGHT - size) / 2.0;
    let back = s::sidebar::WIDTH + s::toolbar::INSET;
    let forward = back + size + s::toolbar::BUTTON_GAP;
    [Rect::at(back, top, size, size), Rect::at(forward, top, size, size)]
}

/// Where the toolbar's title starts.
pub fn title_left() -> f32 {
    history_buttons()[1].r + s::toolbar::TITLE_GAP
}

/// Minimise and close, at the toolbar's end.
pub fn caption_buttons(window_w: f32) -> [Rect; 2] {
    let size = s::toolbar::BUTTON;
    let top = (s::toolbar::HEIGHT - size) / 2.0;
    let close = window_w - s::toolbar::INSET - size;
    let min = close - s::toolbar::BUTTON_GAP - size;
    [Rect::at(min, top, size, size), Rect::at(close, top, size, size)]
}

/// How far the page can scroll.
pub fn max_scroll(content_h: f32, window_h: f32) -> f32 {
    (content_h - window_h).max(0.0)
}

/// The scroll that keeps `rect` (page coordinates) in view below the toolbar, moving as
/// little as possible.
pub fn scroll_to_show(scroll: f32, rect: Rect, window_h: f32, max: f32) -> f32 {
    let margin = s::page::GROUP_GAP;
    let top = s::toolbar::HEIGHT + margin;
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
    Some((track_t + (track_h - len) * (scroll / max).clamp(0.0, 1.0), len))
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
    let t = s::toolbar::HEIGHT + s::scrollbar::MARGIN;
    (t, (window_h - t - s::scrollbar::MARGIN).max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrolling_keeps_things_in_view_and_the_thumb_tracks_it() {
        let (content, window) = (2000.0, 600.0);
        let max = max_scroll(content, window);
        assert_eq!(max, 1400.0);
        assert_eq!(max_scroll(300.0, 600.0), 0.0);
        assert_eq!(scroll_to_show(0.0, Rect::new(0.0, 200.0, 10.0, 260.0), window, max), 0.0);
        assert_eq!(scroll_to_show(0.0, Rect::new(0.0, 900.0, 10.0, 964.0), window, max), 964.0 - (window - s::page::GROUP_GAP));
        assert_eq!(scroll_to_show(500.0, Rect::new(0.0, 520.0, 10.0, 584.0), window, max), 520.0 - s::toolbar::HEIGHT - s::page::GROUP_GAP);
        let (top0, len) = scrollbar(0.0, content, window).unwrap();
        let (top1, _) = scrollbar(max, content, window).unwrap();
        assert_eq!(top0, s::toolbar::HEIGHT + s::scrollbar::MARGIN);
        assert!((top1 + len - (window - s::scrollbar::MARGIN)).abs() < 1e-3);
        assert!((scroll_for_thumb(top1, content, window) - max).abs() < 1e-3);
        assert_eq!(scroll_for_thumb(top0 - 50.0, content, window), 0.0);
        assert!(scrollbar(0.0, 300.0, window).is_none());
    }

    #[test]
    fn sidebar_sections_open_gaps_and_the_toolbar_lines_up() {
        let items = nav_items(&[0, 1, 1, 2]);
        assert_eq!(items[0].t, account().b + s::sidebar::SECTION_GAP);
        assert_eq!(items[1].t, items[0].b + s::sidebar::SECTION_GAP);
        assert_eq!(items[2].t, items[1].b);
        assert_eq!(items[3].t, items[2].b + s::sidebar::SECTION_GAP);
        assert!(search().b < account().t);
        let [min, close] = caption_buttons(1000.0);
        assert_eq!(close.r, 1000.0 - s::toolbar::INSET);
        assert_eq!(min.r, close.l - s::toolbar::BUTTON_GAP);
        let [back, forward] = history_buttons();
        assert_eq!(back.l, s::sidebar::WIDTH + s::toolbar::INSET);
        assert_eq!(forward.l, back.r + s::toolbar::BUTTON_GAP);
        assert_eq!(back.mid_y(), search().mid_y(), "toolbar and search field share a line");
        assert_eq!(column_width(10_000.0), s::page::MAX_WIDTH);
        assert_eq!(page_left(860.0), s::sidebar::WIDTH + s::page::PADDING_X);
        let wide = page_left(2000.0);
        assert_eq!(wide - s::sidebar::WIDTH, 2000.0 - wide - s::page::MAX_WIDTH);
    }
}
