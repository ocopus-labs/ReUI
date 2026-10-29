//! An MCP server's `requested_schema` (Claude Code's `Elicitation` hook) as the fields the
//! island can ask for one at a time (PRD agent moments §5.6), or `None` when it can't show
//! them faithfully and the card offers only "Answer in terminal".
//!
//! MCP keeps elicitation schemas flat: an object whose properties are primitives (string,
//! number, integer, boolean) or a string enum (`enum` + legacy `enumNames`, or `oneOf` of
//! `const` + `title`). Anything else (a nested object, an array, a multi-select) is beyond
//! a card, as is a form longer than six fields. Typed text is checked here too, so a number
//! that doesn't parse never reaches the server. Pure.

use serde_json::{Map, Number, Value};

/// More fields than this are better answered in the terminal than stepped through.
const MAX_FIELDS: usize = 6;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Input {
    /// One of these: (the value sent, its label).
    Choice(Vec<(Value, String)>),
    /// Yes / No.
    Bool,
    Text,
    Number { integer: bool },
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Field {
    /// The property's name: the answer's key.
    pub key: String,
    /// What the card calls it (`title`, else the key).
    pub title: String,
    pub description: Option<String>,
    pub input: Input,
    pub required: bool,
    /// The schema's `default`, when it's a value this field can take.
    pub default: Option<Value>,
}

/// The schema's fields in its order, or `None` when the card can't show it.
pub(super) fn fields(schema: &Value) -> Option<Vec<Field>> {
    let schema = schema.as_object()?;
    if schema.get("type").is_some_and(|t| t != "object") {
        return None;
    }
    let empty = Map::new();
    let properties = match schema.get("properties") {
        Some(p) => p.as_object()?,
        None => &empty,
    };
    if properties.len() > MAX_FIELDS {
        return None;
    }
    let required: Vec<&str> = schema.get("required").and_then(Value::as_array).map(|r| r.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    properties.iter().map(|(key, p)| field(key, p, required.contains(&key.as_str()))).collect()
}

fn field(key: &str, p: &Value, required: bool) -> Option<Field> {
    let text = |k: &str| p.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
    let input = match (p.get("type").and_then(Value::as_str), choices(p)) {
        (Some("string") | None, Some(options)) => Input::Choice(options),
        (Some("string"), None) => Input::Text,
        (Some("number"), None) => Input::Number { integer: false },
        (Some("integer"), None) => Input::Number { integer: true },
        (Some("boolean"), None) => Input::Bool,
        _ => return None,
    };
    let default = p.get("default").cloned().filter(|d| fits(&input, d));
    Some(Field { key: key.into(), title: text("title").unwrap_or_else(|| key.into()), description: text("description"), input, required, default })
}

/// A single-select's options: `enum` (labels from `enumNames` when given) or `oneOf` with
/// `const` / `title`. `None` when the property isn't one.
fn choices(p: &Value) -> Option<Vec<(Value, String)>> {
    let label = |v: &Value| v.as_str().map_or_else(|| v.to_string(), str::to_owned);
    if let Some(values) = p.get("enum").and_then(Value::as_array) {
        let names = p.get("enumNames").and_then(Value::as_array);
        return Some(values.iter().enumerate().map(|(i, v)| (v.clone(), names.and_then(|n| n.get(i)).and_then(Value::as_str).map_or_else(|| label(v), str::to_owned))).collect());
    }
    let one_of = p.get("oneOf").and_then(Value::as_array)?;
    one_of.iter().map(|o| o.get("const").map(|c| (c.clone(), o.get("title").and_then(Value::as_str).map_or_else(|| label(c), str::to_owned)))).collect()
}

fn fits(input: &Input, value: &Value) -> bool {
    match input {
        Input::Choice(options) => options.iter().any(|(v, _)| v == value),
        Input::Bool => value.is_boolean(),
        Input::Text => value.is_string(),
        Input::Number { integer: true } => value.is_i64() || value.is_u64(),
        Input::Number { integer: false } => value.is_number(),
    }
}

/// What was typed into a text or number field, as the value to send; `Err` when a number
/// doesn't parse (the card says so quietly). Empty text is no value.
pub(super) fn parse(input: &Input, typed: &str) -> Result<Option<Value>, ()> {
    let typed = typed.trim();
    if typed.is_empty() {
        return Ok(None);
    }
    match input {
        Input::Number { integer: true } => typed.parse::<i64>().map(|n| Some(Value::from(n))).map_err(|_| ()),
        Input::Number { integer: false } => typed.parse::<f64>().ok().and_then(Number::from_f64).map(|n| Some(Value::Number(n))).ok_or(()),
        _ => Ok(Some(Value::String(typed.into()))),
    }
}

/// A value as the text it shows in the field (a default, or what was typed before Back).
pub(super) fn shown(value: &Value) -> String {
    value.as_str().map_or_else(|| value.to_string(), str::to_owned)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn flat_schemas_become_fields_in_order() {
        // The shape the MCP spec documents, with each kind the card can ask for.
        let schema = json!({
            "type": "object",
            "properties": {
                "project": { "type": "string", "title": "Project key", "description": "Like ABC" },
                "priority": { "type": "string", "enum": ["low", "high"], "enumNames": ["Low", "High"], "default": "low" },
                "board": { "type": "string", "oneOf": [{ "const": "b1", "title": "Main" }, { "const": "b2", "title": "Ops" }] },
                "notify": { "type": "boolean", "default": true },
                "points": { "type": "integer", "minimum": 0 },
                "budget": { "type": "number", "default": "lots" }
            },
            "required": ["project", "points"]
        });
        let f = fields(&schema).unwrap();
        assert_eq!(f.iter().map(|f| f.key.as_str()).collect::<Vec<_>>(), ["project", "priority", "board", "notify", "points", "budget"]);
        assert_eq!((f[0].title.as_str(), f[0].description.as_deref(), f[0].required, &f[0].input), ("Project key", Some("Like ABC"), true, &Input::Text));
        assert_eq!(f[1].input, Input::Choice(vec![(json!("low"), "Low".into()), (json!("high"), "High".into())]));
        assert_eq!((f[1].title.as_str(), f[1].default.clone()), ("priority", Some(json!("low"))));
        assert_eq!(f[2].input, Input::Choice(vec![(json!("b1"), "Main".into()), (json!("b2"), "Ops".into())]));
        assert_eq!((&f[3].input, f[3].default.clone(), f[3].required), (&Input::Bool, Some(json!(true)), false));
        assert_eq!(f[4].input, Input::Number { integer: true });
        assert_eq!(f[5].default, None, "a default the field can't take is dropped");
        assert_eq!(fields(&json!({ "type": "object" })), Some(Vec::new()), "nothing to fill: Submit alone");
    }

    #[test]
    fn what_a_card_cant_show_goes_to_the_terminal() {
        let one = |p: Value| fields(&json!({ "type": "object", "properties": { "x": p } }));
        assert_eq!(one(json!({ "type": "object", "properties": {} })), None, "nested");
        assert_eq!(one(json!({ "type": "array", "items": { "type": "string", "enum": ["a"] } })), None, "multi-select");
        assert_eq!(one(json!({ "type": "null" })), None);
        assert_eq!(fields(&json!({ "type": "array" })), None);
        assert_eq!(fields(&json!("form")), None);
        let seven: Map<String, Value> = (0..7).map(|i| (format!("f{i}"), json!({ "type": "string" }))).collect();
        assert_eq!(fields(&json!({ "type": "object", "properties": seven })), None, "longer than six");
    }

    #[test]
    fn typed_numbers_parse_or_say_so() {
        let int = Input::Number { integer: true };
        assert_eq!(parse(&int, " 42 "), Ok(Some(json!(42))));
        assert_eq!(parse(&int, "4.5"), Err(()));
        assert_eq!(parse(&Input::Number { integer: false }, "4.5"), Ok(Some(json!(4.5))));
        assert_eq!(parse(&Input::Number { integer: false }, "four"), Err(()));
        assert_eq!(parse(&Input::Text, "  ABC "), Ok(Some(json!("ABC"))));
        assert_eq!(parse(&int, "   "), Ok(None), "empty is no value, not an error");
        assert_eq!((shown(&json!("ABC")), shown(&json!(3))), ("ABC".to_string(), "3".to_string()));
    }
}
