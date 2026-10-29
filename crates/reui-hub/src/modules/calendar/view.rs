//! How the calendar looks (PRD §5.2–5.3): the countdown before a meeting, its start, and
//! the day's card. Every card reads eyebrow → hero → detail with Join, when there's a
//! meeting link, as the header's one action beside the hero; the rest of the day is an
//! agenda under it. Meaning only: the island decides the look.

use reui_protocol::{AgendaEntry, Art, Button, ButtonContent, ButtonRole, CardSize, Clock, CompactView, Element, ExpandedView, Headline, Rgb, Row, Tone};

use super::event::Event;
use crate::hub::View;
use crate::modules::timers::time_of_day;

/// Apple Calendar's red.
pub const ACCENT: Rgb = Rgb(255, 69, 58);
const GLYPH_CALENDAR: char = '\u{E787}';
const GLYPH_VIDEO: char = '\u{E714}';

pub const ACTION_JOIN: &str = "join";
pub const ACTION_SETTINGS: &str = "settings";

/// The last minutes before `e` (Ongoing): a ring running down to its start and its title.
pub fn countdown(e: &Event, later: &[&Event], now: u64, lead: u64) -> View {
    let ring = ring(e, now, lead);
    View {
        accent: Some(ACCENT),
        audio: None,
        compact: CompactView { leading: ring.clone(), trailing: Element::Text { text: title(e), tone: Tone::Primary } },
        minimal: ring.clone(),
        expanded: Some(card(e, later, ring, "Calendar", now, lead)),
    }
}

/// `e` has started (Alert): its glyph and title; Join in the card.
pub fn starting(e: &Event, later: &[&Event], now: u64, lead: u64) -> View {
    let glyph = if e.link.is_some() { GLYPH_VIDEO } else { GLYPH_CALENDAR };
    let mark = Element::Glyph { glyph, tone: Tone::Accent };
    View {
        accent: Some(ACCENT),
        audio: None,
        compact: CompactView { leading: mark.clone(), trailing: Element::Text { text: title(e), tone: Tone::Primary } },
        minimal: mark,
        expanded: Some(card(e, later, Element::Art(Art::Glyph(glyph)), "Calendar", now, lead)),
    }
}

/// The day's card (Passive, opened on purpose): the next event and the two after it, or
/// tomorrow's when today is done. `None` events: nothing in two days, or no calendars.
pub fn day(events: &[&Event], tomorrow: bool, now: u64, lead: u64, no_calendars: bool) -> View {
    let mark = Element::Glyph { glyph: GLYPH_CALENDAR, tone: Tone::Accent };
    let expanded = match events.split_first() {
        Some((hero, later)) => {
            let counting = !hero.all_day && hero.start.saturating_sub(lead) <= now && now < hero.start;
            let leading = if counting { ring(hero, now, lead) } else { Element::Art(Art::Glyph(GLYPH_CALENDAR)) };
            card(hero, later, leading, if tomorrow { "Tomorrow" } else { "Calendar" }, now, lead)
        }
        None => {
            let (hero, trailing) = if no_calendars { ("No calendars", Element::Button(text_button(ACTION_SETTINGS, "Add a calendar"))) } else { ("Nothing today", Element::Empty) };
            header(Element::Art(Art::Glyph(GLYPH_CALENDAR)), "Calendar", hero.into(), None, trailing, Vec::new())
        }
    };
    View { accent: Some(ACCENT), audio: None, compact: CompactView { leading: mark.clone(), trailing: Element::Empty }, minimal: mark, expanded: Some(expanded) }
}

fn card(e: &Event, later: &[&Event], leading: Element, eyebrow: &str, now: u64, lead: u64) -> ExpandedView {
    let detail = match e.where_() {
        Some(place) => format!("{} · {place}", span(e)),
        None => span(e),
    };
    let joinable = e.link.is_some() && !e.all_day && e.start.saturating_sub(lead) <= now && now < e.end;
    let trailing = if joinable { Element::Button(text_button(ACTION_JOIN, "Join")) } else { Element::Empty };
    let rows = if later.is_empty() { Vec::new() } else { vec![Row::Agenda(later.iter().map(|e| entry(e)).collect())] };
    header(leading, eyebrow, title(e), Some(detail), trailing, rows)
}

