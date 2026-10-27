//! How long each Devices activity stays: a moment lingers for its time, longer when it's
//! upgraded (the battery arrived), and as long as its card is open, then a little after it
//! closes. Pure: the module asks what's due.

use std::collections::HashMap;
use std::time::Duration;

use tokio::time::Instant;

/// After a card closes.
pub const AFTER_CLOSE: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone, Copy)]
struct Moment {
    until: Instant,
    open: bool,
}

#[derive(Debug, Default)]
pub struct Moments {
    live: HashMap<String, Moment>,
}

impl Moments {
    /// Shows (or keeps) `key` for at least `linger` from now.
    pub fn show(&mut self, key: &str, now: Instant, linger: Duration) {
        let until = now + linger;
        self.live.entry(key.to_owned()).and_modify(|m| m.until = m.until.max(until)).or_insert(Moment { until, open: false });
    }

    pub fn is_live(&self, key: &str) -> bool {
        self.live.contains_key(key)
    }

    pub fn opened(&mut self, key: &str) {
        if let Some(m) = self.live.get_mut(key) {
            m.open = true;
        }
    }

    pub fn closed(&mut self, key: &str, now: Instant, after: Duration) {
        if let Some(m) = self.live.get_mut(key) {
            m.open = false;
            m.until = now + after;
        }
    }

    pub fn remove(&mut self, key: &str) -> bool {
        self.live.remove(key).is_some()
    }

    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.live.keys()
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.live.values().filter(|m| !m.open).map(|m| m.until).min()
    }

    /// Removes and returns the moments whose time is up.
    pub fn due(&mut self, now: Instant) -> Vec<String> {
        let due: Vec<String> = self.live.iter().filter(|(_, m)| !m.open && now >= m.until).map(|(k, _)| k.clone()).collect();
        for k in &due {
            self.live.remove(k);
        }
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_moments_linger_and_hold_while_open() {
        let t = Instant::now();
        let s = |secs: u64| t + Duration::from_secs(secs);
        let mut m = Moments::default();
        m.show("device:1", t, Duration::from_secs(3));
        // An upgrade keeps it a little longer, never shorter.
        m.show("device:1", s(2), Duration::from_secs(2));
        m.show("device:1", s(2), Duration::from_millis(10));
        assert_eq!(m.next_deadline(), Some(s(4)));
        assert!(m.due(s(3)).is_empty());
        m.opened("device:1");
        assert_eq!(m.next_deadline(), None);
        assert!(m.due(s(100)).is_empty());
        m.closed("device:1", s(100), AFTER_CLOSE);
        assert!(m.due(s(101)).is_empty());
        assert_eq!(m.due(s(102)), ["device:1"]);
        assert!(!m.is_live("device:1") && !m.remove("device:1"));
    }
}
