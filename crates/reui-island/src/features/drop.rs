//! The file shelf on the island (docs/PRD-file-shelf.md §5.1, §5.4), the rules only: when
//! a drag passing by is near enough for the island to stop being click-through for it,
//! which drop effect to answer with (never Move), what a drop's formats become, and what
//! dragging the compact pill carries. Pure; the OLE side is `dragdrop/`.

use reui_protocol::{Drag, Dropped, Item};

/// `DROPEFFECT` bits, as OLE has them.
pub const COPY: u32 = 1;
#[cfg(test)]
pub const MOVE: u32 = 2;
pub const LINK: u32 = 4;

/// The most content with no file (a browser's image) one drop may carry: a pipe frame is
/// 16 MB, and the rest of the message needs room.
pub const DATA_MAX: usize = 12 * 1024 * 1024;

/// Whether screen point (`x`, `y`) is within `reach` pixels of `rect` (left, top, right,
/// bottom): a drag there means the island.
pub fn near((l, t, r, b): (f32, f32, f32, f32), reach: f32, x: f32, y: f32) -> bool {
    x >= l - reach && x < r + reach && y >= t - reach && y < b + reach
}

/// The effect to answer a drag with, of those its source `allowed`: Copy, else Link. Never
/// Move: a source told Move may delete its original once the drop is done, and the shelf
/// only ever refers to it.
pub fn effect(allowed: u32) -> u32 {
    if allowed & COPY != 0 {
        COPY
    } else if allowed & LINK != 0 {
        LINK
    } else {
        0
    }
}

/// The effects ReUI offers when something is dragged out of it: the same two.
pub const OFFERED: u32 = COPY | LINK;

/// What a drag offers, as read off its data object (in this order of preference).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Offer {
    /// `CF_HDROP`: real files and folders.
    pub files: Vec<String>,
    /// `FileGroupDescriptorW` + `FileContents`: files with no path (a browser's image, a
    /// mail's attachment), name and bytes (`None` when they couldn't be read or are too big).
    pub virtual_files: Vec<(String, Option<Vec<u8>>)>,
    /// `UniformResourceLocatorW`.
    pub url: Option<String>,
    /// `PNG`: an image with no file.
    pub png: Option<Vec<u8>>,
    /// `CF_UNICODETEXT`.
    pub text: Option<String>,
}

/// A link dragged from a browser comes with a `.url` "file" too; that's the link, not a file.
pub fn is_shortcut(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".url")
}

/// What to send the hub for a drop: real files if there are any (alone: whatever else
/// comes with them describes them), else the virtual files that aren't shortcuts, else
/// the link, else an image, else the text.
pub fn dropped(offer: Offer) -> Vec<Dropped> {
    if !offer.files.is_empty() {
        return vec![Dropped::Files(offer.files)];
    }
    let mut room = DATA_MAX;
    let data: Vec<Dropped> = offer
        .virtual_files
        .into_iter()
        .filter(|(name, _)| !is_shortcut(name))
        .filter_map(|(name, bytes)| {
            let bytes = bytes.filter(|b| !b.is_empty() && b.len() <= room)?;
            room -= bytes.len();
            Some(Dropped::Data { name, bytes })
        })
        .collect();
    if !data.is_empty() {
        return data;
    }
    if let Some(url) = offer.url.filter(|u| !u.trim().is_empty()) {
        return vec![Dropped::Link(url.trim().to_string())];
    }
    if let Some(png) = offer.png.filter(|p| !p.is_empty() && p.len() <= DATA_MAX) {
        return vec![Dropped::Data { name: "Image.png".into(), bytes: png }];
    }
    offer.text.filter(|t| !t.trim().is_empty()).map(Dropped::Text).into_iter().collect()
}

