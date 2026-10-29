//! The moments waiting for you (agent moments PRD §5): permission requests, questions
//! (Claude's and OpenCode's) and Claude's plans, one at a time. Each keeps its own state: a question's step and the
//! answers so far, the text typed into its field, and whether that field is open (deny
//! with a message, a plan's feedback). Pure: `queue.rs` orders them, the module owns the
//! bridges' reply channels, `answer.rs` the JSON, `card.rs` the look.

use std::time::Duration;

use serde_json::Value;
use tokio::time::Instant;

use super::answer::{self, Permission};

/// Hub < bridge (110 s) < hook timeout (120 s): the agent always gets a clean "no decision".
pub(super) const PERMISSION_WAIT: Duration = Duration::from_secs(100);
/// Questions and plans: hub < bridge (290 s) < hook timeout (300 s). Shorter than a
/// person might take, because Claude's own dialog may wait for this hook (PRD V1).
pub(super) const ASK_WAIT: Duration = Duration::from_secs(280);

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Opt {
    pub label: String,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Question {
    pub text: String,
    pub header: String,
    pub options: Vec<Opt>,
    pub multi: bool,
    /// A typed answer is allowed (Claude: always; OpenCode: `custom`, true by default).
    pub custom: bool,
}

/// Who asked, which decides how the answers go back.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Asker {
    /// AskUserQuestion: answers keyed by question text in the tool's input.
    Claude,
    /// OpenCode's `question.asked`: one list of labels per question, in order.
    OpenCode,
}

#[derive(Debug)]
pub(super) enum Kind {
    Permission { tool: String, suggestion: Option<Value> },
    /// `picked[step][option]`; `typed[step]` is a typed answer, which wins over picks.
    Question { questions: Vec<Question>, step: usize, picked: Vec<Vec<bool>>, typed: Vec<Option<String>>, asker: Asker },
    Plan,
}

#[derive(Debug)]
pub(super) struct Moment {
    /// The hook call's id: actions name it, and `HookGone` refers to it.
    pub id: u64,
    pub session: String,
    pub tool_use_id: Option<String>,
    /// The tool's input as received (answers echo it).
    pub input: Value,
    pub kind: Kind,
    /// A bridge waits for the answer. Without one it's read-only (older hooks).
    pub waiting: bool,
    /// The field is open instead of the buttons: deny with a message, or a plan's feedback.
    pub explaining: bool,
    /// What's typed in the field (the question's "Type an answer", or the explanation).
    pub text: String,
    pub deadline: Instant,
}

/// What an action on a moment came to.
#[derive(Debug, PartialEq)]
pub(super) enum Outcome {
    /// Not for this moment, or not yet possible (Next with nothing picked).
    Nothing,
    /// Its card changes (a step, a check, the field).
    Changed,
    /// Settled: this goes to the bridge (`None`: no decision, the agent asks itself).
    Reply(Option<String>),
    /// A read-only moment put away.
    Dismissed,
}

impl Moment {
    pub(super) fn permission(id: u64, session: &str, tool: &str, input: Value, suggestion: Option<Value>, now: Instant) -> Self {
        Self::new(id, session, None, input, Kind::Permission { tool: tool.into(), suggestion }, true, now + PERMISSION_WAIT)
    }

    /// AskUserQuestion or ExitPlanMode from `PreToolUse`; `waiting` when it came through the
    /// blocking hook. `None` for other tools.
    pub(super) fn ask(id: u64, session: &str, tool: &str, tool_use_id: Option<String>, input: Value, waiting: bool, now: Instant) -> Option<Self> {
        let kind = match tool {
            "AskUserQuestion" => Kind::question(parse_questions(&input, "multiSelect", None), Asker::Claude),
            "ExitPlanMode" => Kind::Plan,
            _ => return None,
        };
        Some(Self::new(id, session, tool_use_id, input, kind, waiting, now + ASK_WAIT))
    }

