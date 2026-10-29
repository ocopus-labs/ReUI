//! One hook event into the module's state: the session's phase (thinking, a tool, done…)
//! and the moments that need you (a permission, Claude's questions and plans, OpenCode's
//! questions from its plugin's `Question` event).
//!
//! Claude's AskUserQuestion and ExitPlanMode come through `PreToolUse` twice: the blocking
//! entry (`reui-bridge claude ask`, which waits for the answer) and the async `*` one;
//! `moments::Queue::arrive` makes them one moment. A `PermissionRequest` for either tool is
//! answered "no decision" at once: allowing it wouldn't carry an answer, and the island
//! already offered it through `PreToolUse`. An MCP server's form or link (`Elicitation`,
//! blocking) is a moment of its own; Claude's `elicitation_dialog` notification only comes
//! once that's left to the terminal, so it shows nothing more. Codex's questions
//! (`request_user_input`) show read-only.

use std::path::{Path, PathBuf};

use serde_json::Value;
use tokio::time::Instant;

use super::describe::short_tool_name;
use super::terminal::Origin;
use super::moments::Moment;
use super::view::publish_moments;
use super::{read_usage, set_phase, Agentry, Moments, Phase, Session, PROMPT_CHARS};
use crate::hub::{HookCall, ModuleContext};

/// Claude's tools that need you, answered from `PreToolUse`.
fn asks(tool: &str) -> bool {
    matches!(tool, "AskUserQuestion" | "ExitPlanMode")
}

pub(super) async fn on_hook(ctx: &ModuleContext, me: &Agentry, call: HookCall, sessions: &mut std::collections::HashMap<String, Session>, moments: &mut Moments) {
    let Ok(event) = serde_json::from_str::<Value>(&call.payload) else { return };
    let str_field = |k: &str| event.get(k).and_then(Value::as_str).unwrap_or_default();
    let session_id = str_field("session_id").to_string();
    if session_id.is_empty() {
        return;
    }
    let name = str_field("hook_event_name");
    let tool = str_field("tool_name");
    match name {
        "PermissionRequest" if asks(tool) => return,
        "SessionEnd" => {
            sessions.remove(&session_id);
            ctx.retract(&super::session_key(&session_id));
            if moments.queue.end_read_only(&session_id, None) {
                publish_moments(ctx, me, moments, sessions);
            }
            return;
        }
        _ => {}
    }
    let session = sessions.entry(session_id.clone()).or_insert_with(|| Session {
        project: project_name(str_field("cwd"), me.profile.display),
        prompt: None,
        transcript: None,
        phase: None,
        turn_started: None,
        subagents: 0,
        usage: None,
        retract_at: None,
        last_event: Instant::now(),
        origin: None,
    });
    session.last_event = Instant::now();
    if let Some(origin) = Origin::of(&event) {
        session.origin = Some(origin);
    }
    if let Some(path) = event.get("transcript_path").and_then(Value::as_str) {
        session.transcript = Some(PathBuf::from(path));
    }
    let tool_use_id = event.get("tool_use_id").and_then(Value::as_str).map(str::to_owned);
    let input = || event.get("tool_input").cloned().unwrap_or(Value::Null);
    // A read-only question or plan is over once its tool ran or the turn moved on.
    let mut moments_changed = match name {
        "PostToolUse" => moments.queue.end_read_only(&session_id, tool_use_id.as_deref()),
        "UserPromptSubmit" | "Stop" | "StopFailure" => moments.queue.end_read_only(&session_id, None),
        _ => false,
    };

    let phase = match name {
        "UserPromptSubmit" => {
            if let Some(prompt) = first_line(str_field("prompt")) {
                session.prompt = Some(prompt);
            }
            session.turn_started = Some(Instant::now());
            session.subagents = 0;
            Some(Phase::Thinking)
        }
        // Claude asking you something, or a plan to approve: a moment (answerable when it
        // came through the blocking entry), and the session waits for you.
        "PreToolUse" if me.profile.name == super::CLAUDE.name && asks(tool) => {
            let Some(moment) = Moment::ask(call.id, &session_id, tool, tool_use_id, input(), call.reply.is_some(), Instant::now()) else { return };
            if let Some(reply) = call.reply {
                moments.replies.insert(call.id, reply);
            }
            moments.queue.arrive(moment);
            moments_changed = true;
            Some(Phase::WaitingPermission)
        }
        // OpenCode's question, through its `question.asked` event (the plugin's own
        // "Question"): answerable when the plugin can post the answer back.
        "Question" => {
            let waiting = call.reply.is_some();
            moments.queue.arrive(Moment::opencode_question(call.id, &session_id, event.clone(), waiting, Instant::now()));
            if let Some(reply) = call.reply {
                moments.replies.insert(call.id, reply);
            }
            moments_changed = true;
            Some(Phase::WaitingPermission)
        }
        // An MCP server asking for input (Claude): a form or a link, answered from here.
        "Elicitation" => {
            let Some(reply) = call.reply else { return };
            moments.queue.arrive(Moment::elicitation(call.id, &session_id, &event, Instant::now()));
            moments.replies.insert(call.id, reply);
            moments_changed = true;
            Some(Phase::WaitingPermission)
        }
        // Codex asking you something: shown read-only (no hook can answer it) until its
        // `PostToolUse` or the turn's end.
        "PreToolUse" if tool == "request_user_input" => {
            moments.queue.arrive(Moment::codex_question(call.id, &session_id, tool_use_id.clone(), input(), Instant::now()));
            moments_changed = true;
            Some(Phase::WaitingPermission)
        }
        // OpenCode's question tool runs while its `question.asked` waits; that's the moment.
        "PreToolUse" if tool == "question" => Some(Phase::WaitingPermission),
        "PreToolUse" => Some(Phase::Tool(short_tool_name(tool))),
        "PostToolUse" => Some(Phase::Thinking),
        "PreCompact" => Some(Phase::Compacting),
        "PostCompact" => Some(Phase::Thinking),
        "SubagentStart" => {
            session.subagents += 1;
            session.phase.clone()
        }
        "SubagentStop" => {
            session.subagents = session.subagents.saturating_sub(1);
            session.phase.clone()
        }
        // A moment already shows what the agent needs.
        "Notification" if moments.queue.has_session(&session_id) => None,
        "Notification" if str_field("notification_type") == "agent_needs_input" => match &session.phase {
            // The question the agent asked already says more than the notification.
            Some(Phase::NeedsInput(Some(q))) => Some(Phase::NeedsInput(Some(q.clone()))),
            _ => Some(Phase::NeedsInput(None)),
        },
        "Stop" => {
            session.usage = read_usage(me, &session_id, session.transcript.clone()).await;
            session.subagents = 0;
            Some(Phase::Done { took: session.turn_started.take().map(|t| t.elapsed()) })
        }
        "StopFailure" => {
            let reason = ["error", "message", "reason"].iter().find_map(|k| event.get(*k).and_then(Value::as_str)).and_then(first_line);
            session.turn_started = None;
            Some(Phase::Failed { reason })
        }
        "PermissionRequest" => {
            let Some(reply) = call.reply else { return };
            let suggestion = event
                .get("permission_suggestions")
                .and_then(Value::as_array)
                .and_then(|s| s.first())
                .cloned()
                .filter(|_| me.profile.can_always);
            moments.queue.arrive(Moment::permission(call.id, &session_id, tool, input(), suggestion, Instant::now()));
            moments.replies.insert(call.id, reply);
            moments_changed = true;
            Some(Phase::WaitingPermission)
        }
        _ => None,
    };
    if let Some(phase) = phase {
        set_phase(ctx, me, &session_id, session, phase);
    }
    if moments_changed {
        publish_moments(ctx, me, moments, sessions);
    }
}

