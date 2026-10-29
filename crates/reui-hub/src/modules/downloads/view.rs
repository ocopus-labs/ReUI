//! What the downloads module puts in the island: the downloads in progress (one card, a
//! pager when there are several) and the Downloaded card (the same, for finished ones).
//! Meaning only: the size so far, the speed, a ring and bar only when the total is known.

use std::path::Path;

use reui_protocol::{Art, Button, ButtonContent, ButtonRole, CardSize, CompactView, Element, ExpandedView, Headline, ImageId, Pager, Rgb, Row, Tone};

use super::tracker::{Done, Download};
use crate::hub::View;
use crate::modules::stats::maths::{bytes, bytes_per_second};

/// System green: a download arriving is good news.
pub const ACCENT: Rgb = Rgb(48, 209, 88);
const GLYPH_DOWNLOAD: char = '\u{E896}';
const GLYPH_DONE: char = '\u{E73E}';

/// Button actions, followed by the download's id.
pub const OPEN: &str = "open:";
pub const SHOW: &str = "show:";
/// The pager's.
pub const PREV: &str = "prev";
pub const NEXT: &str = "next";

fn text_button(action: String, label: &str, role: ButtonRole) -> Button {
    Button { action, content: ButtonContent::Text(label.into()), role }
}

fn pager(index: usize, count: usize) -> Option<Pager> {
    (count > 1).then(|| Pager { index: index as u32, count: count as u32, prev: PREV.into(), next: NEXT.into() })
}

/// What the download is called: its final name, or (while Chrome hasn't settled it) just
/// that it's a download.
fn name(d: &Download) -> String {
    d.name.clone().unwrap_or_else(|| "Download".into())
}

/// "48 MB · 3.1 MB/s", "48 MB of 120 MB · 3.1 MB/s", "48 MB" (stalled).
pub fn progress_line(d: &Download, now_ms: u64) -> String {
    let size = match d.total {
        Some(total) => format!("{} of {}", bytes(d.size), bytes(total)),
        None => bytes(d.size),
    };
    match d.speed(now_ms) {
        Some(speed) => format!("{size} · {}", bytes_per_second(speed)),
        None => size,
    }
}

/// The downloads in progress (`list`, newest first), showing the one at `index`.
pub fn active(list: &[&Download], index: usize, now_ms: u64) -> View {
    let index = index.min(list.len().saturating_sub(1));
    let Some(d) = list.get(index) else { return View::default() };
    let mark = match d.fraction() {
        Some(f) => Element::Ring(f),
        None => Element::Art(Art::Glyph(GLYPH_DOWNLOAD)),
    };
    let mut rows = Vec::new();
    if let Some(fraction) = d.fraction() {
        rows.push(Row::Progress { fraction, leading: None, trailing: Some(format!("{} %", (fraction * 100.0).floor() as u32)) });
    }
    rows.push(Row::Buttons(vec![text_button(format!("{SHOW}{}", d.id), "Show in folder", ButtonRole::Secondary)]));
    View {
        accent: Some(ACCENT),
        audio: None,
        // The size first: when the pill cuts the line, the name's end goes, not the number.
        compact: CompactView { leading: mark.clone(), trailing: Element::Text { text: format!("{} · {}", bytes(d.size), name(d)), tone: Tone::Secondary } },
        minimal: mark.clone(),
        expanded: Some(ExpandedView {
            context: None,
            leading: mark,
            eyebrow: Some("Downloading".into()),
            headline: Headline::Text(name(d)),
            detail: Some(progress_line(d, now_ms)),
            trailing: Element::Empty,
            rows,
            size: CardSize::Regular,
            tint: false,
            pager: pager(index, list.len()),
            open: None,
        }),
    }
}

/// The folder a file is in, by its own name ("Downloads").
fn folder_name(path: &Path) -> Option<String> {
    Some(path.parent()?.file_name()?.to_string_lossy().into_owned())
}

