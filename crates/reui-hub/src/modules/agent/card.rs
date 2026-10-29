//! The card for the moment in front (PRD agent moments §5): the session strip, eyebrow →
//! hero → detail in the roomy width, then what it asks: a permission's body and Deny /
//! Always / Allow, a question's choices and its answer field, a plan's prose and Keep
//! planning / Approve, or the field for what to say instead. Several waiting: the pager on
//! the strip walks them and the compact pill shows how many. Every action names its moment
//! (`<id>:allow`), so a click never lands on another one that moved in front. Pure.

use reui_protocol::{BlockKind, Button, ButtonContent, ButtonRole, CardSize, Choice, Context, Element, ExpandedView, Headline, Pager, Row, Tone};
use std::path::Path;

use super::describe::describe_tool;
use super::moments::{Kind, Moment};
use super::plan;
use crate::modules::clipboard::secrets;

pub(super) const GLYPH_DONE: char = '\u{E73E}';
const GLYPH_DENY: char = '\u{E711}';
const GLYPH_PLAN: char = '\u{E8FD}';
const GLYPH_ANSWER: char = '\u{E8BD}';

/// The pager's actions over several moments (a question's steps use `<id>:back` / `next`).
pub(super) const PAGE_PREV: &str = "page:prev";
pub(super) const PAGE_NEXT: &str = "page:next";

