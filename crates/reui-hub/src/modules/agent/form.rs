//! An MCP server's form (Claude Code's `Elicitation`, PRD agent moments §5.6) as you fill
//! it in: one step per field, what's picked or typed in each, and whether it can be sent.
//! Required fields block Submit; a number that doesn't parse holds its step (the card says
//! so). `schema.rs` reads the fields, `answer.rs` writes the reply, `card.rs` draws it.
//! Pure.

use serde_json::{Map, Value};

use super::schema::{self, Field, Input};

#[derive(Debug)]
pub(super) struct Form {
    /// `None`: a schema the card can't show; only "Answer in terminal".
    pub fields: Option<Vec<Field>>,
    pub step: usize,
    /// Per field: the picked option (a choice's index; Yes 0, No 1).
    pub chosen: Vec<Option<usize>>,
    /// Per field: what's typed (the moment's field text is the current step's).
    pub texts: Vec<String>,
}

/// What an action on the form came to.
#[derive(Debug, PartialEq)]
pub(super) enum Step {
    Nothing,
    Changed,
    Submit(Map<String, Value>),
    Decline,
}

/// Yes / No, a boolean's two choices.
pub(super) const BOOL_LABELS: [&str; 2] = ["Yes", "No"];

impl Form {
    pub(super) fn new(schema: &Value) -> Self {
        let fields = schema::fields(schema);
        let all = fields.as_deref().unwrap_or_default();
        let chosen = all
            .iter()
            .map(|f| match (&f.input, &f.default) {
                (Input::Choice(options), Some(d)) => options.iter().position(|(v, _)| v == d),
                (Input::Bool, Some(Value::Bool(b))) => Some(if *b { 0 } else { 1 }),
                _ => None,
            })
            .collect();
        let texts = all.iter().map(|f| f.default.as_ref().filter(|_| matches!(f.input, Input::Text | Input::Number { .. })).map(schema::shown).unwrap_or_default()).collect();
        Self { fields, step: 0, chosen, texts }
    }

    pub(super) fn field(&self) -> Option<&Field> {
        self.fields.as_ref()?.get(self.step)
    }

    pub(super) fn count(&self) -> usize {
        self.fields.as_ref().map_or(0, Vec::len)
    }

    fn is_last(&self) -> bool {
        self.step + 1 >= self.count()
    }

    /// Field `i`'s value: `Err` when typed text doesn't parse, `Ok(None)` when empty.
    pub(super) fn value(&self, i: usize) -> Result<Option<Value>, ()> {
        let Some(f) = self.fields.as_ref().and_then(|f| f.get(i)) else { return Ok(None) };
        match &f.input {
            Input::Choice(options) => Ok(self.chosen[i].and_then(|c| options.get(c)).map(|(v, _)| v.clone())),
            Input::Bool => Ok(self.chosen[i].map(|c| Value::Bool(c == 0))),
            input => schema::parse(input, &self.texts[i]),
        }
    }

    /// Field `i` may be left: it has a value, or it's optional and empty.
    pub(super) fn ready(&self, i: usize) -> bool {
        match self.value(i) {
            Ok(Some(_)) => true,
            Ok(None) => !self.fields.as_ref().is_some_and(|f| f.get(i).is_some_and(|f| f.required)),
            Err(()) => false,
        }
    }

    /// Every field may be left: the form can be sent.
    pub(super) fn complete(&self) -> bool {
        (0..self.count()).all(|i| self.ready(i))
    }

