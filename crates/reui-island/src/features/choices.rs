//! The keyboard in a card that asks something (a choice list, an answer field), once a
//! click has given it the keyboard. Pure, so the rules are testable without a window;
//! `pill::summon` applies what it decides.
//!
//! A card that arrives by itself never gets here: a stray Enter while typing in an editor
//! must not answer anything (agent moments PRD §5.7).

use reui_protocol::{ButtonRole, Choice, ExpandedView, Row};

use crate::features::clipboard::Key;

/// What a key does in such a card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Not ours: the panel handles it as it always has (typing into the field, Enter
    /// submitting what was typed, Esc in a summoned card).
    Pass,
    /// Swallowed with nothing to do.
    Ignore,
    /// Move the keyboard cursor to this choice.
    Cursor(usize),
    /// Send this action; `at` moves the cursor there first (a picked choice).
    Send { action: String, at: Option<usize> },
    /// Give the keyboard back to the app in front, leaving the card open.
    Release,
}

/// Whether a card can use the keyboard at all: it has a choice list or a field.
pub fn wants_keys(view: &ExpandedView) -> bool {
    view.rows.iter().any(|r| matches!(r, Row::Choices { .. } | Row::Search { .. }))
}

fn list(view: &ExpandedView) -> Option<(&[Choice], bool)> {
    view.rows.iter().find_map(|r| match r {
        Row::Choices { options, multi } => Some((options.as_slice(), *multi)),
        _ => None,
    })
}

/// The card's primary button, which Enter presses when no single choice is under the cursor.
fn primary(view: &ExpandedView) -> Option<String> {
    view.rows.iter().find_map(|r| match r {
        Row::Buttons(buttons) => buttons.iter().find(|b| b.role == ButtonRole::Primary).map(|b| b.action.clone()),
        _ => None,
    })
}

/// What `key` does. `cursor` is the choice under the keyboard cursor, `typing` whether the
/// field has text (then digits and Space type, and Enter sends the text), `summoned`
/// whether the card was opened by a hotkey (then Esc keeps its old meaning: close).
pub fn on_key(key: Key, view: &ExpandedView, cursor: Option<usize>, typing: bool, summoned: bool) -> Effect {
    let field = view.rows.iter().any(|r| matches!(r, Row::Search { .. }));
    let pass_or_ignore = if field { Effect::Pass } else { Effect::Ignore };
    if key == Key::Escape {
        return if typing || summoned { Effect::Pass } else { Effect::Release };
    }
    let Some((options, multi)) = list(view).filter(|(o, _)| !o.is_empty()) else {
        return match key {
            Key::Activate if !field => primary(view).map_or(Effect::Ignore, |action| Effect::Send { action, at: None }),
            _ => pass_or_ignore,
        };
    };
    let last = options.len() - 1;
    let pick = |i: usize| Effect::Send { action: options[i].action.clone(), at: Some(i) };
    match key {
        Key::Move(d) => Effect::Cursor(match cursor {
            Some(at) => at.saturating_add_signed(d as isize).min(last),
            None if d < 0 => last,
            None => 0,
        }),
        Key::First => Effect::Cursor(0),
        Key::Last => Effect::Cursor(last),
        Key::Type(c @ '1'..='9') if !typing => {
            let i = c as usize - '1' as usize;
            if i <= last { pick(i) } else { Effect::Ignore }
        }
        Key::Type(' ') if !typing => cursor.filter(|&i| i <= last).map_or(Effect::Ignore, pick),
        Key::Activate if typing => Effect::Pass,
        Key::Activate => match (multi, cursor.filter(|&i| i <= last)) {
            (false, Some(i)) => pick(i),
            _ => primary(view).map_or(Effect::Ignore, |action| Effect::Send { action, at: None }),
        },
        _ => pass_or_ignore,
    }
}

