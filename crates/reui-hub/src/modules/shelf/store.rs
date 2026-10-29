//! What the shelf holds, in `%LOCALAPPDATA%\ReUI\shelf\held.json`: references to the
//! user's files (paths, never copies), links and text, plus the files ReUI made itself
//! (`owned`: a browser's image, a file rescued from a temporary folder) under `files\`.
//! Written on every change, never on a timer. The rules (expiry, the cap on ReUI's own
//! copies) are pure, so they're tested without a disk.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::settings::Keep;

/// One card: what one drop held.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Held {
    pub id: u64,
    /// When it was dropped (Unix ms).
    pub added_ms: i64,
    #[serde(flatten)]
    pub what: What,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum What {
    /// Files and folders by full path. `owned`: ReUI made these copies (in its folder), so
    /// they count against the cap and go when the card goes; otherwise they're the user's
    /// and are never touched. `bytes` is the copies' size.
    Files {
        paths: Vec<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        owned: bool,
        #[serde(default, skip_serializing_if = "is_zero")]
        bytes: u64,
    },
    Link { url: String },
    Text { text: String },
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

impl What {
    fn owned_bytes(&self) -> Option<u64> {
        match self {
            What::Files { owned: true, bytes, .. } => Some(*bytes),
            _ => None,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Shelf {
    /// Oldest first.
    #[serde(default)]
    pub items: Vec<Held>,
    #[serde(default)]
    pub next_id: u64,
}

impl Shelf {
    /// Holds `what`, dropped at `now_ms`; returns its id.
    #[cfg(test)]
    pub fn add(&mut self, what: What, now_ms: i64) -> u64 {
        let id = self.reserve();
        self.insert(Held { id, added_ms: now_ms, what });
        id
    }

    /// An id for a card whose files are still being written (off the hub thread), so they
    /// can go straight into that card's own folder.
    pub fn reserve(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    pub fn insert(&mut self, held: Held) {
        self.next_id = self.next_id.max(held.id);
        self.items.push(held);
    }

    pub fn get(&self, id: u64) -> Option<&Held> {
        self.items.iter().find(|h| h.id == id)
    }

    pub fn remove(&mut self, id: u64) -> Option<Held> {
        let i = self.items.iter().position(|h| h.id == id)?;
        Some(self.items.remove(i))
    }

    /// Everything goes (Clear).
    pub fn clear(&mut self) -> Vec<Held> {
        std::mem::take(&mut self.items)
    }

    /// What `keep` says has had its time at `now_ms`, removed and returned. A clock set back
    /// never makes something expire.
    pub fn expire(&mut self, keep: Keep, now_ms: i64) -> Vec<Held> {
        let Some(max) = keep.max_age_ms() else { return Vec::new() };
        let (gone, kept) = std::mem::take(&mut self.items).into_iter().partition(|h| now_ms - h.added_ms > max);
        self.items = kept;
        gone
    }

    /// ReUI's own copies beyond `cap` bytes, oldest first, removed and returned. References
    /// cost nothing and are never removed for space.
    pub fn over_cap(&mut self, cap: u64) -> Vec<Held> {
        let mut total: u64 = self.items.iter().filter_map(|h| h.what.owned_bytes()).sum();
        let mut gone = Vec::new();
        while total > cap {
            let Some(i) = self.items.iter().position(|h| h.what.owned_bytes().is_some()) else { break };
            let held = self.items.remove(i);
            total -= held.what.owned_bytes().unwrap_or(0);
            gone.push(held);
        }
        gone
    }
}

/// `%LOCALAPPDATA%\ReUI\shelf`.
pub fn folder() -> PathBuf {
    crate::paths::local().join("shelf")
}

/// Where card `id`'s own copies go: a folder per card, so names never collide and the
/// names the user sees are the ones the files had.
pub fn files_dir(root: &Path, id: u64) -> PathBuf {
    root.join("files").join(id.to_string())
}

pub fn load(root: &Path) -> Shelf {
    let path = root.join("held.json");
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(text.trim_start_matches('\u{feff}')).unwrap_or_else(|e| {
            eprintln!("shelf: {} isn't readable ({e}); starting empty", path.display());
            Shelf::default()
        }),
        Err(_) => Shelf::default(),
    }
}

/// Written whole to a temporary file and renamed over the old one, so a crash mid-write
/// never leaves half a list.
pub fn save(root: &Path, shelf: &Shelf) {
    _ = std::fs::create_dir_all(root);
    let path = root.join("held.json");
    let tmp = root.join("held.json.tmp");
    let text = serde_json::to_string_pretty(shelf).unwrap_or_default();
    if let Err(e) = std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, &path)) {
        eprintln!("shelf: can't write {}: {e}", path.display());
    }
}

/// Deletes what ReUI owned of removed cards (their folders); the user's files are never
/// touched.
pub fn forget(root: &Path, gone: &[Held]) {
    for held in gone {
        if matches!(held.what, What::Files { owned: true, .. }) {
            _ = std::fs::remove_dir_all(files_dir(root, held.id));
        }
    }
}

/// Deletes card folders under `files\` that no held card owns (a crash between writing a
/// drop's copies and saving the list, a hand-edited list), so ReUI's folder never keeps
/// what nobody can see.
pub fn sweep(root: &Path, shelf: &Shelf) {
    let Ok(dirs) = std::fs::read_dir(root.join("files")) else { return };
    for dir in dirs.flatten() {
        let owned = dir.file_name().to_str().and_then(|n| n.parse::<u64>().ok()).is_some_and(|id| shelf.get(id).is_some_and(|h| matches!(h.what, What::Files { owned: true, .. })));
        if !owned {
            _ = std::fs::remove_dir_all(dir.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(paths: &[&str]) -> What {
        What::Files { paths: paths.iter().map(|p| p.to_string()).collect(), owned: false, bytes: 0 }
    }

    fn owned(bytes: u64) -> What {
        What::Files { paths: vec!["x.png".into()], owned: true, bytes }
    }

    #[test]
    fn cards_get_ids_and_go() {
        let mut shelf = Shelf::default();
        let a = shelf.add(files(&[r"C:\a.pdf"]), 0);
        let b = shelf.add(What::Text { text: "hi".into() }, 1);
        assert_eq!((a, b), (1, 2));
        assert_eq!(shelf.remove(a).map(|h| h.id), Some(1));
        assert!(shelf.remove(a).is_none());
        assert_eq!(shelf.add(What::Link { url: "https://x.dev".into() }, 2), 3, "ids are never reused");
        assert_eq!(shelf.clear().len(), 2);
        assert!(shelf.items.is_empty());
    }

    #[test]
    fn things_expire_as_the_user_chose() {
        let day = 86_400_000;
        let mut shelf = Shelf::default();
        shelf.add(files(&["old"]), 0);
        shelf.add(files(&["new"]), 3 * day);
        assert!(shelf.expire(Keep::Removed, 30 * day).is_empty());
        assert!(shelf.expire(Keep::Restart, 30 * day).is_empty(), "restart is handled at start");
        let gone = shelf.expire(Keep::Day, 3 * day + 10);
        assert_eq!(gone.len(), 1);
        assert_eq!(shelf.items.len(), 1);
        assert!(shelf.expire(Keep::Week, -5 * day).is_empty(), "a clock set back expires nothing");
    }

    #[test]
    fn only_reuis_own_copies_count_against_the_cap() {
        let mut shelf = Shelf::default();
        shelf.add(owned(300), 0);
        shelf.add(files(&[r"D:\huge.iso"]), 1);
        shelf.add(owned(300), 2);
        shelf.add(owned(300), 3);
        let gone = shelf.over_cap(700);
        assert_eq!(gone.iter().map(|h| h.id).collect::<Vec<_>>(), [1], "the oldest copy goes first");
        assert_eq!(shelf.items.len(), 3);
        assert_eq!(shelf.over_cap(0).len(), 2);
        assert!(matches!(&shelf.items[..], [Held { what: What::Files { owned: false, .. }, .. }]), "a reference stays");
    }

    #[test]
    fn the_store_round_trips_and_reads_old_files() {
        let mut shelf = Shelf::default();
        shelf.add(files(&[r"C:\a.pdf", r"C:\b.pdf"]), 5);
        shelf.add(owned(12), 6);
        shelf.add(What::Link { url: "https://x.dev".into() }, 7);
        let text = serde_json::to_string(&shelf).unwrap();
        assert!(text.contains(r#""kind":"files""#) && !text.contains(r#""owned":false"#));
        assert_eq!(serde_json::from_str::<Shelf>(&text).unwrap(), shelf);
        let minimal: Shelf = serde_json::from_str(r#"{ "items": [{ "id": 4, "added_ms": 1, "kind": "text", "text": "hi" }] }"#).unwrap();
        assert_eq!(minimal.items[0].what, What::Text { text: "hi".into() });
    }

    #[test]
    fn saving_and_forgetting_touch_only_reuis_folder() {
        let root = std::env::temp_dir().join(format!("reui-shelf-store-{}", std::process::id()));
        let mut shelf = Shelf::default();
        let id = shelf.add(owned(1), 0);
        let dir = files_dir(&root, id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("x.png"), b"x").unwrap();
        save(&root, &shelf);
        assert_eq!(load(&root), shelf);
        let stray = files_dir(&root, 99);
        std::fs::create_dir_all(&stray).unwrap();
        sweep(&root, &shelf);
        assert!(dir.exists() && !stray.exists(), "only what no card owns goes");
        let gone = shelf.clear();
        forget(&root, &gone);
        assert!(!dir.exists());
        _ = std::fs::remove_dir_all(&root);
    }
}
