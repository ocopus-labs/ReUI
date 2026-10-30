//! How reminders read: the timer field's preview of one being typed, the next few under an
//! empty field, and the card of one that fired. Times come in already written in the
//! user's format (`time_of_day`), so this is plain text assembly and tested as such.

use reui_protocol::{Art, Button, ButtonContent, ButtonRole, CardSize, CompactView, Element, ExpandedView, Fact, Headline, Pager, Row, Tone};

use super::super::view::ACCENT;
use super::when::Every;
use crate::hub::View;

const GLYPH_BELL: char = '\u{EA8F}';

pub const ACTION_DONE: &str = "done";
pub const ACTION_SNOOZE: &str = "snooze";
pub const ACTION_PREV: &str = "prev";
pub const ACTION_NEXT: &str = "next";

/// One reminder as the cards show it: what, and when in words.
pub struct Shown<'a> {
    pub text: Option<&'a str>,
    /// "6:00 PM".
    pub time: String,
    /// "Tomorrow", "Fri", "Mon 3 Nov"; `None` for today.
    pub day: Option<String>,
    pub every: Option<Every>,
    pub late: bool,
}

/// What the timer field's hero shows for a reminder being typed: eyebrow, hero, detail.
pub fn preview(r: &Shown) -> (&'static str, String, Option<String>) {
    let when = r.every.map(|e| every_text(e).to_string()).or_else(|| r.day.clone());
    let detail = match (r.text, when) {
        (Some(t), Some(w)) => Some(format!("{t} · {w}")),
        (Some(t), None) => Some(t.to_string()),
        (None, w) => w,
    };
    ("Reminder", r.time.clone(), detail)
}

/// The next few, under an empty timer field: when, over what.
pub fn upcoming(list: &[Shown]) -> Option<Row> {
    let facts: Vec<Fact> = list
        .iter()
        .take(3)
        .map(|r| Fact { label: r.day.as_ref().map_or_else(|| r.time.clone(), |d| format!("{d} {}", r.time)), value: short(r.text.unwrap_or("Reminder")), tone: Tone::Primary })
        .collect();
    (!facts.is_empty()).then_some(Row::Facts(facts))
}

/// The card of the reminders that fired, `index` of them showing (the oldest first).
pub fn fired(shown: &Shown, index: usize, count: usize) -> View {
    let bell = Element::Glyph { glyph: GLYPH_BELL, tone: Tone::Accent };
    let headline = shown.text.map_or_else(|| shown.time.clone(), str::to_string);
    let when = match &shown.day {
        Some(day) => format!("{day} {}", shown.time),
        None => shown.time.clone(),
    };
    let mut detail = if shown.late { format!("Was due at {when}") } else { when };
    if let Some(every) = shown.every {
        detail = format!("{detail} · {}", every_text(every));
    }
    let pager = (count > 1).then(|| Pager { index: index as u32, count: count as u32, prev: ACTION_PREV.into(), next: ACTION_NEXT.into() });
    View {
        accent: Some(ACCENT),
        audio: None,
        compact: CompactView { leading: bell.clone(), trailing: Element::Text { text: short(&headline), tone: Tone::Primary } },
        minimal: bell,
        expanded: Some(ExpandedView {
            leading: Element::Art(Art::Glyph(GLYPH_BELL)),
            eyebrow: Some("Reminder".into()),
            headline: Headline::Text(headline),
            detail: Some(detail),
            trailing: Element::Empty,
            rows: vec![Row::Buttons(vec![text(ACTION_SNOOZE, "Snooze", ButtonRole::Secondary), text(ACTION_DONE, "Done", ButtonRole::Primary)])],
            context: None,
            size: CardSize::Regular,
            tint: false,
            pager,
            open: None,
        }),
    }
}

fn every_text(every: Every) -> &'static str {
    match every {
        Every::Day => "Every day",
        Every::Weekday => "Weekdays",
        Every::Week(_) => "Every week",
    }
}

/// Short enough for the pill's side or a fact: the first words, then an ellipsis.
fn short(text: &str) -> String {
    const MAX: usize = 18;
    if text.chars().count() <= MAX {
        return text.to_string();
    }
    let cut: String = text.chars().take(MAX - 1).collect();
    format!("{}…", cut.trim_end())
}

fn text(action: &str, label: &str, role: ButtonRole) -> Button {
    Button { action: action.into(), content: ButtonContent::Text(label.into()), role }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(text: Option<&str>, day: Option<&str>, every: Option<Every>, late: bool) -> Shown<'static> {
        let text = text.map(|t| &*Box::leak(t.to_string().into_boxed_str()));
        Shown { text, time: "6:00 PM".into(), day: day.map(str::to_string), every, late }
    }

    #[test]
    fn the_preview_reads_time_then_what_and_when() {
        assert_eq!(preview(&shown(Some("Call mom"), None, None, false)), ("Reminder", "6:00 PM".into(), Some("Call mom".into())));
        assert_eq!(preview(&shown(Some("Dentist"), Some("Tomorrow"), None, false)).2.as_deref(), Some("Dentist · Tomorrow"));
        assert_eq!(preview(&shown(Some("Meds"), None, Some(Every::Day), false)).2.as_deref(), Some("Meds · Every day"));
        assert_eq!(preview(&shown(None, Some("Fri"), None, false)).2.as_deref(), Some("Fri"));
    }

    #[test]
    fn a_fired_card_says_when_and_if_late() {
        let card = fired(&shown(Some("Call mom"), None, None, false), 0, 1).expanded.unwrap();
        assert_eq!((card.eyebrow.as_deref(), card.headline, card.detail.as_deref()), (Some("Reminder"), Headline::Text("Call mom".into()), Some("6:00 PM")));
        assert!(card.pager.is_none());
        let late = fired(&shown(Some("Bins"), Some("Wed"), Some(Every::Weekday), true), 1, 3).expanded.unwrap();
        assert_eq!(late.detail.as_deref(), Some("Was due at Wed 6:00 PM · Weekdays"));
        assert_eq!(late.pager.map(|p| (p.index, p.count)), Some((1, 3)));
        assert!(matches!(&late.rows[0], Row::Buttons(b) if b[1].action == ACTION_DONE && b[1].role == ButtonRole::Primary));
        // No text: the time is the hero.
        assert_eq!(fired(&shown(None, None, None, false), 0, 1).expanded.unwrap().headline, Headline::Text("6:00 PM".into()));
    }

    #[test]
    fn the_list_takes_three_and_keeps_them_short() {
        let list: Vec<Shown> = (0..5).map(|_| shown(Some("Renew the car insurance online"), Some("Tomorrow"), None, false)).collect();
        let Some(Row::Facts(facts)) = upcoming(&list) else { panic!() };
        assert_eq!(facts.len(), 3);
        assert_eq!((facts[0].label.as_str(), facts[0].value.as_str()), ("Tomorrow 6:00 PM", "Renew the car ins…"));
        assert!(upcoming(&[]).is_none());
    }
}
