//! Encoded image bytes as the hub sent them, for both consumers: the pill decodes the ones
//! a snapshot shows (on demand, see `Pill::drain_hub`), the clipboard window serves them to
//! its page as `https://reui.local/image/<id>`.
//!
//! The hub sends each image once per connection and forgets it only when a snapshot neither
//! shows it nor names it in a window's latest message, so eviction runs at the same point
//! in the stream: on each snapshot, keeping what it shows plus the window's images. The
//! window side keeps a little more than the hub assumes (the latest view *and* the latest
//! preview), which only costs memory, never a missing image.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use reui_protocol::ImageId;

#[derive(Default)]
struct Store {
    bytes: HashMap<ImageId, Rc<[u8]>>,
    /// Images of the latest window message of any kind (what the hub keeps as sent).
    latest: Vec<ImageId>,
    /// Images of the latest `open`/`state` and of the latest `preview`: the page may still
    /// (re)load these while it shows them.
    view: Vec<ImageId>,
    preview: Vec<ImageId>,
}

thread_local! {
    static STORE: RefCell<Store> = RefCell::new(Store::default());
}

pub fn put(id: ImageId, bytes: Vec<u8>) {
    STORE.with(|s| s.borrow_mut().bytes.insert(id, bytes.into()));
}

pub fn get(id: ImageId) -> Option<Rc<[u8]>> {
    STORE.with(|s| s.borrow().bytes.get(&id).cloned())
}

/// A window message arrived; `kind` is its `"type"`.
pub fn window_message(kind: &str, images: &[ImageId]) {
    STORE.with(|s| {
        let mut s = s.borrow_mut();
        s.latest = images.to_vec();
        match kind {
            "open" | "state" => s.view = images.to_vec(),
            "preview" => s.preview = images.to_vec(),
            _ => {}
        }
    });
}

/// A snapshot arrived that shows `shown`: drop every image nobody can ask for any more.
pub fn evict(shown: &HashSet<ImageId>) {
    STORE.with(|s| {
        let mut s = s.borrow_mut();
        let keep = keep_set(shown, &s.latest, &s.view, &s.preview);
        s.bytes.retain(|id, _| keep.contains(id));
    });
}

/// The hub forgot everything it sent (new connection).
pub fn clear() {
    STORE.with(|s| *s.borrow_mut() = Store::default());
}

fn keep_set(shown: &HashSet<ImageId>, latest: &[ImageId], view: &[ImageId], preview: &[ImageId]) -> HashSet<ImageId> {
    shown.iter().chain(latest).chain(view).chain(preview).copied().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eviction_keeps_snapshot_and_window_images() {
        clear();
        for i in 1..=5 {
            put(ImageId(i), vec![i as u8]);
        }
        window_message("state", &[ImageId(2)]);
        window_message("preview", &[ImageId(3)]);
        window_message("export", &[ImageId(4)]);
        evict(&HashSet::from([ImageId(1)]));
        assert!(get(ImageId(1)).is_some(), "shown by the snapshot");
        assert!(get(ImageId(2)).is_some(), "the page's current view");
        assert!(get(ImageId(3)).is_some(), "the page's current preview");
        assert!(get(ImageId(4)).is_some(), "the latest message (the hub keeps it as sent)");
        assert!(get(ImageId(5)).is_none());
        // A new view replaces the old one; the preview stays until replaced.
        window_message("state", &[ImageId(1)]);
        evict(&HashSet::new());
        assert!(get(ImageId(2)).is_none());
        assert!(get(ImageId(3)).is_some());
        clear();
    }
}
