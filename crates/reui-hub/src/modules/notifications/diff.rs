//! What changed between two lists of the notification centre, by notification id. Pure.
//!
//! Windows gives no "new toast" event without package identity, so every list is compared
//! with the one before. Only ids are compared: reading a toast's app and text costs a
//! cross-process call each (measured ~8–12 ms per toast), so those are read only for the
//! ids this reports as new.

use std::collections::BTreeSet;

/// The ids seen in the last list; `None` until the first list, which only sets the
/// baseline (the toasts already in the notification centre when ReUI starts aren't news).
#[derive(Debug, Default)]
pub struct Seen {
    ids: Option<BTreeSet<u32>>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Change {
    /// New since the last list, oldest first (ids grow as toasts arrive).
    pub added: Vec<u32>,
    /// Gone since the last list: dismissed, expired, or opened from the notification centre.
    pub removed: Vec<u32>,
}

impl Seen {
    pub fn update(&mut self, current: &[u32]) -> Change {
        let now: BTreeSet<u32> = current.iter().copied().collect();
        let Some(before) = self.ids.replace(now.clone()) else { return Change::default() };
        Change { added: now.difference(&before).copied().collect(), removed: before.difference(&now).copied().collect() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notifications_first_list_is_only_the_baseline() {
        let mut seen = Seen::default();
        assert_eq!(seen.update(&[4, 9, 2]), Change::default(), "already there at start: not news");
        assert_eq!(seen.update(&[4, 9, 2]), Change::default());
        assert_eq!(seen.update(&[4, 9, 2, 10]).added, vec![10]);
    }

    #[test]
    fn notifications_new_and_gone_ids() {
        let mut seen = Seen::default();
        seen.update(&[1, 2, 3]);
        assert_eq!(seen.update(&[2, 3, 7, 5]), Change { added: vec![5, 7], removed: vec![1] });
        assert_eq!(seen.update(&[]), Change { added: vec![], removed: vec![2, 3, 5, 7] });
        assert_eq!(seen.update(&[8]), Change { added: vec![8], removed: vec![] });
    }
}
