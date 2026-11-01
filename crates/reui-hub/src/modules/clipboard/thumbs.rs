//! Files' pictures for their cards (`docs/PRD-clipboard-panel-v3.md` §3): what Explorer
//! shows for each of a clip's first files (a picture, a PDF's first page, else the type's
//! icon), made on the blocking pool for the first cards of an open panel only, and kept by
//! path while the hub runs so reopening the panel costs nothing.

use std::collections::{HashMap, HashSet};

use super::os::file_thumbnail;
use super::store::Saved;

/// Pictures on one card: the design fans out three.
pub const PER_CARD: usize = 3;
/// Cards whose files get pictures: what's in view and a scroll or two past it.
const LOOKAHEAD: usize = 12;
/// Pixels on the picture's long side: a card's body at 200 %.
const PX: u32 = 192;
/// Paths remembered; past this the oldest batch is forgotten (they're made again if shown).
const KEEP: usize = 256;

/// A batch of made pictures: each path and its PNG (`None`: the shell had nothing).
pub type Made = Vec<(String, Option<Vec<u8>>)>;

#[derive(Default)]
pub struct Thumbs {
    made: HashMap<String, Option<Vec<u8>>>,
    pending: HashSet<String>,
}

impl Thumbs {
    /// Paths among the first cards that have no picture yet and aren't being made, now
    /// marked as being made. Files that are gone have nothing to show.
    pub fn wanted(&mut self, items: &[Saved]) -> Vec<String> {
        let mut wanted = Vec::new();
        for path in shown(items) {
            if !self.made.contains_key(path) && self.pending.insert(path.clone()) {
                wanted.push(path.clone());
            }
        }
        wanted
    }

    /// Takes a batch the pool made.
    pub fn made(&mut self, batch: Made) {
        if self.made.len() + batch.len() > KEEP {
            self.made.clear();
        }
        for (path, png) in batch {
            self.pending.remove(&path);
            self.made.insert(path, png);
        }
    }

    /// The pictures there are for these cards, by path.
    pub fn for_items(&self, items: &[Saved]) -> HashMap<String, Vec<u8>> {
        shown(items).filter_map(|p| Some((p.clone(), self.made.get(p)?.clone()?))).collect()
    }
}

/// The first files of the first cards holding files.
fn shown(items: &[Saved]) -> impl Iterator<Item = &String> {
    items.iter().take(LOOKAHEAD).filter(|s| !s.files.is_empty() && !s.missing).flat_map(|s| s.files.iter().take(PER_CARD))
}

/// Makes `paths`' pictures on the blocking pool (the shell may open each file).
pub async fn make(paths: Vec<String>) -> Made {
    tokio::task::spawn_blocking(move || paths.into_iter().map(|p| {
        let png = file_thumbnail(&p, PX);
        (p, png)
    }).collect()).await.unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(paths: &[&str]) -> Saved {
        Saved { category: "files".into(), files: paths.iter().map(|p| p.to_string()).collect(), ..Default::default() }
    }

    #[test]
    fn pictures_are_made_once_for_the_first_files_of_the_first_cards() {
        let mut t = Thumbs::default();
        let items = [files(&["a", "b", "c", "d"]), Saved { category: "text".into(), ..Default::default() }, Saved { missing: true, ..files(&["gone"]) }];
        // Three per card, none for text or files that are gone.
        assert_eq!(t.wanted(&items), ["a", "b", "c"]);
        // Already being made: not asked again.
        assert!(t.wanted(&items).is_empty());
        t.made(vec![("a".into(), Some(vec![1])), ("b".into(), None), ("c".into(), Some(vec![3]))]);
        assert!(t.wanted(&items).is_empty());
        let shown = t.for_items(&items);
        assert_eq!((shown.get("a"), shown.get("b"), shown.get("c")), (Some(&vec![1]), None, Some(&vec![3])));
    }

    #[test]
    fn only_the_first_cards_get_pictures() {
        let mut t = Thumbs::default();
        let items: Vec<Saved> = (0..20).map(|i| files(&[&format!("f{i}")])).collect();
        assert_eq!(t.wanted(&items).len(), LOOKAHEAD);
    }

    #[test]
    fn the_cache_forgets_when_full() {
        let mut t = Thumbs::default();
        t.made((0..KEEP).map(|i| (format!("p{i}"), None)).collect());
        t.made(vec![("new".into(), Some(vec![9]))]);
        assert_eq!(t.made.len(), 1);
    }
}

#[cfg(test)]
mod shell {
    /// Asks the real shell (needs a desktop session): a picture gets its thumbnail, a text
    /// file its type's icon, a path that isn't there nothing.
    #[test]
    #[ignore = "uses the Windows shell: cargo test -p reui-hub files_get_pictures_from_the_shell -- --ignored"]
    fn files_get_pictures_from_the_shell() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for file in ["docs/images/hero.png", "Cargo.toml"] {
            let path = root.join(file).canonicalize().unwrap();
            let png = super::file_thumbnail(&path.to_string_lossy(), super::PX).unwrap_or_else(|| panic!("no picture for {file}"));
            assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "{file}");
            let size = image_size(&png);
            println!("{file}: {} bytes, {size:?}", png.len());
            assert!(size.0 <= super::PX && size.1 <= super::PX && size.0.max(size.1) >= 32, "{file}: {size:?}");
        }
        assert!(super::file_thumbnail(&root.join("no such file.pdf").to_string_lossy(), super::PX).is_none());
    }

    fn image_size(png: &[u8]) -> (u32, u32) {
        let be = |b: &[u8]| u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
        (be(&png[16..20]), be(&png[20..24]))
    }
}
