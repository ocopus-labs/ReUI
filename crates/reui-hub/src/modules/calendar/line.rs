//! One ICS link and where its refreshing is: when it's next asked, whether a fetch is out,
//! which edit of the link that fetch was for, and when its body last really changed. Pure
//! (times are passed in), so the edit race, the debounce and "checked vs changed" are
//! tested with a fake clock; the module does the network and the disk.

use std::sync::Arc;

use super::fetch::{self, Cached, Validators};
use super::ics::Feed;
use super::settings::Source;
use super::status::Refresh;

/// A refresh asked for again sooner than this (resume, then the network, then unlock)
/// is the same refresh.
pub const DEBOUNCE_MS: u64 = 60_000;

pub struct Line {
    pub source: Source,
    pub feed: Option<Arc<Feed>>,
    pub validators: Validators,
    pub failures: u32,
    /// Next refresh (Unix ms).
    pub due: u64,
    pub busy: bool,
    tried: u64,
    /// Which edit of the link this is. A fetch is tagged with it and dropped when it lands
    /// after the link changed, so an old link's body can't replace the new one's.
    pub generation: u64,
    /// The body held, fingerprinted, so a 200 with the same body isn't a change.
    digest: Option<blake3::Hash>,
    /// When that body first came (Unix ms).
    changed: Option<u64>,
}

/// How a fetch went.
pub enum Outcome {
    NotModified,
    Body { feed: Feed, text: String, validators: Validators },
    Failed(String),
}

/// A fetch that landed for the link as it is now.
pub struct Landed {
    pub refresh: Refresh,
    /// The feed changed: work the schedule out again.
    pub new_feed: bool,
    /// The copy to keep on disk.
    pub save: Option<Cached>,
}

impl Line {
    pub fn new(source: Source, generation: u64) -> Self {
        Self { source, feed: None, validators: Validators::default(), failures: 0, due: 0, busy: false, tried: 0, generation, digest: None, changed: None }
    }

    /// The copy from disk, unless it's for an older link or a fetch got here first.
    pub fn cached(&mut self, generation: u64, feed: Feed, cached: Cached) -> bool {
        if generation != self.generation || self.feed.is_some() {
            return false;
        }
        self.feed = Some(Arc::new(feed));
        self.digest = Some(blake3::hash(cached.text.as_bytes()));
        self.changed = cached.changed;
        self.validators = cached.validators;
        true
    }

    /// Starts a fetch when one is due: the generation to tag it with.
    pub fn start(&mut self, now: u64) -> Option<u64> {
        if !self.source.on || self.busy || self.due > now {
            return None;
        }
        self.busy = true;
        self.tried = now;
        Some(self.generation)
    }

    /// Asks again now. Resume, unlock and the network coming back arrive together and
    /// fold into one; `force` (asked for from Settings) never waits.
    pub fn ask(&mut self, now: u64, force: bool) {
        if force || now.saturating_sub(self.tried) > DEBOUNCE_MS {
            self.due = now;
        }
    }

    /// The internet is back: again now if the last try failed.
    pub fn retry(&mut self, now: u64) {
        if self.failures > 0 {
            self.ask(now, false);
        }
    }

