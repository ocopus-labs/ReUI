//! Coding agents (Claude Code, Codex, OpenCode): live sessions, permission decisions and
//! token usage, one module instance per agent with a [`Profile`] for what differs.
//!
//! Fed by `reui-bridge <agent>` hook events. Claude Code and Codex send their own hook
//! JSON; OpenCode's plugin (installed by `reui-bridge install opencode`) translates its
//! events into the same shape, so one state machine serves all three.
//!
//! Each session is one activity that follows the turn: thinking → running a tool (or
//! compacting, waiting on a permission, asking you something) → done or failed. Done and
//! failed show briefly and then vanish. Each permission request is its own decision
//! activity; answering it replies to the waiting bridge, and if the bridge goes away
//! (answered in the agent's own UI, timed out) the request vanishes.
//!
//! Status text is quiet (secondary); colour is kept for the logo and for "your turn".
//!
//! Each hook event is also noted (throttled) in `agents.json` for Settings' status line,
//! and `reui-bridge test <agent>` (Settings' "Send test event") gets a short "connected"
//! confirmation instead of a session.
//!
//! Files: `view.rs` (how sessions and requests look), `usage.rs` (token usage),
//! `status.rs` (`agents.json`, the test event).

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reui_protocol::{Art, Element, Rgb};
use serde_json::{json, Value};
use tokio::sync::oneshot;
use tokio::time::{sleep_until, Instant};

mod status;
mod usage;
mod view;

use self::status::Throttle;
use self::usage::Ledger;
use self::view::{publish_request, publish_session, publish_test, short_tool_name, GLYPH_TERMINAL, TEST_KEY};
use super::Module;
use crate::hub::{HookCall, ModuleContext, ModuleInput};

/// Where an agent's token usage comes from.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum UsageSource {
    /// `~/.claude/projects/**/*.jsonl` transcripts.
    ClaudeTranscripts,
    /// `~/.codex/state_*.sqlite` → `threads.tokens_used`.
    CodexThreads,
    /// `~/.local/share/opencode/opencode.db` → `session` tokens and cost.
    OpenCodeSessions,
}

/// What differs between agents.
pub struct Profile {
    /// Module name; also what `reui-bridge <agent>` is called with.
    pub name: &'static str,
    pub display: &'static str,
    pub accent: Rgb,
    pub logo: Option<&'static [u8]>,
    /// Tile glyph when there's no logo.
    pub glyph: char,
    /// Whether the agent accepts "always allow" (Claude Code's `updatedPermissions`).
    pub can_always: bool,
    pub usage: UsageSource,
}

pub const CLAUDE: Profile = Profile {
    name: "claude",
    display: "Claude Code",
    accent: Rgb(217, 119, 87),
    logo: Some(include_bytes!("../../../../../design/assets/agents/claude.svg")),
    glyph: GLYPH_TERMINAL,
    can_always: true,
    usage: UsageSource::ClaudeTranscripts,
};

pub const CODEX: Profile = Profile {
    name: "codex",
    display: "Codex",
    accent: Rgb(16, 163, 127),
    logo: None,
    glyph: GLYPH_TERMINAL,
    // Codex reserves `updatedPermissions` and fails closed on it today.
    can_always: false,
    usage: UsageSource::CodexThreads,
};

pub const OPENCODE: Profile = Profile {
    name: "opencode",
    display: "OpenCode",
    accent: Rgb(130, 170, 255),
    logo: Some(include_bytes!("../../../../../design/assets/agents/opencode.svg")),
    glyph: GLYPH_TERMINAL,
    // `permission.ask` only takes allow / deny / ask.
    can_always: false,
    usage: UsageSource::OpenCodeSessions,
};

