//! How the pointer's intent is read (docs/PRD-behaviour.md §4.1): whether a rest opens the
//! card or only peeks, and how long the rest is. The rules are pure; the pill
//! (`pill/input.rs`) feeds them the pointer and the clock.

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

/// The pill's settings for reading intent.
#[derive(Clone, Copy, Debug)]
pub struct Intent {
    pub open_on: OpenOn,
    /// The rest, in ms (`hover_delay`).
    pub delay: u32,
}

impl Default for Intent {
    fn default() -> Self {
        Self { open_on: OpenOn::default(), delay: motion::delay::HOVER_INTENT }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_parse_with_defaults() {
        assert_eq!((OpenOn::parse("hover"), OpenOn::parse("click"), OpenOn::parse("tap")), (Some(OpenOn::Hover), Some(OpenOn::Click), None));
        assert_eq!(OpenOn::default(), OpenOn::Click, "today's behaviour");
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
}
