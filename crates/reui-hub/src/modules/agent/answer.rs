//! The JSON the bridge prints back to the agent for each kind of moment, exactly in the
//! shapes Claude Code documents (code.claude.com/docs/en/hooks): `PermissionRequest`'s
//! `decision`, and `PreToolUse`'s `permissionDecision` + `updatedInput` for the tools that
//! need you (AskUserQuestion, ExitPlanMode), and `Elicitation`'s `action` / `content` for
//! MCP forms and links. Codex reads the permission shape as-is; the
//! OpenCode plugin reads `decision` from it (allow, allow with `updatedPermissions` for
//! "always", deny with its `message`) and its questions' answers from our own shape. "No decision" is no JSON at all
//! (`None` to the bridge), so it isn't here. Pure.

use serde_json::{json, Map, Value};

/// What a deny tells the agent when you didn't say more.
pub(super) const DENIED: &str = "Denied from the ReUI island.";

/// A permission answered from the island.
#[derive(Debug, PartialEq)]
pub(super) enum Permission {
    Allow,
    /// Allow, and the agent's own "always allow" suggestion with it.
    Always(Value),
    /// Deny, with what the model is told instead.
    Deny(String),
}

pub(super) fn permission(answer: Permission) -> String {
    let decision = match answer {
        Permission::Always(suggestion) => json!({ "behavior": "allow", "updatedPermissions": [suggestion] }),
        Permission::Allow => json!({ "behavior": "allow" }),
        Permission::Deny(message) => json!({ "behavior": "deny", "message": message }),
    };
    json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": decision } }).to_string()
}

/// AskUserQuestion answered: the tool's input as received (its `questions` untouched) plus
/// `answers`, the chosen label (several joined with ", ", a typed answer as its text) keyed
/// by each question's text. "allow" alone wouldn't answer anything.
pub(super) fn questions(input: &Value, answers: &[(String, String)]) -> String {
    let mut updated = input.as_object().cloned().unwrap_or_default();
    let answers: Map<String, Value> = answers.iter().map(|(q, a)| (q.clone(), Value::String(a.clone()))).collect();
    updated.insert("answers".into(), Value::Object(answers));
    allow(Value::Object(updated))
}

/// An MCP elicitation (`Elicitation` hook) answered from the island.
#[derive(Debug, PartialEq)]
pub(super) enum Elicitation {
    /// The form's values keyed by the schema's property names; `None` for a URL the island
    /// opened (which Claude then doesn't open again).
    Accept(Option<Map<String, Value>>),
    Decline,
}

/// The `Elicitation` hook's answer: `action` and, when accepting a form, its `content`.
/// ("cancel", Claude's Esc, isn't sent: leaving a request is "Answer in terminal", which
/// is no output and Claude's own dialog.)
pub(super) fn elicitation(answer: Elicitation) -> String {
    let out = match answer {
        Elicitation::Accept(Some(content)) => json!({ "hookEventName": "Elicitation", "action": "accept", "content": content }),
        Elicitation::Accept(None) => json!({ "hookEventName": "Elicitation", "action": "accept" }),
        Elicitation::Decline => json!({ "hookEventName": "Elicitation", "action": "decline" }),
    };
    json!({ "hookSpecificOutput": out }).to_string()
}

/// OpenCode's questions answered: one list per question, in order (the picked labels, or
/// the typed text alone), as the plugin posts them to `/question/{id}/reply`. ReUI's own
/// shape, in the same envelope as the rest; only our plugin reads it.
pub(super) fn opencode_questions(answers: &[Vec<String>]) -> String {
    json!({ "hookSpecificOutput": { "hookEventName": "Question", "answers": answers } }).to_string()
}

/// ExitPlanMode approved: allow, echoing the input (the plan Claude Code injected). Which
/// permission mode Claude continues in stays Claude's choice; we set none.
pub(super) fn plan_approved(input: &Value) -> String {
    allow(if input.is_object() { input.clone() } else { json!({}) })
}

/// ExitPlanMode sent back: deny, with what should change as the reason Claude reads.
pub(super) fn plan_revised(feedback: &str) -> String {
    json!({ "hookSpecificOutput": { "hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": feedback } }).to_string()
}