    /// OpenCode's `question.asked` (`questions[]`: `question`, `header`, `options[]`,
    /// `multiple`, `custom`); `waiting` when the plugin can send the answer back.
    pub(super) fn opencode_question(id: u64, session: &str, input: Value, waiting: bool, now: Instant) -> Self {
        let kind = Kind::question(parse_questions(&input, "multiple", Some("custom")), Asker::OpenCode);
        Self::new(id, session, None, input, kind, waiting, now + ASK_WAIT)
    }

    fn new(id: u64, session: &str, tool_use_id: Option<String>, input: Value, kind: Kind, waiting: bool, deadline: Instant) -> Self {
        Self { id, session: session.into(), tool_use_id, input, kind, waiting, explaining: false, text: String::new(), deadline }
    }

    /// Whether it can be answered here: a bridge waits, and a question is one we can answer
    /// faithfully (answers are keyed by question text, so two alike can't be, E-A7).
    pub(super) fn answerable(&self) -> bool {
        self.waiting
            && match &self.kind {
                Kind::Question { questions, asker: Asker::Claude, .. } => {
                    let mut texts: Vec<&str> = questions.iter().map(|q| q.text.as_str()).collect();
                    texts.sort_unstable();
                    texts.dedup();
                    !questions.is_empty() && texts.len() == questions.len() && questions.iter().all(|q| !q.options.is_empty())
                }
                // Answers go by position, so alike texts are fine; each question needs
                // something to pick or room to type.
                Kind::Question { questions, asker: Asker::OpenCode, .. } => !questions.is_empty() && questions.iter().all(|q| !q.options.is_empty() || q.custom),
                _ => true,
            }
    }

    /// The plan, for ExitPlanMode (`plan` as Claude Code injects it).
    pub(super) fn plan(&self) -> &str {
        self.input.get("plan").and_then(Value::as_str).unwrap_or_default()
    }

    /// Applies an action (the part after the moment's id: `allow`, `pick:2`, `alt:deny`…;
    /// `submit` is Enter in the field).
    pub(super) fn act(&mut self, action: &str) -> Outcome {
        // Alt only means something on Deny and Keep planning (the field); elsewhere it's a
        // plain press.
        let action = match action {
            "alt:deny" | "alt:keep" => action,
            other => other.strip_prefix("alt:").unwrap_or(other),
        };
        if action == "terminal" {
            return if self.waiting { Outcome::Reply(None) } else { Outcome::Dismissed };
        }
        if !self.answerable() {
            return Outcome::Nothing;
        }
        let text = self.text.trim().to_string();
        match (&mut self.kind, action) {
            (_, "cancel") if self.explaining => {
                self.explaining = false;
                self.text.clear();
                Outcome::Changed
            }
            (Kind::Permission { .. } | Kind::Plan, "alt:deny" | "keep" | "alt:keep") if !self.explaining => {
                self.explaining = true;
                self.text.clear();
                Outcome::Changed
            }
            (Kind::Permission { .. }, "send" | "submit") if self.explaining => match text.is_empty() {
                true => Outcome::Nothing,
                false => Outcome::Reply(Some(answer::permission(Permission::Deny(text)))),
            },
            (Kind::Permission { .. }, _) if self.explaining => Outcome::Nothing,
            (Kind::Permission { suggestion, .. }, "always") => Outcome::Reply(Some(answer::permission(match suggestion.take() {
                Some(s) => Permission::Always(s),
                None => Permission::Allow,
            }))),
            (Kind::Permission { .. }, "allow") => Outcome::Reply(Some(answer::permission(Permission::Allow))),
            (Kind::Permission { .. }, "deny") => Outcome::Reply(Some(answer::permission(Permission::Deny(answer::DENIED.into())))),
            (Kind::Plan, "send" | "submit") if self.explaining => match text.is_empty() {
                true => Outcome::Nothing,
                false => Outcome::Reply(Some(answer::plan_revised(&text))),
            },
            (Kind::Plan, "approve") if !self.explaining => Outcome::Reply(Some(answer::plan_approved(&self.input))),
            (Kind::Question { questions, step, picked, typed, asker }, action) => {
                let s = *step;
                let advance = |step: &mut usize| {
                    *step += 1;
                    *step == questions.len()
                };
                let done = match action {
                    "back" if s > 0 => {
                        *step -= 1;
                        false
                    }
                    "next" | "send" if picked[s].iter().any(|p| *p) || typed[s].is_some() => advance(step),
                    "submit" if !text.is_empty() && questions[s].custom => {
                        typed[s] = Some(text);
                        picked[s].iter_mut().for_each(|p| *p = false);
                        self.text.clear();
                        advance(step)
                    }
                    pick => match pick.strip_prefix("pick:").and_then(|i| i.parse::<usize>().ok()).filter(|&i| i < picked[s].len()) {
                        Some(i) if questions[s].multi => {
                            picked[s][i] = !picked[s][i];
                            typed[s] = None;
                            false
                        }
                        Some(i) => {
                            picked[s] = (0..picked[s].len()).map(|j| j == i).collect();
                            typed[s] = None;
                            advance(step)
                        }
                        None => return Outcome::Nothing,
                    },
                };
                if done {
                    let each = questions.iter().zip(picked.iter()).zip(typed.iter());
                    Outcome::Reply(Some(match asker {
                        Asker::Claude => answer::questions(&self.input, &each.map(|((q, p), t)| (q.text.clone(), answer_of(q, p, t))).collect::<Vec<_>>()),
                        Asker::OpenCode => answer::opencode_questions(&each.map(|((q, p), t)| labels_of(q, p, t)).collect::<Vec<_>>()),
                    }))
                } else {
                    // A new step's field starts empty.
                    if *step != s {
                        self.text.clear();
                    }
                    Outcome::Changed
                }
            }
            _ => Outcome::Nothing,
        }
    }
}

