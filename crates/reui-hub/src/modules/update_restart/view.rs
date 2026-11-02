//! The note (docs/PRD-update-restart.md §4): the pill says Restart beside the update
//! glyph; open, Windows Update as the eyebrow, Restart needed as the hero, when Windows
//! restarts by itself as the detail, and Schedule / Restart now. Pure.

use reui_protocol::{Button, ButtonContent, ButtonRole, CardSize, CompactView, Element, ExpandedView, Headline, Rgb, Row, Tone};

use super::plan::State;
use crate::hub::View;

/// Windows' own blue: it's Windows speaking, not an app.
const ACCENT: Rgb = Rgb(0, 120, 212);
const GLYPH: char = '\u{E777}';

pub const SCHEDULE: &str = "schedule";
pub const RESTART: &str = "restart";

pub fn view(state: &State, confirming: bool) -> View {
    let mark = Element::Glyph { glyph: GLYPH, tone: Tone::Accent };
    let restart = if confirming { "Restart \u{00B7} sure?" } else { "Restart now" };
    let buttons = vec![
        Button { action: SCHEDULE.into(), content: ButtonContent::Text("Schedule".into()), role: ButtonRole::Secondary },
        Button { action: RESTART.into(), content: ButtonContent::Text(restart.into()), role: if confirming { ButtonRole::Destructive } else { ButtonRole::Accent } },
    ];
    View {
        accent: Some(ACCENT),
        audio: None,
        compact: CompactView { leading: mark.clone(), trailing: Element::Text { text: "Restart".into(), tone: Tone::Accent } },
        minimal: mark,
        expanded: Some(ExpandedView {
            context: None,
            leading: Element::Empty,
            eyebrow: Some("Windows Update".into()),
            headline: Headline::Text("Restart needed".into()),
            detail: state.active.map(outside),
            trailing: Element::Empty,
            rows: vec![Row::Buttons(buttons)],
            size: CardSize::Roomy,
            tint: false,
            pager: None,
            open: None,
        }),
    }
}

/// When Windows restarts by itself: outside active hours.
fn outside((start, end): (u32, u32)) -> String {
    format!("Windows restarts on its own between {end:02}:00 and {start:02}:00")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_note_says_when_windows_restarts_and_asks_before_restarting() {
        let state = State { waiting: vec!["a".into()], active: Some((8, 17)) };
        let v = view(&state, false);
        let e = v.expanded.unwrap();
        assert_eq!(e.headline, Headline::Text("Restart needed".into()));
        assert_eq!(e.detail.as_deref(), Some("Windows restarts on its own between 17:00 and 08:00"));
        let Row::Buttons(b) = &e.rows[0] else { panic!() };
        assert_eq!((b[0].action.as_str(), b[1].action.as_str()), (SCHEDULE, RESTART));
        assert_eq!(b[1].content, ButtonContent::Text("Restart now".into()));
        let Row::Buttons(b) = &view(&state, true).expanded.unwrap().rows[0] else { panic!() };
        assert_eq!((b[1].content.clone(), b[1].role), (ButtonContent::Text("Restart \u{00B7} sure?".into()), ButtonRole::Destructive));
        // Active hours unknown: no guess.
        assert_eq!(view(&State { active: None, ..state }, false).expanded.unwrap().detail, None);
    }
}
