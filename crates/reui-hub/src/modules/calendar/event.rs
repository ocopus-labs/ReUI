//! An event as the island needs it: one occurrence (a recurring meeting expanded), its
//! times in Unix ms, and what the card says about it. Every source (an ICS feed, Windows'
//! store) produces these, so the schedule and views never know where one came from.

use super::links::Link;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// Stable across refreshes: the source, the event's UID and this occurrence's start,
    /// so a dismissed start stays dismissed when the feed is fetched again.
    pub key: String,
    pub title: String,
    pub start: u64,
    pub end: u64,
    pub all_day: bool,
    /// A room or address worth showing; never a link.
    pub place: Option<String>,
    pub link: Option<Link>,
}

impl Event {
    /// The one place the card names: the room, else the meeting service.
    pub fn where_(&self) -> Option<String> {
        self.place.clone().or_else(|| self.link.as_ref().map(|l| l.service.name().to_string()))
    }
}

/// Where events come from: a feed, Windows' store, later an account. Each gives its
/// occurrences in a window; fetching is the module's business, not the source's.
pub trait Source {
    /// Occurrences overlapping `from..to` (Unix ms).
    fn events(&self, from: u64, to: u64) -> Vec<Event>;
}