/// Finished downloads (`list`, newest first), showing the one at `index` with its file
/// icon (`icon`, once it's been read).
pub fn done(list: &[&Done], index: usize, icon: Option<ImageId>) -> View {
    let index = index.min(list.len().saturating_sub(1));
    let Some(d) = list.get(index) else { return View::default() };
    let mark = Element::Art(icon.map_or(Art::Glyph(GLYPH_DONE), Art::Image));
    let detail = [Some(bytes(d.size)), folder_name(&d.path)].into_iter().flatten().collect::<Vec<_>>().join(" · ");
    let buttons = vec![text_button(format!("{SHOW}{}", d.id), "Show in folder", ButtonRole::Secondary), text_button(format!("{OPEN}{}", d.id), "Open", ButtonRole::Accent)];
    View {
        accent: Some(ACCENT),
        audio: None,
        compact: CompactView { leading: mark.clone(), trailing: Element::Text { text: d.name(), tone: Tone::Secondary } },
        minimal: mark.clone(),
        expanded: Some(ExpandedView {
            context: None,
            leading: mark,
            eyebrow: Some("Downloaded".into()),
            headline: Headline::Text(d.name()),
            detail: Some(detail),
            trailing: Element::Empty,
            rows: vec![Row::Buttons(buttons)],
            size: CardSize::Regular,
            tint: false,
            pager: pager(index, list.len()),
            open: Some(format!("{OPEN}{}", d.id)),
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::modules::downloads::tracker::Tracker;
    use crate::modules::folders::Change;

    fn tracker() -> Tracker {
        let dir = PathBuf::from(r"C:\Users\me\Downloads");
        let mut t = Tracker::default();
        for (at, size) in [(0, 0), (1_000, 2_000_000), (2_000, 48_200_000)] {
            t.on_change(&Change::Modified(dir.join("setup.exe.crdownload")), at);
            let paths = t.due_reads(at).unwrap();
            t.on_sizes(&paths.into_iter().map(|p| (p, Some(size))).collect::<Vec<_>>(), at);
        }
        t.on_change(&Change::Modified(dir.join("Unconfirmed 1.crdownload")), 2_000);
        t
    }

    #[test]
    fn a_download_in_progress_says_its_size_and_speed_and_never_a_made_up_percentage() {
        let t = tracker();
        let showing = t.showing(3_100);
        assert_eq!(showing.len(), 2);
        let view = active(&showing, 1, 2_000);
        assert_eq!(view.compact.leading, Element::Art(Art::Glyph(GLYPH_DOWNLOAD)), "no ring without a total");
        assert_eq!(view.compact.trailing, Element::Text { text: "48 MB · setup.exe".into(), tone: Tone::Secondary });
        let card = view.expanded.unwrap();
        assert_eq!(card.headline, Headline::Text("setup.exe".into()));
        assert_eq!(card.detail.as_deref(), Some("48 MB · 24 MB/s"));
        assert!(!card.rows.iter().any(|r| matches!(r, Row::Progress { .. })));
        assert_eq!(card.pager.map(|p| (p.index, p.count)), Some((1, 2)));
        let unnamed = active(&showing, 0, 2_000).expanded.unwrap();
        assert_eq!(unnamed.headline, Headline::Text("Download".into()));
        assert_eq!(active(&showing, 9, 2_000).expanded.unwrap().pager.map(|p| p.index), Some(1), "a stale index stays in range");
        assert_eq!(active(&[], 0, 0).expanded, None);
    }

    #[test]
    fn a_known_total_adds_a_ring_and_a_bar() {
        let t = tracker();
        let mut d = t.showing(3_100)[1].clone();
        d.total = Some(120_000_000);
        let view = active(&[&d], 0, 9_000);
        assert_eq!(view.compact.leading, Element::Ring(d.fraction().unwrap()));
        let card = view.expanded.unwrap();
        assert_eq!(card.detail.as_deref(), Some("48 MB of 120 MB"), "stalled: no speed");
        assert!(matches!(&card.rows[0], Row::Progress { trailing: Some(p), .. } if p == "40 %"));
    }

    #[test]
    fn a_finished_download_offers_open_and_its_folder() {
        let mut t = tracker();
        let dir = PathBuf::from(r"C:\Users\me\Downloads");
        t.on_change(&Change::Renamed { from: dir.join("setup.exe.crdownload"), to: dir.join("setup.exe") }, 3_000);
        let list = t.done();
        let view = done(&list, 0, None);
        assert_eq!(view.compact.trailing, Element::Text { text: "setup.exe".into(), tone: Tone::Secondary });
        let card = view.expanded.unwrap();
        assert_eq!(card.eyebrow.as_deref(), Some("Downloaded"));
        assert_eq!(card.detail.as_deref(), Some("48 MB · Downloads"));
        let id = list[0].id;
        assert_eq!(card.rows, vec![Row::Buttons(vec![text_button(format!("show:{id}"), "Show in folder", ButtonRole::Secondary), text_button(format!("open:{id}"), "Open", ButtonRole::Accent)])]);
        assert_eq!(done(&list, 0, Some(ImageId(3))).compact.leading, Element::Art(Art::Image(ImageId(3))));
    }
}