/// One question's answer: what was typed, else the picked labels joined with ", " (as
/// Claude joins multi-select answers).
fn answer_of(q: &Question, picked: &[bool], typed: &Option<String>) -> String {
    match typed {
        Some(t) => t.clone(),
        None => q.options.iter().zip(picked).filter(|(_, on)| **on).map(|(o, _)| o.label.as_str()).collect::<Vec<_>>().join(", "),
    }
}

/// One question's answer for OpenCode: the picked labels, or what was typed.
fn labels_of(q: &Question, picked: &[bool], typed: &Option<String>) -> Vec<String> {
    match typed {
        Some(t) => vec![t.clone()],
        None => q.options.iter().zip(picked).filter(|(_, on)| **on).map(|(o, _)| o.label.clone()).collect(),
    }
}

impl Kind {
    fn question(questions: Vec<Question>, asker: Asker) -> Self {
        Kind::Question { step: 0, picked: questions.iter().map(|q| vec![false; q.options.len()]).collect(), typed: vec![None; questions.len()], questions, asker }
    }
}

/// `questions[]`: `question`, `header`, `options[]` (`label`, `description`), whether
/// several may be picked (`multi`: Claude's `multiSelect`, OpenCode's `multiple`) and
/// whether one may be typed (`custom`: OpenCode's flag, true when absent; Claude always
/// takes one). What's missing reads as empty, so a malformed input is a question we can't
/// answer rather than an error.
fn parse_questions(input: &Value, multi: &str, custom: Option<&str>) -> Vec<Question> {
    let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().trim().to_string();
    let Some(list) = input.get("questions").and_then(Value::as_array) else { return Vec::new() };
    list.iter()
        .map(|q| Question {
            text: s(q, "question"),
            header: s(q, "header"),
            multi: q.get(multi).and_then(Value::as_bool).unwrap_or(false),
            custom: custom.and_then(|k| q.get(k)).and_then(Value::as_bool).unwrap_or(true),
            options: q
                .get("options")
                .and_then(Value::as_array)
                .map(|o| o.iter().map(|o| Opt { label: s(o, "label"), detail: Some(s(o, "description")).filter(|d| !d.is_empty()) }).filter(|o| !o.label.is_empty()).collect())
                .unwrap_or_default(),
        })
        .collect()
}