    /// A fetch landed. `None` when it was for an older edit of the link: dropped, and the
    /// fetch for the new link (already started or due) stands.
    pub fn land(&mut self, generation: u64, now: u64, outcome: Outcome) -> Option<Landed> {
        if generation != self.generation {
            return None;
        }
        self.busy = false;
        let (mut new_feed, mut save, mut error) = (false, None, None);
        match outcome {
            Outcome::NotModified => self.failures = 0,
            Outcome::Body { feed, text, validators } => {
                self.failures = 0;
                let digest = blake3::hash(text.as_bytes());
                if self.digest != Some(digest) || self.changed.is_none() {
                    self.changed = Some(now);
                }
                self.digest = Some(digest);
                self.feed = Some(Arc::new(feed));
                self.validators = validators.clone();
                new_feed = true;
                save = Some(Cached { validators, text, changed: self.changed });
            }
            Outcome::Failed(why) => {
                self.failures += 1;
                error = Some(why);
            }
        }
        self.due = now + fetch::retry_after(self.failures);
        let refresh = Refresh {
            ok: error.is_none(),
            at: now,
            events: self.feed.as_ref().map_or(0, |f| f.len()),
            error,
            changed: self.changed,
            modified: self.validators.last_modified.as_deref().and_then(fetch::http_date_ms),
        };
        Some(Landed { refresh, new_feed, save })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60_000;
    const ICS: &str = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:a\r\nDTSTART:20261030T100000Z\r\nDTEND:20261030T103000Z\r\nSUMMARY:One\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    const ICS2: &str = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:b\r\nDTSTART:20261030T110000Z\r\nDTEND:20261030T113000Z\r\nSUMMARY:Two\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    fn source(url: &str) -> Source {
        Source { id: "s1".into(), host: "example.com".into(), url: url.into(), on: true }
    }

    fn body(text: &str, last_modified: Option<&str>) -> Outcome {
        Outcome::Body { feed: Feed::parse("s1", text), text: text.into(), validators: Validators { etag: None, last_modified: last_modified.map(str::to_string) } }
    }

    #[test]
    fn a_fetch_for_an_edited_link_is_dropped() {
        let mut line = Line::new(source("https://old.example.com/a.ics"), 1);
        let old = line.start(0).unwrap();
        // The link is edited while the old fetch is out: a new line, a new fetch at once.
        line = Line::new(source("https://new.example.com/b.ics"), 2);
        let new = line.start(10).unwrap();
        assert!(line.land(old, 20, body(ICS, None)).is_none(), "the old link's body is dropped");
        assert!(line.feed.is_none() && line.busy, "and the new fetch is still out");
        let landed = line.land(new, 30, body(ICS2, None)).unwrap();
        assert!(landed.new_feed && landed.save.is_some() && landed.refresh.ok);
        assert_eq!(line.feed.as_ref().unwrap().len(), 1);
        assert!(!line.cached(old, Feed::parse("s1", ICS), Cached { validators: Validators::default(), text: ICS.into(), changed: Some(0) }), "nor does an old copy from disk");
    }

    #[test]
    fn checked_is_not_changed() {
        let mut line = Line::new(source("https://example.com/a.ics"), 1);
        let g = line.start(0).unwrap();
        let first = line.land(g, 0, body(ICS, Some("Wed, 21 Oct 2015 07:28:00 GMT"))).unwrap().refresh;
        assert_eq!((first.at, first.changed, first.modified), (0, Some(0), Some(1_445_412_480_000)));
        // A 304 an hour later: checked now, changed still at the start.
        line.ask(60 * MIN, false);
        let g = line.start(60 * MIN).unwrap();
        let r = line.land(g, 60 * MIN, Outcome::NotModified).unwrap();
        assert!(!r.new_feed && r.save.is_none());
        assert_eq!((r.refresh.ok, r.refresh.at, r.refresh.changed, r.refresh.events), (true, 60 * MIN, Some(0), 1));
        // The same body again (a server without validators): still not a change.
        line.ask(120 * MIN, false);
        let g = line.start(120 * MIN).unwrap();
        assert_eq!(line.land(g, 120 * MIN, body(ICS, None)).unwrap().refresh.changed, Some(0));
        // A different body is.
        line.ask(180 * MIN, false);
        let g = line.start(180 * MIN).unwrap();
        let landed = line.land(g, 180 * MIN, body(ICS2, None)).unwrap();
        assert_eq!((landed.refresh.changed, landed.save.unwrap().changed), (Some(180 * MIN), Some(180 * MIN)));
    }

    #[test]
    fn the_copy_from_disk_keeps_when_it_changed() {
        let mut line = Line::new(source("https://example.com/a.ics"), 1);
        assert!(line.cached(1, Feed::parse("s1", ICS), Cached { validators: Validators::default(), text: ICS.into(), changed: Some(5 * MIN) }));
        let g = line.start(100 * MIN).unwrap();
        assert_eq!(line.land(g, 100 * MIN, body(ICS, None)).unwrap().refresh.changed, Some(5 * MIN), "the same body as on disk");
    }

    #[test]
    fn failures_keep_the_copy_and_say_why() {
        let mut line = Line::new(source("https://example.com/a.ics"), 1);
        let g = line.start(0).unwrap();
        line.land(g, 0, body(ICS, None));
        line.ask(20 * MIN, false);
        let g = line.start(20 * MIN).unwrap();
        let r = line.land(g, 20 * MIN, Outcome::Failed("timed out".into())).unwrap().refresh;
        assert_eq!((r.ok, r.error.as_deref(), r.events, r.changed), (false, Some("timed out"), 1, Some(0)));
        assert!(line.feed.is_some(), "the island keeps the last good copy");
        assert_eq!(line.due, 21 * MIN, "and asks again soon");
    }

    #[test]
    fn automatic_asks_fold_but_settings_never_waits() {
        let mut line = Line::new(source("https://example.com/a.ics"), 1);
        let g = line.start(0).unwrap();
        line.land(g, 0, body(ICS, None));
        assert_eq!(line.due, fetch::REFRESH_MS);
        line.ask(30_000, false);
        assert_eq!(line.due, fetch::REFRESH_MS, "resume right after a refresh is the same refresh");
        line.retry(30_000);
        assert_eq!(line.due, fetch::REFRESH_MS, "nothing failed");
        line.ask(30_000, true);
        assert_eq!(line.due, 30_000, "Settings' refresh goes now");
        assert!(line.start(30_000).is_some());
        assert!(line.start(30_000).is_none(), "one fetch out at a time");
    }
}
