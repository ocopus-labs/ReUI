//! The sidebar's search: which pages hold the words typed (their titles, section headers,
//! row titles and footnotes), and the first matching row on a page, to scroll it into view.

use crate::pages::{self, Page};
use crate::state::State;
use crate::ui::layout::{Layout, Rect, Shape};
use crate::ui::Block;

/// The words a page can be found by, lower-cased.
fn words(page: Page, blocks: &[Block]) -> String {
    let mut out = page.title().to_lowercase();
    for block in blocks {
        match block {
            Block::Group { label, rows, footer } => {
                for text in label.iter().chain(footer.iter()).chain(rows.iter().map(|r| &r.title)) {
                    out.push('\n');
                    out.push_str(&text.to_lowercase());
                }
            }
            Block::Text { label: Some(label), title, .. } => {
                out.push('\n');
                out.push_str(&label.to_lowercase());
                out.push('\n');
                out.push_str(&title.to_lowercase());
            }
            Block::Figures { label: Some(label), .. } => {
                out.push('\n');
                out.push_str(&label.to_lowercase());
            }
            _ => {}
        }
    }
    out
}

/// The pages of `nav` that match `query` (all of them when it's blank).
pub fn filter(nav: &[Page], query: &str, state: &State) -> Vec<Page> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return nav.to_vec();
    }
    nav.iter().copied().filter(|&page| words(page, &pages::build(page, state)).contains(&q)).collect()
}

/// The first row on the page whose title or footnote holds `query` (page coordinates).
pub fn first_match(blocks: &[Block], layout: &Layout, query: &str) -> Option<Rect> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return None;
    }
    layout.shapes.iter().find_map(|shape| match shape {
        Shape::Row { block, row, rect, .. } => match &blocks[*block] {
            Block::Group { rows, .. } if rows[*row].title.to_lowercase().contains(&q) => Some(*rect),
            _ => None,
        },
        Shape::Footer { block, rect } => match &blocks[*block] {
            Block::Group { footer: Some(text), .. } if text.to_lowercase().contains(&q) => Some(*rect),
            _ => None,
        },
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::tests::state;

    #[test]
    fn pages_are_found_by_their_rows() {
        let (state, dir) = state("search");
        assert_eq!(filter(&Page::ALL, "", &state).len(), Page::ALL.len());
        // Titles, rows and footnotes all count: General's footnote mentions the clipboard.
        assert_eq!(filter(&Page::ALL, "clip", &state)[..2], [Page::General, Page::Clipboard]);
        let lyrics = filter(&Page::ALL, "LYRICS", &state);
        assert!(lyrics.contains(&Page::NowPlaying) && lyrics.contains(&Page::Privacy));
        assert!(filter(&Page::ALL, "airpods", &state) == [Page::Devices]);
        assert!(filter(&Page::ALL, "zzzz", &state).is_empty());
        _ = std::fs::remove_dir_all(&dir);
    }
}