#[cfg(test)]
pub(super) mod tests {
    use serde_json::json;

    use super::*;

    fn questions() -> Value {
        json!({ "questions": [
            { "question": "Which framework?", "header": "Framework", "options": [{ "label": "Astro", "description": "Static" }, { "label": "SvelteKit", "description": "" }], "multiSelect": false },
            { "question": "Which features?", "header": "Features", "options": [{ "label": "Blog" }, { "label": "Search" }, { "label": "Dark mode" }], "multiSelect": true },
            { "question": "Deploy?", "header": "Release", "options": [{ "label": "Yes" }, { "label": "No" }] }
        ] })
    }

    pub(in super::super) fn ask(id: u64, waiting: bool) -> Moment {
        Moment::ask(id, "s1", "AskUserQuestion", Some("toolu_1".into()), questions(), waiting, Instant::now()).unwrap()
    }

    fn answers(out: &Outcome) -> Value {
        let Outcome::Reply(Some(json)) = out else { panic!("not answered: {out:?}") };
        serde_json::from_str::<Value>(json).unwrap()["hookSpecificOutput"]["updatedInput"]["answers"].clone()
    }

    #[test]
    fn questions_step_through_and_answer_once_at_the_end() {
        let mut m = ask(1, true);
        assert_eq!(m.act("next"), Outcome::Nothing, "nothing picked yet");
        assert_eq!(m.act("pick:1"), Outcome::Changed, "one pick answers the step");
        // Several to pick: checks toggle, Next moves on.
        assert_eq!(m.act("pick:0"), Outcome::Changed);
        assert_eq!(m.act("pick:2"), Outcome::Changed);
        assert_eq!(m.act("pick:0"), Outcome::Changed);
        assert_eq!(m.act("pick:1"), Outcome::Changed);
        // Back keeps the answers; Next goes on again.
        assert_eq!(m.act("back"), Outcome::Changed);
        assert!(matches!(&m.kind, Kind::Question { step: 0, picked, .. } if picked[0] == [false, true]));
        assert_eq!(m.act("next"), Outcome::Changed);
        assert_eq!(m.act("next"), Outcome::Changed);
        // The last step is typed.
        m.text = "  Not yet ".into();
        let out = m.act("submit");
        assert_eq!(answers(&out), json!({ "Which framework?": "SvelteKit", "Which features?": "Search, Dark mode", "Deploy?": "Not yet" }));
    }

    #[test]
    fn the_terminal_is_always_one_click() {
        assert_eq!(ask(1, true).act("terminal"), Outcome::Reply(None));
        assert_eq!(ask(1, false).act("terminal"), Outcome::Dismissed);
        assert_eq!(ask(1, false).act("pick:0"), Outcome::Nothing, "read-only");
    }

    #[test]
    fn questions_alike_in_text_are_answered_in_the_terminal_only() {
        let input = json!({ "questions": [{ "question": "Which?", "options": [{ "label": "A" }] }, { "question": "Which?", "options": [{ "label": "B" }] }] });
        let mut m = Moment::ask(1, "s1", "AskUserQuestion", None, input, true, Instant::now()).unwrap();
        assert!(!m.answerable());
        assert_eq!(m.act("pick:0"), Outcome::Nothing);
        assert_eq!(m.act("terminal"), Outcome::Reply(None));
        assert!(!Moment::ask(1, "s1", "AskUserQuestion", None, json!({}), true, Instant::now()).unwrap().answerable());
    }