fn allow(updated_input: Value) -> String {
    json!({ "hookSpecificOutput": { "hookEventName": "PreToolUse", "permissionDecision": "allow", "updatedInput": updated_input } }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn permissions_match_the_hook_schema() {
        let allow = parse(&permission(Permission::Allow));
        assert_eq!(allow, json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": { "behavior": "allow" } } }));

        let suggestion = json!({ "type": "addRules", "rules": [{ "toolName": "Bash", "ruleContent": "cargo test" }], "behavior": "allow", "destination": "localSettings" });
        let always = parse(&permission(Permission::Always(suggestion.clone())));
        assert_eq!(always["hookSpecificOutput"]["decision"], json!({ "behavior": "allow", "updatedPermissions": [suggestion] }));

        let deny = parse(&permission(Permission::Deny("Use cargo nextest instead".into())));
        assert_eq!(deny["hookSpecificOutput"]["decision"], json!({ "behavior": "deny", "message": "Use cargo nextest instead" }));
    }

    #[test]
    fn questions_echo_the_input_and_add_answers_by_question_text() {
        // The documented example (hooks reference, "Tools that require user interaction").
        let input = json!({ "questions": [
            { "question": "Which framework?", "header": "Framework", "options": [{ "label": "React", "description": "Component library" }, { "label": "Vue", "description": "Progressive framework" }], "multiSelect": false },
            { "question": "Which features?", "header": "Features", "options": [{ "label": "Blog", "description": "" }, { "label": "Search", "description": "" }], "multiSelect": true }
        ] });
        let out = questions(&input, &[("Which framework?".into(), "React".into()), ("Which features?".into(), "Blog, Search".into())]);
        assert_eq!(
            parse(&out),
            json!({ "hookSpecificOutput": { "hookEventName": "PreToolUse", "permissionDecision": "allow", "updatedInput": {
                "questions": input["questions"],
                "answers": { "Which framework?": "React", "Which features?": "Blog, Search" }
            } } })
        );
    }

    #[test]
    fn elicitations_match_the_hook_schema() {
        // The hooks reference's examples, verbatim.
        let content = json!({ "username": "alice" }).as_object().unwrap().clone();
        assert_eq!(parse(&elicitation(Elicitation::Accept(Some(content)))), json!({ "hookSpecificOutput": { "hookEventName": "Elicitation", "action": "accept", "content": { "username": "alice" } } }));
        assert_eq!(parse(&elicitation(Elicitation::Decline)), json!({ "hookSpecificOutput": { "hookEventName": "Elicitation", "action": "decline" } }));
        assert_eq!(parse(&elicitation(Elicitation::Accept(None))), json!({ "hookSpecificOutput": { "hookEventName": "Elicitation", "action": "accept" } }));
    }

    #[test]
    fn opencode_answers_are_one_list_per_question() {
        // `POST /question/{requestID}/reply` takes `{ answers: string[][] }` (OpenCode 1.19).
        let out = parse(&opencode_questions(&[vec!["Astro".into()], vec!["Blog".into(), "Search".into()], vec!["typed".into()]]));
        assert_eq!(out, json!({ "hookSpecificOutput": { "hookEventName": "Question", "answers": [["Astro"], ["Blog", "Search"], ["typed"]] } }));
    }

    #[test]
    fn plans_are_approved_with_their_input_or_sent_back_with_a_reason() {
        let input = json!({ "plan": "# Refactor auth\n1. Extract", "planFilePath": "C:/Users/me/.claude/plans/auth.md" });
        let approved = parse(&plan_approved(&input));
        assert_eq!(approved["hookSpecificOutput"]["permissionDecision"], "allow");
        assert_eq!(approved["hookSpecificOutput"]["updatedInput"], input);
        assert!(approved["hookSpecificOutput"].get("updatedPermissions").is_none(), "no mode is picked for you");

        let revised = parse(&plan_revised("Keep the old API"));
        assert_eq!(revised, json!({ "hookSpecificOutput": { "hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": "Keep the old API" } }));
    }
}