/// Dragging the compact pill carries every file its card holds (S2: several files from
/// several folders into one upload). `None` when the card holds no files to drag.
pub fn drag_all(items: &[Item]) -> Option<Drag> {
    let files: Vec<String> = items
        .iter()
        .filter_map(|i| match &i.drag {
            Some(Drag::Files(paths)) => Some(paths.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    (!files.is_empty()).then_some(Drag::Files(files))
}

/// A press that has moved this far (screen pixels, the system's drag rectangle) is a drag.
pub fn moved_past(from: (i32, i32), to: (i32, i32), (cx, cy): (i32, i32)) -> bool {
    (to.0 - from.0).abs() > cx / 2 || (to.1 - from.1).abs() > cy / 2
}

#[cfg(test)]
mod tests {
    use super::*;
    use reui_protocol::Preview;

    #[test]
    fn a_drag_counts_only_near_the_island() {
        let pill = (100.0, 10.0, 300.0, 50.0);
        assert!(near(pill, 24.0, 90.0, 60.0));
        assert!(near(pill, 24.0, 200.0, 0.0), "the band above it, at the screen's edge");
        assert!(!near(pill, 24.0, 200.0, 80.0), "a page under the island's empty area keeps the drop");
        assert!(!near(pill, 24.0, 330.0, 30.0));
    }

    #[test]
    fn never_move() {
        assert_eq!(effect(COPY | MOVE | LINK), COPY);
        assert_eq!(effect(MOVE | LINK), LINK);
        assert_eq!(effect(MOVE), 0, "a source that only moves keeps its file");
        assert_eq!(OFFERED & MOVE, 0);
    }

    #[test]
    fn real_files_win_then_content_then_the_link() {
        let files = Offer { files: vec![r"C:\a.pdf".into()], url: Some("https://x.dev".into()), text: Some("a.pdf".into()), ..Default::default() };
        assert_eq!(dropped(files), [Dropped::Files(vec![r"C:\a.pdf".into()])]);
        // A browser image: the picture as a file, not its address.
        let image = Offer { virtual_files: vec![("cat.png".into(), Some(vec![1, 2]))], url: Some("https://x.dev/cat.png".into()), ..Default::default() };
        assert_eq!(dropped(image), [Dropped::Data { name: "cat.png".into(), bytes: vec![1, 2] }]);
        // A browser link: its .url shortcut is the link.
        let link = Offer { virtual_files: vec![("Example.url".into(), Some(vec![9]))], url: Some(" https://example.com ".into()), text: Some("https://example.com".into()), ..Default::default() };
        assert_eq!(dropped(link), [Dropped::Link("https://example.com".into())]);
        assert_eq!(dropped(Offer { png: Some(vec![5]), text: Some("x".into()), ..Default::default() }), [Dropped::Data { name: "Image.png".into(), bytes: vec![5] }]);
        assert_eq!(dropped(Offer { text: Some("hello".into()), ..Default::default() }), [Dropped::Text("hello".into())]);
        assert!(dropped(Offer { text: Some("  ".into()), ..Default::default() }).is_empty());
    }

    #[test]
    fn content_too_big_for_one_message_is_left_out() {
        let big = Offer { virtual_files: vec![("a.bin".into(), Some(vec![0; DATA_MAX - 10])), ("b.bin".into(), Some(vec![0; 20])), ("c.bin".into(), Some(vec![0; 5])), ("d.bin".into(), None)], ..Default::default() };
        let names: Vec<String> = dropped(big).into_iter().map(|d| if let Dropped::Data { name, .. } = d { name } else { String::new() }).collect();
        assert_eq!(names, ["a.bin", "c.bin"]);
    }

    #[test]
    fn the_pill_drags_every_file_held() {
        let item = |drag| Item { action: String::new(), preview: Preview::Text(String::new()), icon: None, age: String::new(), meta: None, pinned: false, actions: vec![], drag };
        let items = [item(Some(Drag::Files(vec!["a".into()]))), item(Some(Drag::Text("t".into()))), item(None), item(Some(Drag::Files(vec!["b".into(), "c".into()])))];
        assert_eq!(drag_all(&items), Some(Drag::Files(vec!["a".into(), "b".into(), "c".into()])));
        assert_eq!(drag_all(&items[1..3]), None);
    }

    #[test]
    fn a_press_becomes_a_drag_past_the_system_distance() {
        assert!(!moved_past((10, 10), (12, 13), (8, 8)));
        assert!(moved_past((10, 10), (15, 10), (8, 8)));
    }
}
