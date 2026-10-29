//! The keyboard in a card that asks something (a choice list, an answer field), once a
//! click has given it the keyboard. Pure, so the rules are testable without a window;
//! `pill::summon` applies what it decides.
//!
//! A card that arrives by itself never gets here: a stray Enter while typing in an editor
//! must not answer anything (agent moments PRD §5.7).

use reui_protocol::{Button, ButtonRole, Choice, ExpandedView, ItemKey, Row, ACTION_ALT};

use crate::features::keys::Key;

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

/// Whether a card can use the keyboard at all: it has a choice list or a field, or it's a
/// moment (a session strip) with buttons to press (Enter for the main one, Shift+Enter for
/// the second meaning of its first: deny and say why).
pub fn wants_keys(view: &ExpandedView) -> bool {
    view.rows.iter().any(|r| matches!(r, Row::Choices { .. } | Row::Search { .. }) || (view.context.is_some() && matches!(r, Row::Buttons(b) if !b.is_empty())))
}

fn list(view: &ExpandedView) -> Option<(&[Choice], bool)> {
    view.rows.iter().find_map(|r| match r {
        Row::Choices { options, multi } => Some((options.as_slice(), *multi)),
        _ => None,
    })
}

fn buttons(view: &ExpandedView) -> &[Button] {
    view.rows.iter().rev().find_map(|r| if let Row::Buttons(b) = r { Some(b.as_slice()) } else { None }).unwrap_or_default()
}

/// The card's main button, which Enter presses when no single choice is under the cursor:
/// the `Primary` one, else the accent-tinted one (an alert's preferred action).
fn primary(view: &ExpandedView) -> Option<String> {
    let b = buttons(view);
    b.iter().find(|b| b.role == ButtonRole::Primary).or_else(|| b.iter().find(|b| b.role == ButtonRole::Accent)).map(|b| b.action.clone())
}

/// What Shift+Enter sends in a moment: the first button that isn't `Plain` (the "no"
/// side, Deny), pressed for its second meaning.
fn second(view: &ExpandedView) -> Option<String> {
    view.context.as_ref()?;
    buttons(view).iter().find(|b| b.role != ButtonRole::Plain).map(|b| format!("{ACTION_ALT}{}", b.action))
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
    if key == Key::Item(ItemKey::ShiftEnter) && !typing {
        return second(view).map_or(pass_or_ignore, |action| Effect::Send { action, at: None });
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

/// The scroll that shows the span `top`..`bottom` (a choice, in list coordinates) in a
/// list window `shown` tall now scrolled by `scroll`: as little movement as it takes.
pub fn reveal(scroll: f32, shown: f32, (top, bottom): (f32, f32)) -> f32 {
    if top < scroll {
        top
    } else if bottom > scroll + shown {
        bottom - shown
    } else {
        scroll
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

    #[test]
    fn moments_take_enter_for_the_main_button_and_shift_enter_for_the_second_meaning() {
        let text = |action: &str, role| Button { action: action.into(), content: ButtonContent::Text(action.into()), role };
        let mut v = view(false, false);
        v.rows = vec![Row::Buttons(vec![text("7:deny", ButtonRole::Secondary), text("7:always", ButtonRole::Secondary), text("7:allow", ButtonRole::Accent)])];
        assert!(!wants_keys(&v), "buttons alone aren't enough outside a moment");
        v.context = Some(reui_protocol::Context { mark: Element::Empty, title: "ReUI".into(), facts: vec![] });
        assert!(wants_keys(&v));
        assert_eq!(on_key(Key::Activate, &v, None, false, false), send("7:allow", None), "the accent one is the main one");
        assert_eq!(on_key(Key::Item(ItemKey::ShiftEnter), &v, None, false, false), send("alt:7:deny", None));
        // A question's quiet Answer in terminal is never the one.
        v.rows = vec![Row::Buttons(vec![text("3:terminal", ButtonRole::Plain), text("3:back", ButtonRole::Secondary)])];
        assert_eq!(on_key(Key::Item(ItemKey::ShiftEnter), &v, None, false, false), send("alt:3:back", None));
    }

    #[test]
    fn the_cursor_scrolls_into_view_with_as_little_movement_as_it_takes() {
        assert_eq!(reveal(0.0, 100.0, (20.0, 60.0)), 0.0);
        assert_eq!(reveal(0.0, 100.0, (90.0, 140.0)), 40.0);
        assert_eq!(reveal(80.0, 100.0, (20.0, 60.0)), 20.0);
    }
}
