//! How the pointer's intent is read (docs/PRD-behaviour.md §4.1–4.2): whether a rest opens
//! the card or only peeks, how long the rest is, how much of the top edge reveals a hidden
//! island, and when the drawn pill may take a click at once. The rules are pure; the pill
//! (`pill/input.rs`) feeds them the pointer and the clock.

use std::time::{Duration, Instant};

use reui_design::tokens::motion;

/// `open_on`: what resting the pointer on the pill does.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum OpenOn {
    /// A rest peeks; a click opens (as ReUI always did).
    #[default]
    Click,
    /// A rest opens the card, for people who'd rather not click.
    Hover,
}

impl OpenOn {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "click" => Some(OpenOn::Click),
            "hover" => Some(OpenOn::Hover),
            _ => None,
        }
    }
}

/// `reveal_zone`: how much of the top edge reveals the island in a hidden visibility mode.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum RevealZone {
    /// The top edge never reveals it.
    Off,
    /// Only across the pill's own width, so the rest of the edge stays the browser's (tabs
    /// sit up there, so this is the default).
    #[default]
    Pill,
    /// Across the island's window, the widest card's width (as ReUI did before).
    Wide,
}

impl RevealZone {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "off" => Some(RevealZone::Off),
            "pill" => Some(RevealZone::Pill),
            "wide" => Some(RevealZone::Wide),
            _ => None,
        }
    }
}

/// The rest the settings file may ask for: shorter can't tell a pass from a rest, longer
/// feels broken.
pub const HOVER_DELAY_RANGE: std::ops::RangeInclusive<u32> = 50..=2000;

/// `hover_delay` as read from the file: in range, or the token's default.
pub fn hover_delay(value: Option<u64>) -> u32 {
    value.and_then(|ms| u32::try_from(ms).ok()).filter(|ms| HOVER_DELAY_RANGE.contains(ms)).unwrap_or(motion::delay::HOVER_INTENT)
}

/// What a rest on the pill does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rest {
    /// Grow a little (and take clicks).
    Peek,
    /// Open the card.
    Open,
    /// Only take clicks (the bubble, or a pill already open).
    Nothing,
}

/// What resting on the island does: `compact` is a compact (or idle) pill under the pointer,
/// `card` whether the front activity has a card, `bubble` the pointer is on the bubble. The
/// idle pill only peeks with "open on hover" too: its row of places is a menu of doors, and
/// opening one by brushing past would be the very thing this setting is careful about.
pub fn after_rest(open_on: OpenOn, compact: bool, card: bool, bubble: bool) -> Rest {
    match (compact && !bubble, open_on, card) {
        (false, ..) => Rest::Nothing,
        (true, OpenOn::Hover, true) => Rest::Open,
        (true, ..) => Rest::Peek,
    }
}

/// Whether a click on the drawn pill is the island's at once, before any rest. Only when it
/// had been on screen, as it is, for a whole `delay` when the pointer arrived: then a quick
/// click there is clearly aimed at it. A pill that appeared under the pointer (revealed from
/// the edge, shown by a moment while the pointer was up there, a decision opening itself)
/// was never aimed at, so it waits for the rest like the band above it, and the click
/// reaches the browser tab (or doesn't land on Allow by accident).
pub fn clicks_at_once(appeared_at: Option<Instant>, arrived_at: Instant, delay: Duration) -> bool {
    appeared_at.is_some_and(|appeared| arrived_at.checked_duration_since(appeared).is_some_and(|on_screen| on_screen >= delay))
}

/// Whether screen x `x` at the top edge reveals the island: `window` is the island's window
/// span and `pill` the pill's own span, both in screen pixels.
pub fn reveals(zone: RevealZone, x: i32, window: (i32, i32), pill: (i32, i32)) -> bool {
    let within = |(left, right): (i32, i32)| x >= left && x < right;
    match zone {
        RevealZone::Off => false,
        RevealZone::Pill => within(pill),
        RevealZone::Wide => within(window),
    }
}