/// Whether the list changed into another one (the next step of a question), so the cursor
/// starts over; a re-publish with a check flipped keeps it.
pub fn same_list(a: Option<&ExpandedView>, b: Option<&ExpandedView>) -> bool {
    let labels = |v: Option<&ExpandedView>| v.and_then(list).map(|(o, _)| o.iter().map(|c| c.label.clone()).collect::<Vec<_>>());
    let headline = |v: Option<&ExpandedView>| v.map(|v| format!("{:?}", v.headline));
    labels(a) == labels(b) && headline(a) == headline(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reui_protocol::{Button, ButtonContent, CardSize, Element, Headline};

    fn view(multi: bool, field: bool) -> ExpandedView {
        let choice = |i: usize| Choice { label: format!("Option {i}"), detail: None, selected: false, action: format!("pick:{i}") };
        let mut rows = vec![Row::Choices { options: (0..3).map(choice).collect(), multi }];
        if field {
            rows.push(Row::Search { query: String::new(), buttons: vec![], placeholder: None, glyph: None });
        }
        rows.push(Row::Buttons(vec![
            Button { action: "back".into(), content: ButtonContent::Text("Back".into()), role: ButtonRole::Secondary },
            Button { action: "next".into(), content: ButtonContent::Text("Next".into()), role: ButtonRole::Primary },
        ]));
        ExpandedView { context: None, leading: Element::Empty, eyebrow: None, headline: Headline::Text("Which?".into()), detail: None, trailing: Element::Empty, rows, size: CardSize::Roomy, tint: false, pager: None, open: None }
    }

    fn send(action: &str, at: Option<usize>) -> Effect {
        Effect::Send { action: action.into(), at }
    }

    #[test]
    fn arrows_move_and_stop_at_the_ends() {
        let v = view(false, false);
        assert_eq!(on_key(Key::Move(1), &v, None, false, false), Effect::Cursor(0));
        assert_eq!(on_key(Key::Move(-1), &v, None, false, false), Effect::Cursor(2));
        assert_eq!(on_key(Key::Move(1), &v, Some(2), false, false), Effect::Cursor(2));
        assert_eq!(on_key(Key::Move(-1), &v, Some(0), false, false), Effect::Cursor(0));
        assert_eq!(on_key(Key::Last, &v, Some(0), false, false), Effect::Cursor(2));
    }

    #[test]
    fn digits_and_space_pick_and_enter_answers() {
        let single = view(false, false);
        assert_eq!(on_key(Key::Type('2'), &single, None, false, false), send("pick:1", Some(1)));
        assert_eq!(on_key(Key::Type('9'), &single, None, false, false), Effect::Ignore);
        assert_eq!(on_key(Key::Type(' '), &single, Some(2), false, false), send("pick:2", Some(2)));
        assert_eq!(on_key(Key::Activate, &single, Some(0), false, false), send("pick:0", Some(0)));
        // Several to pick: Enter is the primary button, Space toggles.
        let multi = view(true, false);
        assert_eq!(on_key(Key::Activate, &multi, Some(1), false, false), send("next", None));
        assert_eq!(on_key(Key::Type(' '), &multi, Some(1), false, false), send("pick:1", Some(1)));
        assert_eq!(on_key(Key::Type('x'), &multi, Some(1), false, false), Effect::Ignore, "nothing to type into");
    }

    #[test]
    fn a_field_with_text_takes_digits_space_and_enter() {
        let v = view(false, true);
        assert_eq!(on_key(Key::Type('1'), &v, Some(0), true, false), Effect::Pass);
        assert_eq!(on_key(Key::Type(' '), &v, Some(0), true, false), Effect::Pass);
        assert_eq!(on_key(Key::Activate, &v, Some(0), true, false), Effect::Pass);
        assert_eq!(on_key(Key::Type('a'), &v, Some(0), false, false), Effect::Pass);
        assert_eq!(on_key(Key::Type('1'), &v, Some(0), false, false), send("pick:0", Some(0)));
    }

    #[test]
    fn escape_gives_the_keyboard_back_unless_summoned_or_typing() {
        let v = view(false, true);
        assert_eq!(on_key(Key::Escape, &v, None, false, false), Effect::Release);
        assert_eq!(on_key(Key::Escape, &v, None, true, false), Effect::Pass, "clears the field first");
        assert_eq!(on_key(Key::Escape, &v, None, false, true), Effect::Pass);
    }

    #[test]
    fn the_cursor_starts_over_on_a_new_step_only() {
        let a = view(true, false);
        let mut checked = a.clone();
        if let Row::Choices { options, .. } = &mut checked.rows[0] {
            options[1].selected = true;
        }
        assert!(same_list(Some(&a), Some(&checked)));
        let next = ExpandedView { headline: Headline::Text("And?".into()), ..a.clone() };
        assert!(!same_list(Some(&a), Some(&next)));
        assert!(wants_keys(&a));
    }
}