/// The compact pill's trailing side and the card, for `moment` at `position` (index,
/// count) in the queue. `agent` is the agent's short name ("Claude"), for the field that
/// tells it what to do instead.
pub(super) fn card(moment: &Moment, context: Context, position: (usize, usize), agent: &str) -> (Element, ExpandedView) {
    let id = moment.id;
    let act = |verb: &str| format!("{id}:{verb}");
    let text = |verb: &str, label: &str, role| Button { action: act(verb), content: ButtonContent::Text(label.into()), role };
    let accent = |g| Element::Glyph { glyph: g, tone: Tone::Accent };
    let field = |placeholder: String| Row::Search { query: moment.text.clone(), buttons: Vec::new(), placeholder: Some(placeholder), glyph: Some(GLYPH_ANSWER) };
    let (index, count) = position;
    let mut pager = (count > 1).then(|| Pager { index: index as u32, count: count as u32, prev: PAGE_PREV.into(), next: PAGE_NEXT.into() });
    let answerable = moment.answerable();

    let (kind, leading, eyebrow, headline, detail, rows) = match &moment.kind {
        Kind::Permission { tool, suggestion } => {
            let described = describe_tool(tool, &moment.input);
            let mut rows = Vec::new();
            if let Some((body, kind)) = described.body {
                rows.push(Row::Block { text: body, kind });
            }
            if moment.explaining {
                rows.push(field(format!("Tell {agent} what to do instead")));
                rows.push(Row::Buttons(vec![text("cancel", "Cancel", ButtonRole::Secondary), text("send", "Deny", ButtonRole::Destructive)]));
            } else if rows.is_empty() && suggestion.is_none() {
                // Nothing more to say and no Always: two circles beside the hero (§5.8).
                let glyph = |verb: &str, g, role| Button { action: act(verb), content: ButtonContent::Glyph(g), role };
                rows.push(Row::Buttons(vec![glyph("deny", GLYPH_DENY, ButtonRole::Destructive), glyph("allow", GLYPH_DONE, ButtonRole::Accent)]));
            } else {
                let mut b = vec![text("deny", "Deny", ButtonRole::Secondary)];
                if suggestion.is_some() {
                    b.push(text("always", "Always", ButtonRole::Secondary));
                }
                b.push(text("allow", "Allow", ButtonRole::Accent));
                rows.push(Row::Buttons(b));
            }
            ("Allow?", accent(described.glyph), "Permission", described.headline, described.detail, rows)
        }
        Kind::Question { questions, step, picked, typed, .. } => {
            let step = (*step).min(questions.len().saturating_sub(1));
            let q = questions.get(step);
            let mut rows = Vec::new();
            if let Some(q) = q {
                let options = q
                    .options
                    .iter()
                    .zip(&picked[step])
                    .enumerate()
                    .map(|(i, (o, on))| Choice {
                        label: secrets::mask(&o.label),
                        detail: o.detail.as_deref().map(secrets::mask),
                        selected: *on,
                        // Read-only: shown, not clickable.
                        action: if answerable { act(&format!("pick:{i}")) } else { String::new() },
                    })
                    .collect();
                rows.push(Row::Choices { options, multi: q.multi });
            }
            let mut buttons = vec![text("terminal", "Answer in terminal", if answerable { ButtonRole::Plain } else { ButtonRole::Secondary })];
            let mut detail = q.map(|q| q.header.clone()).filter(|h| !h.is_empty());
            if answerable {
                // Claude always takes a typed answer ("Other"); OpenCode when `custom`.
                if q.is_some_and(|q| q.custom) {
                    rows.push(field("Type an answer".into()));
                }
                if questions.len() > 1 {
                    match pager {
                        // The strip's pager is the steps when nothing else waits.
                        None => pager = Some(Pager { index: step as u32, count: questions.len() as u32, prev: act("back"), next: act("next") }),
                        // Otherwise the step is a fact, and Back a button.
                        Some(_) => {
                            let at = format!("{} of {}", step + 1, questions.len());
                            detail = Some(detail.map_or(at.clone(), |d| format!("{d} · {at}")));
                            if step > 0 {
                                buttons.push(text("back", "Back", ButtonRole::Secondary));
                            }
                        }
                    }
                }
                let answered = picked[step].iter().any(|p| *p) || typed[step].is_some();
                if q.is_some_and(|q| q.multi) || answered {
                    let last = step + 1 == questions.len();
                    buttons.push(text("next", if last { "Send" } else { "Next" }, ButtonRole::Accent));
                }
            }
            rows.push(Row::Buttons(buttons));
            let hero = q.map_or_else(|| "Question".to_string(), |q| secrets::mask(&q.text));
            ("Question", Element::Empty, "Question", Headline::Text(hero), detail, rows)
        }
        Kind::Plan => {
            let plan = secrets::mask(moment.plan());
            let file = moment.input.get("planFilePath").and_then(|p| p.as_str()).and_then(|p| Path::new(p).file_name()).map(|f| f.to_string_lossy().into_owned());
            let steps = match plan::steps(&plan) {
                0 => None,
                1 => Some("1 step".to_string()),
                n => Some(format!("{n} steps")),
            };
            let detail = Some([file, steps].into_iter().flatten().collect::<Vec<_>>().join(" · ")).filter(|d| !d.is_empty());
            let mut rows = Vec::new();
            let prose = plan::prose(&plan);
            if !prose.is_empty() {
                rows.push(Row::Block { text: prose, kind: BlockKind::Prose });
            }
            if !answerable {
                rows.push(Row::Buttons(vec![text("terminal", "Answer in terminal", ButtonRole::Secondary)]));
            } else if moment.explaining {
                rows.push(field("What should change?".into()));
                rows.push(Row::Buttons(vec![text("cancel", "Cancel", ButtonRole::Secondary), text("send", "Send", ButtonRole::Accent)]));
            } else {
                rows.push(Row::Buttons(vec![text("keep", "Keep planning", ButtonRole::Secondary), text("approve", "Approve", ButtonRole::Accent)]));
            }
            ("Plan", accent(GLYPH_PLAN), "Plan", Headline::Text(plan::title(&plan).unwrap_or_else(|| "Plan".into())), detail, rows)
        }
    };
    // Several waiting: how many, beside the logo; one: what it is.
    let trailing = Element::Text { text: if count > 1 { count.to_string() } else { kind.into() }, tone: Tone::Accent };
    let view = ExpandedView { context: Some(context), leading, eyebrow: Some(eyebrow.into()), headline, detail, trailing: Element::Empty, rows, size: CardSize::Roomy, tint: false, pager, open: None };
    (trailing, view)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tokio::time::Instant;

    use super::*;

    fn context() -> Context {
        Context { mark: Element::Empty, title: "ReUI".into(), facts: vec!["fix the build".into()] }
    }

    fn buttons(view: &ExpandedView) -> Vec<(String, String)> {
        let Some(Row::Buttons(b)) = view.rows.last() else { panic!("no buttons") };
        b.iter().map(|b| (b.action.clone(), match &b.content { ButtonContent::Text(t) => t.clone(), ButtonContent::Glyph(g) => g.to_string() })).collect()
    }

    #[test]
    fn a_permission_card_names_its_moment_and_opens_a_field_to_explain() {
        let mut m = Moment::permission(7, "s", "Bash", json!({ "command": "set -e\ncargo test" }), Some(json!({})), Instant::now());
        let (trailing, view) = card(&m, context(), (0, 1), "Claude");
        assert_eq!(trailing, Element::Text { text: "Allow?".into(), tone: Tone::Accent });
        assert_eq!(buttons(&view), [("7:deny".into(), "Deny".into()), ("7:always".into(), "Always".into()), ("7:allow".into(), "Allow".into())]);
        assert!(matches!(view.rows[0], Row::Block { .. }));
        m.explaining = true;
        let (_, view) = card(&m, context(), (0, 1), "Claude");
        assert!(view.rows.iter().any(|r| matches!(r, Row::Search { placeholder: Some(p), .. } if p == "Tell Claude what to do instead")));
        assert_eq!(buttons(&view)[1].0, "7:send");
    }

    #[test]
    fn several_waiting_page_on_the_strip_and_count_in_the_pill() {
        let m = Moment::permission(7, "s", "Read", json!({ "file_path": "a.rs" }), None, Instant::now());
        let (trailing, view) = card(&m, context(), (1, 3), "Claude");
        assert_eq!(trailing, Element::Text { text: "3".into(), tone: Tone::Accent });
        assert_eq!(view.pager, Some(Pager { index: 1, count: 3, prev: PAGE_PREV.into(), next: PAGE_NEXT.into() }));
    }

    #[test]
    fn a_question_steps_on_the_pager_unless_other_moments_wait() {
        let input = json!({ "questions": [
            { "question": "Which?", "header": "Framework", "options": [{ "label": "A", "description": "first" }, { "label": "B" }] },
            { "question": "And?", "header": "Extras", "options": [{ "label": "X" }], "multiSelect": true }
        ] });
        let mut m = Moment::ask(3, "s", "AskUserQuestion", None, input, true, Instant::now()).unwrap();
        let (_, view) = card(&m, context(), (0, 1), "Claude");
        assert_eq!(view.pager.as_ref().map(|p| (p.index, p.count, p.next.as_str())), Some((0, 2, "3:next")));
        assert_eq!(view.detail.as_deref(), Some("Framework"));
        let Row::Choices { options, multi: false } = &view.rows[0] else { panic!() };
        assert_eq!((options[0].action.as_str(), options[0].detail.as_deref()), ("3:pick:0", Some("first")));
        assert_eq!(buttons(&view), [("3:terminal".into(), "Answer in terminal".into())], "one pick answers");
        m.act("pick:1");
        let (_, view) = card(&m, context(), (0, 2), "Claude");
        assert_eq!(view.detail.as_deref(), Some("Extras · 2 of 2"));
        assert_eq!(buttons(&view).iter().map(|b| b.1.as_str()).collect::<Vec<_>>(), ["Answer in terminal", "Back", "Send"]);
    }

    #[test]
    fn read_only_questions_show_their_options_and_the_terminal() {
        let input = json!({ "questions": [{ "question": "Which?", "options": [{ "label": "A" }] }] });
        let m = Moment::ask(3, "s", "AskUserQuestion", None, input, false, Instant::now()).unwrap();
        let (_, view) = card(&m, context(), (0, 1), "Claude");
        assert!(matches!(&view.rows[0], Row::Choices { options, .. } if options[0].action.is_empty()));
        assert!(!view.rows.iter().any(|r| matches!(r, Row::Search { .. })));
        assert_eq!(buttons(&view), [("3:terminal".into(), "Answer in terminal".into())]);
    }

    #[test]
    fn a_plan_card_has_its_title_steps_and_prose() {
        let input = json!({ "plan": "# Add the launcher\n\n1. Parse\n2. Show\n", "planFilePath": "C:/Users/me/.claude/plans/launcher.md" });
        let mut m = Moment::ask(4, "s", "ExitPlanMode", None, input, true, Instant::now()).unwrap();
        let (trailing, view) = card(&m, context(), (0, 1), "Claude");
        assert_eq!(trailing, Element::Text { text: "Plan".into(), tone: Tone::Accent });
        assert_eq!((view.headline, view.detail.as_deref()), (Headline::Text("Add the launcher".into()), Some("launcher.md · 2 steps")));
        assert_eq!(view.rows[0], Row::Block { text: "1. Parse\n2. Show".into(), kind: BlockKind::Prose });
        m.act("keep");
        let (_, view) = card(&m, context(), (0, 1), "Claude");
        assert!(view.rows.iter().any(|r| matches!(r, Row::Search { placeholder: Some(p), .. } if p == "What should change?")));
    }
}