fn header(leading: Element, eyebrow: &str, hero: String, detail: Option<String>, trailing: Element, rows: Vec<Row>) -> ExpandedView {
    ExpandedView { context: None, leading, eyebrow: Some(eyebrow.into()), headline: Headline::Text(hero), detail, trailing, rows, size: CardSize::Regular, tint: false, pager: None, open: None }
}

fn entry(e: &Event) -> AgendaEntry {
    AgendaEntry { time: if e.all_day { "All day".into() } else { time_of_day(e.start) }, title: title(e), detail: e.where_() }
}

/// The time left to its start out of `lead`, running down by itself in the island.
fn ring(e: &Event, now: u64, lead: u64) -> Element {
    let clock = Clock { value_ms: e.start.saturating_sub(now), at_unix_ms: now, running: true, down: true };
    Element::ClockRing { clock, total_ms: lead.max(1) }
}

/// "10:00 – 10:15"; "All day"; a moment with no length is just its time.
fn span(e: &Event) -> String {
    if e.all_day {
        "All day".into()
    } else if e.end <= e.start {
        time_of_day(e.start)
    } else {
        format!("{} – {}", time_of_day(e.start), time_of_day(e.end))
    }
}

/// An untitled event is what a free/busy-only feed sends.
fn title(e: &Event) -> String {
    if e.title.trim().is_empty() { "Busy".into() } else { e.title.clone() }
}

fn text_button(action: &str, label: &str) -> Button {
    Button { action: action.into(), content: ButtonContent::Text(label.into()), role: ButtonRole::Primary }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::calendar::links::{Link, Service};

    const MIN: u64 = 60_000;

    fn ev(title: &str, start_min: u64, link: bool) -> Event {
        let link = link.then(|| Link { url: "https://zoom.us/j/1".into(), service: Service::Zoom });
        Event { key: title.into(), title: title.into(), start: start_min * MIN, end: (start_min + 30) * MIN, all_day: false, place: None, link }
    }

    #[test]
    fn the_countdown_runs_to_the_start_and_offers_join() {
        let standup = ev("Standup", 600, true);
        let review = ev("Review", 700, false);
        let v = countdown(&standup, &[&review], 595 * MIN, 10 * MIN);
        let Element::ClockRing { clock, total_ms } = v.compact.leading else { panic!() };
        assert_eq!((clock.value_ms, clock.down, total_ms), (5 * MIN, true, 10 * MIN));
        let card = v.expanded.unwrap();
        assert_eq!(card.headline, Headline::Text("Standup".into()));
        assert!(card.detail.unwrap().ends_with(" · Zoom"));
        assert!(matches!(card.trailing, Element::Button(Button { ref action, .. }) if action == ACTION_JOIN));
        assert!(matches!(&card.rows[..], [Row::Agenda(lines)] if lines.len() == 1 && lines[0].title == "Review"));
    }

    #[test]
    fn join_only_with_a_link_and_only_near_the_meeting() {
        let lunch = ev("Lunch", 720, false);
        assert_eq!(starting(&lunch, &[], 720 * MIN, 10 * MIN).expanded.unwrap().trailing, Element::Empty);
        let later = ev("Later", 900, true);
        let card = day(&[&later], false, 600 * MIN, 10 * MIN, false).expanded.unwrap();
        assert_eq!(card.trailing, Element::Empty, "hours before, no Join yet");
        assert!(card.rows.is_empty(), "nothing after it: no agenda row");
    }

    #[test]
    fn an_empty_day_says_so_once_and_no_calendars_offers_settings() {
        let none = day(&[], false, 0, 10 * MIN, true).expanded.unwrap();
        assert_eq!(none.headline, Headline::Text("No calendars".into()));
        assert!(matches!(none.trailing, Element::Button(Button { ref action, .. }) if action == ACTION_SETTINGS));
        let free = day(&[], false, 0, 10 * MIN, false).expanded.unwrap();
        assert_eq!((free.headline, free.detail, free.rows.len()), (Headline::Text("Nothing today".into()), None, 0));
        let tomorrow = day(&[&ev("Early", 2000, false)], true, 0, 10 * MIN, false).expanded.unwrap();
        assert_eq!(tomorrow.eyebrow.as_deref(), Some("Tomorrow"));
        assert_eq!(title(&ev("  ", 0, false)), "Busy");
    }
}