/// Shorter than the bridge's wait (110 s), which is shorter than the hook timeout (120 s),
/// so the agent always gets a clean "no decision" rather than a killed hook.
const DECISION_TIMEOUT: Duration = Duration::from_secs(100);
/// A finished turn shows briefly, then the session disappears until the next prompt.
const DONE_SHOW: Duration = Duration::from_secs(6);
const FAILED_SHOW: Duration = Duration::from_secs(15);
/// A session that sends nothing for this long is gone (crashed, or the terminal was killed
/// before `SessionEnd`). Tools report before and after running, so this rarely bites.
const STALE_AFTER: Duration = Duration::from_secs(15 * 60);
const PROMPT_CHARS: usize = 120;
/// The test event's confirmation stays this long.
const TEST_SHOW: Duration = Duration::from_secs(4);

pub struct Agent(pub &'static Profile);

impl Module for Agent {
    fn name(&self) -> &'static str {
        self.0.name
    }

    fn run(self: Box<Self>, ctx: ModuleContext) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(run(self.0, ctx))
    }
}

#[derive(Clone, PartialEq, Debug)]
enum Phase {
    Thinking,
    Tool(String),
    Compacting,
    WaitingPermission,
    NeedsInput,
    Done { took: Option<Duration> },
    Failed { reason: Option<String> },
}

struct Session {
    project: String,
    prompt: Option<String>,
    transcript: Option<PathBuf>,
    phase: Option<Phase>,
    turn_started: Option<Instant>,
    subagents: u32,
    /// Tokens for this session and today, read when a turn finishes.
    usage: Option<Usage>,
    retract_at: Option<Instant>,
    last_event: Instant,
}

/// Fresh tokens (cache reads left out where the agent reports them).
#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct Usage {
    session: Option<u64>,
    today: Option<u64>,
}

struct Pending {
    session: String,
    tool: String,
    reply: oneshot::Sender<Option<String>>,
    suggestion: Option<Value>,
    deadline: Instant,
}

/// Everything a module instance needs besides its sessions.
struct Agentry {
    profile: &'static Profile,
    mark: Element,
    ledger: Arc<Mutex<Ledger>>,
    /// `agents.json` writes.
    status: Mutex<Throttle>,
}

async fn run(profile: &'static Profile, mut ctx: ModuleContext) {
    let mark = match profile.logo {
        Some(svg) => Element::Art(Art::Logo { image: ctx.image(svg.to_vec()), animated: false }),
        None => Element::Art(Art::Glyph(profile.glyph)),
    };
    let me = Agentry { profile, mark, ledger: Arc::new(Mutex::new(Ledger::default())), status: Mutex::new(Throttle::default()) };
    // Claude's usage needs today's transcripts read once; taken out of the loop when done,
    // so a finished scan can't wake the loop again.
    let mut scan = (profile.usage == UsageSource::ClaudeTranscripts)
        .then(usage::claude_projects_dir)
        .flatten()
        .map(|root| {
            let ledger = me.ledger.clone();
            tokio::task::spawn_blocking(move || Ledger::scan_today(&ledger, &root))
        });

    let mut sessions: HashMap<String, Session> = HashMap::new();
    let mut pending: HashMap<u64, Pending> = HashMap::new();
    let mut test_until: Option<Instant> = None;

    loop {
        let next_deadline = pending
            .values()
            .map(|p| p.deadline)
            .chain(sessions.values().flat_map(|s| [s.retract_at, Some(s.last_event + STALE_AFTER)].into_iter().flatten()))
            .chain(test_until)
            .min();
        tokio::select! {
            input = ctx.next() => match input {
                Some(ModuleInput::Hook(call)) if status::is_test(&call.payload) => {
                    publish_test(&ctx, &me);
                    test_until = Some(Instant::now() + TEST_SHOW);
                }
                Some(ModuleInput::Hook(call)) => {
                    status::note_event(&me.status, profile.name);
                    on_hook(&ctx, &me, call, &mut sessions, &mut pending).await
                }
                Some(ModuleInput::HookGone(id)) => {
                    if let Some(p) = pending.remove(&id) {
                        ctx.retract(&permission_key(id));
                        // Answered elsewhere: the tool most likely runs now.
                        resume(&ctx, &me, &mut sessions, &p.session, Phase::Tool(short_tool_name(&p.tool)));
                    }
                }
                Some(ModuleInput::Action(action)) => {
                    let Some(id) = action.key.strip_prefix("permission-").and_then(|n| n.parse().ok()) else { continue };
                    let Some(p) = pending.remove(&id) else { continue };
                    println!("{}: {} answered from the island: {}", profile.name, action.key, action.action);
                    let allowed = action.action != "deny";
                    _ = p.reply.send(Some(decision(&action.action, p.suggestion)));
                    ctx.retract(&action.key);
                    let next = if allowed { Phase::Tool(short_tool_name(&p.tool)) } else { Phase::Thinking };
                    resume(&ctx, &me, &mut sessions, &p.session, next);
                }
                None => break,
            },
            _ = async { scan.as_mut().unwrap().await }, if scan.is_some() => {
                scan = None;
                for (id, session) in &sessions {
                    publish_session(&ctx, &me, id, session);
                }
            }
            _ = async { sleep_until(next_deadline.unwrap()).await }, if next_deadline.is_some() => {
                let now = Instant::now();
                if test_until.is_some_and(|t| t <= now) {
                    test_until = None;
                    ctx.retract(TEST_KEY);
                }
                let mut timed_out = Vec::new();
                pending.retain(|id, p| {
                    let expired = p.deadline <= now;
                    if expired {
                        // Silence: the agent falls back to its own prompt.
                        ctx.retract(&permission_key(*id));
                        timed_out.push(p.session.clone());
                    }
                    !expired
                });
                for session in timed_out {
                    resume(&ctx, &me, &mut sessions, &session, Phase::WaitingPermission);
                }
                sessions.retain(|id, session| {
                    let gone = session.retract_at.is_some_and(|t| t <= now) || session.last_event + STALE_AFTER <= now;
                    if gone {
                        ctx.retract(&session_key(id));
                    }
                    !gone
                });
            }
        }
    }
}

