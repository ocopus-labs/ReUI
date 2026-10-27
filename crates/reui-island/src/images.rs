//! Encoded image bytes as the hub sent them; the pill decodes the ones a snapshot shows (on
//! demand, see `Pill::drain_hub`).
//!
//! The hub sends each image once per connection and forgets it when a snapshot no longer
//! shows it, so eviction runs at the same point in the stream: on each snapshot, keeping
//! what it shows.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use reui_protocol::ImageId;

thread_local! {
    static STORE: RefCell<HashMap<ImageId, Rc<[u8]>>> = RefCell::new(HashMap::new());
}

pub fn put(id: ImageId, bytes: Vec<u8>) {
    STORE.with(|s| s.borrow_mut().insert(id, bytes.into()));
}

pub fn get(id: ImageId) -> Option<Rc<[u8]>> {
    STORE.with(|s| s.borrow().get(&id).cloned())
}

/// A snapshot arrived that shows `shown`: drop every image nobody can ask for any more.
pub fn evict(shown: &HashSet<ImageId>) {
    STORE.with(|s| s.borrow_mut().retain(|id, _| shown.contains(id)));
}

/// The hub forgot everything it sent (new connection).
pub fn clear() {
    STORE.with(|s| s.borrow_mut().clear());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eviction_keeps_what_the_snapshot_shows() {
        clear();
        put(ImageId(1), vec![1]);
        put(ImageId(2), vec![2]);
        evict(&HashSet::from([ImageId(2)]));
        assert!(get(ImageId(1)).is_none());
        assert_eq!(get(ImageId(2)).as_deref(), Some(&[2u8][..]));
    }
}