    #[test]
    fn opencode_questions_answer_by_position() {
        let input = json!({ "questions": [
            { "question": "Which?", "header": "Pick", "options": [{ "label": "A", "description": "first" }, { "label": "B", "description": "" }] },
            { "question": "Which?", "header": "Several", "options": [{ "label": "X", "description": "" }, { "label": "Y", "description": "" }], "multiple": true },
            { "question": "Name it", "header": "Name", "options": [], "custom": true },
            { "question": "Sure?", "header": "Confirm", "options": [{ "label": "Yes", "description": "" }], "custom": false }
        ] });
        let mut m = Moment::opencode_question(1, "s1", input, true, Instant::now());
        assert!(m.answerable(), "alike texts are fine: answers go by position");
        assert_eq!(m.act("pick:1"), Outcome::Changed);
        m.act("pick:0");
        m.act("pick:1");
        assert_eq!(m.act("next"), Outcome::Changed);
        m.text = "Island".into();
        assert_eq!(m.act("submit"), Outcome::Changed);
        m.text = "maybe".into();
        assert_eq!(m.act("submit"), Outcome::Nothing, "no typed answer here");
        let Outcome::Reply(Some(out)) = m.act("pick:0") else { panic!() };
        assert_eq!(serde_json::from_str::<Value>(&out).unwrap()["hookSpecificOutput"]["answers"], json!([["B"], ["X", "Y"], ["Island"], ["Yes"]]));
        let nothing = Moment::opencode_question(2, "s1", json!({ "questions": [{ "question": "?", "options": [], "custom": false }] }), true, Instant::now());
        assert!(!nothing.answerable());
        assert!(!Moment::opencode_question(3, "s1", json!({ "questions": [] }), true, Instant::now()).answerable());
    }

    #[test]
    fn deny_with_a_message_opens_the_field_first() {
        let mut m = Moment::permission(1, "s1", "Bash", json!({ "command": "rm -rf target" }), None, Instant::now());
        assert_eq!(m.act("alt:deny"), Outcome::Changed);
        assert!(m.explaining);
        assert_eq!(m.act("allow"), Outcome::Nothing, "the field is up");
        assert_eq!(m.act("submit"), Outcome::Nothing, "nothing typed");
        m.text = "Use cargo clean".into();
        let Outcome::Reply(Some(out)) = m.act("send") else { panic!() };
        assert_eq!(serde_json::from_str::<Value>(&out).unwrap()["hookSpecificOutput"]["decision"], json!({ "behavior": "deny", "message": "Use cargo clean" }));
        let mut plain = Moment::permission(2, "s1", "Bash", json!({}), Some(json!({ "type": "addRules" })), Instant::now());
        assert!(matches!(plain.act("always"), Outcome::Reply(Some(s)) if s.contains("updatedPermissions")));
        let mut cancelled = Moment::permission(3, "s1", "Bash", json!({}), None, Instant::now());
        cancelled.act("alt:deny");
        assert_eq!(cancelled.act("cancel"), Outcome::Changed);
        assert!(matches!(cancelled.act("deny"), Outcome::Reply(Some(s)) if s.contains(answer::DENIED)));
    }

    #[test]
    fn plans_are_approved_or_sent_back_with_feedback() {
        let input = json!({ "plan": "# Plan\n1. Do it", "planFilePath": "plan.md" });
        let mut m = Moment::ask(1, "s1", "ExitPlanMode", None, input.clone(), true, Instant::now()).unwrap();
        assert!(matches!(m.act("approve"), Outcome::Reply(Some(s)) if s.contains("\"allow\"")));
        assert_eq!(m.act("keep"), Outcome::Changed);
        assert_eq!(m.act("approve"), Outcome::Nothing, "the field is up");
        m.text = "Keep the old API".into();
        assert!(matches!(m.act("submit"), Outcome::Reply(Some(s)) if s.contains("Keep the old API") && s.contains("\"deny\"")));
        assert_eq!(m.plan(), "# Plan\n1. Do it");
    }

}