/// The pill's settings for reading intent, and the two moments the click rule compares.
#[derive(Clone, Copy, Debug)]
pub struct Intent {
    pub open_on: OpenOn,
    /// The rest, in ms (`hover_delay`).
    pub delay: u32,
    pub reveal_zone: RevealZone,
    /// When the island last came on screen or opened by itself; `None` while hidden.
    pub appeared_at: Option<Instant>,
    /// When the pointer last came onto the island.
    pub arrived_at: Instant,
}

impl Default for Intent {
    fn default() -> Self {
        let now = Instant::now();
        Self { open_on: OpenOn::default(), delay: motion::delay::HOVER_INTENT, reveal_zone: RevealZone::default(), appeared_at: Some(now), arrived_at: now }
    }
}

impl Intent {
    /// The pointer is on the drawn pill: whether it takes a click before the rest.
    pub fn clicks_at_once(&self) -> bool {
        clicks_at_once(self.appeared_at, self.arrived_at, Duration::from_millis(u64::from(self.delay)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_parse_with_defaults() {
        assert_eq!((OpenOn::parse("hover"), OpenOn::parse("click"), OpenOn::parse("tap")), (Some(OpenOn::Hover), Some(OpenOn::Click), None));
        assert_eq!((RevealZone::parse("off"), RevealZone::parse("pill"), RevealZone::parse("wide"), RevealZone::parse("")), (Some(RevealZone::Off), Some(RevealZone::Pill), Some(RevealZone::Wide), None));
        assert_eq!((OpenOn::default(), RevealZone::default()), (OpenOn::Click, RevealZone::Pill), "the top edge stays the browser's");
        assert_eq!((hover_delay(Some(150)), hover_delay(Some(500))), (150, 500));
        assert_eq!(hover_delay(None), motion::delay::HOVER_INTENT);
        assert_eq!((hover_delay(Some(0)), hover_delay(Some(10_000)), hover_delay(Some(u64::MAX))), (250, 250, 250));
    }

    #[test]
    fn a_rest_opens_only_with_open_on_hover_and_a_card() {
        assert_eq!(after_rest(OpenOn::Click, true, true, false), Rest::Peek);
        assert_eq!(after_rest(OpenOn::Hover, true, true, false), Rest::Open);
        // Nothing to open (a level, the idle pill): a peek.
        assert_eq!(after_rest(OpenOn::Hover, true, false, false), Rest::Peek);
        // The bubble only becomes clickable; an open card stays as it is.
        assert_eq!(after_rest(OpenOn::Hover, true, true, true), Rest::Nothing);
        assert_eq!(after_rest(OpenOn::Hover, false, true, false), Rest::Nothing);
    }

    #[test]
    fn a_pill_that_appeared_under_the_pointer_waits_for_the_rest() {
        let t = Instant::now();
        let delay = Duration::from_millis(250);
        let at = |ms: u64| t + Duration::from_millis(ms);
        // On screen well before the pointer came: a quick click is aimed at it.
        assert!(clicks_at_once(Some(t), at(250), delay));
        assert!(clicks_at_once(Some(t), at(5_000), delay));
        // Revealed (or shown by a moment) as the pointer got there: not yet.
        assert!(!clicks_at_once(Some(t), at(0), delay));
        assert!(!clicks_at_once(Some(t), at(249), delay));
        // Hidden: never.
        assert!(!clicks_at_once(None, at(1_000), delay));
        // The pointer was there first.
        assert!(!clicks_at_once(Some(at(100)), at(0), delay));
    }

    #[test]
    fn the_reveal_zone_is_as_wide_as_chosen() {
        let (window, pill) = ((0, 900), (390, 510));
        assert!(!reveals(RevealZone::Off, 450, window, pill));
        assert!(reveals(RevealZone::Pill, 450, window, pill) && reveals(RevealZone::Pill, 390, window, pill));
        assert!(!reveals(RevealZone::Pill, 380, window, pill) && !reveals(RevealZone::Pill, 510, window, pill));
        assert!(reveals(RevealZone::Wide, 10, window, pill) && !reveals(RevealZone::Wide, 900, window, pill));
    }
}