async fn on_hook(ctx: &ModuleContext, me: &Agentry, call: HookCall, sessions: &mut HashMap<String, Session>, pending: &mut HashMap<u64, Pending>) {
    let Ok(event) = serde_json::from_str::<Value>(&call.payload) else { return };
    let str_field = |k: &str| event.get(k).and_then(Value::as_str).unwrap_or_default();
    let session_id = str_field("session_id").to_string();
    if session_id.is_empty() {
        return;
    }
    if str_field("hook_event_name") == "SessionEnd" {
        sessions.remove(&session_id);
        ctx.retract(&session_key(&session_id));
        return;
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
    });
    session.last_event = Instant::now();
    if let Some(path) = event.get("transcript_path").and_then(Value::as_str) {
        session.transcript = Some(PathBuf::from(path));
    }

    let phase = match str_field("hook_event_name") {
        "UserPromptSubmit" => {
            if let Some(prompt) = first_line(str_field("prompt")) {
                session.prompt = Some(prompt);
            }
            session.turn_started = Some(Instant::now());
            session.subagents = 0;
            Some(Phase::Thinking)
        }
        // The agent asking you something is a question, not a tool run.
        "PreToolUse" if matches!(str_field("tool_name"), "AskUserQuestion" | "question") => Some(Phase::NeedsInput),
        "PreToolUse" => Some(Phase::Tool(short_tool_name(str_field("tool_name")))),
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
        "Notification" if matches!(str_field("notification_type"), "agent_needs_input" | "elicitation_dialog") => Some(Phase::NeedsInput),
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
            let tool = str_field("tool_name").to_string();
            let input = event.get("tool_input").cloned().unwrap_or(Value::Null);
            let suggestion = event
                .get("permission_suggestions")
                .and_then(Value::as_array)
                .and_then(|s| s.first())
                .cloned()
                .filter(|_| me.profile.can_always);
            publish_request(ctx, me, call.id, &session.project, &tool, &input, suggestion.is_some());
            pending.insert(call.id, Pending { session: session_id.clone(), tool, reply, suggestion, deadline: Instant::now() + DECISION_TIMEOUT });
            Some(Phase::WaitingPermission)
        }
        _ => None,
    };
    if let Some(phase) = phase {
        set_phase(ctx, me, &session_id, session, phase);
    }
}

