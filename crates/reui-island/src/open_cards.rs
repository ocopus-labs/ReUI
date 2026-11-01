//! Which cards are open across the islands (docs/PRD-all-displays.md §5.4). A module
//! hears `expanded` when the first island opens its card and `collapsed` when the last one
//! closes it, however many displays show the card. With one island this is exactly the
//! island's own open and close.

use std::cell::RefCell;
use std::collections::HashMap;

use reui_protocol::ActivityId;

/// Open counts per activity. Pure.
#[derive(Default, Debug)]
pub struct OpenCards {
    open: HashMap<ActivityId, usize>,
}

impl OpenCards {
    /// An island opened `id`'s card: whether it's the first (tell the module).
    pub fn opened(&mut self, id: &ActivityId) -> bool {
        let n = self.open.entry(id.clone()).or_default();
        *n += 1;
        *n == 1
    }

    /// An island closed `id`'s card: whether it was the last (tell the module).
    pub fn closed(&mut self, id: &ActivityId) -> bool {
        match self.open.get_mut(id) {
            Some(n) if *n > 1 => {
                *n -= 1;
                false
            }
            Some(_) => {
                self.open.remove(id);
                true
            }
            // Never counted (a rebuilt pill closing what its predecessor opened): say so.
            None => true,
        }
    }
}

thread_local! {
    static OPEN: RefCell<OpenCards> = RefCell::new(OpenCards::default());
}

pub fn opened(id: &ActivityId) -> bool {
    OPEN.with_borrow_mut(|o| o.opened(id))
}

pub fn closed(id: &ActivityId) -> bool {
    OPEN.with_borrow_mut(|o| o.closed(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(key: &str) -> ActivityId {
        ActivityId { module: "stats".into(), key: key.into() }
    }

    #[test]
    fn the_first_open_and_the_last_close_reach_the_module() {
        let mut cards = OpenCards::default();
        assert!(cards.opened(&id("card")), "display 1 opens it");
        assert!(!cards.opened(&id("card")), "display 2 too: already open");
        assert!(!cards.closed(&id("card")), "display 1 closes: still open on 2");
        assert!(cards.closed(&id("card")), "display 2 closes: now closed");
        assert!(cards.opened(&id("card")), "and opens again from scratch");
    }

    #[test]
    fn cards_are_counted_apart_and_an_unknown_close_still_closes() {
        let mut cards = OpenCards::default();
        assert!(cards.opened(&id("a")) && cards.opened(&id("b")));
        assert!(cards.closed(&id("a")));
        assert!(cards.closed(&id("never-opened")));
        assert!(cards.closed(&id("b")));
    }
}