    /// The button that moves on, when it may: Next, or Submit on the last step.
    pub(super) fn forward(&self) -> Option<&'static str> {
        match self.is_last() {
            false => self.ready(self.step).then_some("Next"),
            true => self.complete().then_some("Submit"),
        }
    }

    /// The answer: each field with a value, keyed by its property name.
    fn content(&self) -> Map<String, Value> {
        let fields = self.fields.as_deref().unwrap_or_default();
        fields.iter().enumerate().filter_map(|(i, f)| self.value(i).ok().flatten().map(|v| (f.key.clone(), v))).collect()
    }

    /// What's typed into the current step's field, as you type (so Submit shows when it
    /// may, and a number that doesn't parse says so at once).
    pub(super) fn typed(&mut self, text: &str) {
        if let Some(t) = self.texts.get_mut(self.step) {
            *t = text.to_string();
        }
    }

    /// Applies `action`; `text` is the moment's field (this step's typing), loaded with the
    /// new step's when the step changes.
    pub(super) fn act(&mut self, action: &str, text: &mut String) -> Step {
        if action == "decline" {
            return Step::Decline;
        }
        if self.fields.is_none() {
            return Step::Nothing;
        }
        if self.step < self.texts.len() {
            self.texts[self.step] = text.clone();
        }
        let to = match action {
            "back" if self.step > 0 => self.step - 1,
            "next" | "submit" if !self.ready(self.step) => return Step::Nothing,
            "next" | "submit" if self.is_last() => return if self.complete() { Step::Submit(self.content()) } else { Step::Nothing },
            "next" | "submit" => self.step + 1,
            pick => {
                let Some(i) = pick.strip_prefix("pick:").and_then(|i| i.parse::<usize>().ok()) else { return Step::Nothing };
                let options = match self.field().map(|f| &f.input) {
                    Some(Input::Choice(o)) => o.len(),
                    Some(Input::Bool) => BOOL_LABELS.len(),
                    _ => return Step::Nothing,
                };
                if i >= options {
                    return Step::Nothing;
                }
                self.chosen[self.step] = Some(i);
                // A pick answers its step; the last one waits for Submit.
                if self.is_last() {
                    return Step::Changed;
                }
                self.step + 1
            }
        };
        self.step = to;
        *text = self.texts.get(to).cloned().unwrap_or_default();
        Step::Changed
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn jira() -> Form {
        Form::new(&json!({ "type": "object", "properties": {
            "priority": { "type": "string", "enum": ["low", "high"], "default": "low" },
            "notify": { "type": "boolean" },
            "key": { "type": "string", "title": "Project key" },
            "points": { "type": "integer" }
        }, "required": ["key", "notify"] }))
    }

    #[test]
    fn a_form_steps_through_its_fields_and_submits_their_values() {
        let mut f = jira();
        let mut text = String::new();
        assert_eq!(f.chosen[0], Some(0), "the default is picked");
        assert_eq!(f.forward(), Some("Next"));
        assert_eq!(f.act("next", &mut text), Step::Changed);
        assert_eq!(f.forward(), None, "required, nothing picked");
        assert_eq!(f.act("next", &mut text), Step::Nothing);
        assert_eq!(f.act("pick:1", &mut text), Step::Changed, "No");
        assert_eq!(f.step, 2);
        assert_eq!(f.act("submit", &mut text), Step::Nothing, "a required field is empty");
        text = "DOCS".into();
        assert_eq!(f.act("submit", &mut text), Step::Changed);
        assert_eq!((f.step, text.as_str()), (3, ""));
        // A number that doesn't parse holds the step; empty is fine for an optional one.
        text = "four".into();
        assert_eq!(f.act("submit", &mut text), Step::Nothing);
        assert!(!f.ready(3) && f.forward().is_none());
        // Back keeps what was typed.
        assert_eq!(f.act("back", &mut text), Step::Changed);
        assert_eq!(text, "DOCS");
        f.act("next", &mut text);
        text = "5".into();
        let Step::Submit(content) = f.act("submit", &mut text) else { panic!() };
        assert_eq!(Value::Object(content), json!({ "priority": "low", "notify": false, "key": "DOCS", "points": 5 }));
    }

    #[test]
    fn optional_fields_may_stay_empty_and_decline_always_works() {
        let mut f = Form::new(&json!({ "type": "object", "properties": { "note": { "type": "string" } } }));
        let mut text = String::new();
        assert_eq!(f.forward(), Some("Submit"));
        assert_eq!(f.act("submit", &mut text), Step::Submit(Map::new()));
        assert_eq!(f.act("decline", &mut text), Step::Decline);
        let mut empty = Form::new(&json!({ "type": "object" }));
        assert_eq!(empty.act("next", &mut text), Step::Submit(Map::new()), "nothing to fill");
        let mut nested = Form::new(&json!({ "type": "object", "properties": { "a": { "type": "object" } } }));
        assert!(nested.fields.is_none());
        assert_eq!(nested.act("submit", &mut text), Step::Nothing);
        assert_eq!(nested.act("decline", &mut text), Step::Decline);
    }

    #[test]
    fn the_last_pick_waits_for_submit() {
        let mut f = Form::new(&json!({ "type": "object", "properties": { "env": { "type": "string", "oneOf": [{ "const": "prod", "title": "Production" }] } }, "required": ["env"] }));
        let mut text = String::new();
        assert_eq!(f.forward(), None);
        assert_eq!(f.act("pick:3", &mut text), Step::Nothing);
        assert_eq!(f.act("pick:0", &mut text), Step::Changed);
        assert_eq!(f.forward(), Some("Submit"));
        assert_eq!(f.act("next", &mut text), Step::Submit(json!({ "env": "prod" }).as_object().unwrap().clone()));
    }
}