/// Session and today's tokens, from wherever this agent keeps them.
async fn read_usage(me: &Agentry, session_id: &str, transcript: Option<PathBuf>) -> Option<Usage> {
    let source = me.profile.usage;
    let ledger = me.ledger.clone();
    let session_id = session_id.to_string();
    tokio::task::spawn_blocking(move || match source {
        UsageSource::ClaudeTranscripts => {
            let mut ledger = ledger.lock().unwrap();
            if let Some(path) = &transcript {
                ledger.update(path);
            }
            let session = transcript.as_deref().map(|p| ledger.file_total(p).fresh()).filter(|t| *t > 0);
            Some(Usage { session, today: Some(ledger.today_total().fresh()) })
        }
        UsageSource::CodexThreads => usage::codex(&session_id).map(|(session, today)| Usage { session, today: Some(today) }),
        UsageSource::OpenCodeSessions => usage::opencode(&session_id).map(|(session, today)| Usage { session, today: Some(today) }),
    })
    .await
    .ok()
    .flatten()
}

/// Moves a session on after its permission request is settled.
fn resume(ctx: &ModuleContext, me: &Agentry, sessions: &mut HashMap<String, Session>, id: &str, phase: Phase) {
    if let Some(session) = sessions.get_mut(id) {
        if session.phase == Some(Phase::WaitingPermission) {
            set_phase(ctx, me, id, session, phase);
        }
    }
}

fn set_phase(ctx: &ModuleContext, me: &Agentry, id: &str, session: &mut Session, phase: Phase) {
    session.retract_at = match phase {
        Phase::Done { .. } => Some(Instant::now() + DONE_SHOW),
        Phase::Failed { .. } => Some(Instant::now() + FAILED_SHOW),
        _ => None,
    };
    session.phase = Some(phase);
    publish_session(ctx, me, id, session);
}

/// The JSON the bridge prints. Claude Code and Codex read it as-is; the OpenCode plugin
/// reads `decision.behavior` from it.
fn decision(action: &str, suggestion: Option<Value>) -> String {
    let decision = match (action, suggestion) {
        ("always", Some(suggestion)) => json!({ "behavior": "allow", "updatedPermissions": [suggestion] }),
        ("allow" | "always", _) => json!({ "behavior": "allow" }),
        _ => json!({ "behavior": "deny", "message": "Denied from the ReUI island." }),
    };
    json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": decision } }).to_string()
}

/// First non-empty line, without paste markers, cut to a readable length.
fn first_line(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with("<pasted_content"))?;
    Some(match line.char_indices().nth(PROMPT_CHARS) {
        Some((cut, _)) => format!("{}…", &line[..cut]),
        None => line.to_string(),
    })
}

/// Last folder of the working directory (Codex reports `\\?\D:\...` paths).
fn project_name(cwd: &str, fallback: &str) -> String {
    let cwd = cwd.trim_start_matches(r"\\?\").trim_end_matches(['/', '\\']);
    Path::new(cwd).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| fallback.into())
}

fn session_key(id: &str) -> String {
    format!("session-{id}")
}

fn permission_key(id: u64) -> String {
    format!("permission-{id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decisions_match_the_hook_schema() {
        let allow: Value = serde_json::from_str(&decision("allow", None)).unwrap();
        assert_eq!(allow["hookSpecificOutput"]["hookEventName"], "PermissionRequest");
        assert_eq!(allow["hookSpecificOutput"]["decision"], json!({ "behavior": "allow" }));

        let suggestion = json!({ "type": "addRules", "rules": [{ "toolName": "Bash", "ruleContent": "cargo test" }], "behavior": "allow", "destination": "localSettings" });
        let always: Value = serde_json::from_str(&decision("always", Some(suggestion.clone()))).unwrap();
        assert_eq!(always["hookSpecificOutput"]["decision"]["updatedPermissions"], json!([suggestion]));

        let deny: Value = serde_json::from_str(&decision("deny", None)).unwrap();
        assert_eq!(deny["hookSpecificOutput"]["decision"]["behavior"], "deny");
        assert!(deny["hookSpecificOutput"]["decision"]["message"].is_string());
    }

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
}