/// First non-empty line, without paste markers, cut to a readable length.
pub(super) fn first_line(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with("<pasted_content"))?;
    Some(match line.char_indices().nth(PROMPT_CHARS) {
        Some((cut, _)) => format!("{}…", &line[..cut]),
        None => line.to_string(),
    })
}

/// Last folder of the working directory (Codex reports `\\?\D:\...` paths).
pub(super) fn project_name(cwd: &str, fallback: &str) -> String {
    let cwd = cwd.trim_start_matches(r"\\?\").trim_end_matches(['/', '\\']);
    Path::new(cwd).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| fallback.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_are_named_by_their_folder() {
        assert_eq!(project_name(r"\\?\D:\ocopus", "Codex"), "ocopus");
        assert_eq!(project_name("D:/fusion-proxy/", "OpenCode"), "fusion-proxy");
        assert_eq!(project_name("", "Codex"), "Codex");
    }

    #[test]
    fn prompts_become_one_readable_line() {
        assert_eq!(first_line("\n  fix the build  \nmore"), Some("fix the build".into()));
        assert_eq!(first_line("<pasted_content id=\"x\">\nlog line"), Some("log line".into()));
        assert_eq!(first_line(&"a".repeat(200)).unwrap().chars().count(), PROMPT_CHARS + 1);
        assert_eq!(first_line("   "), None);
    }

    #[test]
    fn only_claudes_asking_tools_are_answered_from_pre_tool_use() {
        assert!(asks("AskUserQuestion") && asks("ExitPlanMode"));
        assert!(!asks("Bash") && !asks("question") && !asks("request_user_input"));
    }
}
