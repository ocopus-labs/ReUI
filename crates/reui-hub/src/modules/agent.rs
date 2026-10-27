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

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reui_protocol::{Art, Button, ButtonContent, ButtonRole, CompactView, Element, ExpandedView, Headline, Priority, Rgb, Row, Tone};
use serde_json::{json, Value};
use tokio::sync::oneshot;
use tokio::time::{sleep_until, Instant};

use super::usage::{self, format_tokens, Ledger};
use super::Module;
use crate::hub::{HookCall, ModuleContext, ModuleInput, View};

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
    logo: Some(include_bytes!("../../../../design/assets/agents/claude.svg")),
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
    logo: Some(include_bytes!("../../../../design/assets/agents/opencode.svg")),
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

const GLYPH_TERMINAL: char = '\u{E756}';
const GLYPH_EDIT: char = '\u{E70F}';
const GLYPH_READ: char = '\u{E8A5}';
const GLYPH_WEB: char = '\u{E774}';
const GLYPH_SEARCH: char = '\u{E721}';
const GLYPH_TOOL: char = '\u{E90F}';
const GLYPH_DONE: char = '\u{E73E}';
const GLYPH_FAILED: char = '\u{E7BA}';

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
}

async fn run(profile: &'static Profile, mut ctx: ModuleContext) {
    let mark = match profile.logo {
        Some(svg) => Element::Art(Art::Logo { image: ctx.image(svg.to_vec()), animated: false }),
        None => Element::Art(Art::Glyph(profile.glyph)),
    };
    let me = Agentry { profile, mark, ledger: Arc::new(Mutex::new(Ledger::default())) };
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

    loop {
        let next_deadline = pending
            .values()
            .map(|p| p.deadline)
            .chain(sessions.values().flat_map(|s| [s.retract_at, Some(s.last_event + STALE_AFTER)].into_iter().flatten()))
            .min();
        tokio::select! {
            input = ctx.next() => match input {
                Some(ModuleInput::Hook(call)) => on_hook(&ctx, &me, call, &mut sessions, &mut pending).await,
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

/// (priority, compact trailing, status line) for a phase. Text is quiet by default; colour
/// only where it's your move.
fn present(phase: &Phase, session: &Session) -> (Priority, Element, String) {
    let quiet = |text: &str| Element::Text { text: text.into(), tone: Tone::Secondary };
    let with_agents = |status: String| match session.subagents {
        0 => status,
        1 => format!("{status} · 1 subagent"),
        n => format!("{status} · {n} subagents"),
    };
    match phase {
        Phase::Thinking => (Priority::Ongoing, quiet("Thinking"), with_agents("Thinking".into())),
        Phase::Tool(tool) => (Priority::Ongoing, quiet(tool), with_agents(format!("Running {tool}"))),
        Phase::Compacting => (Priority::Ongoing, quiet("Compacting"), "Compacting the conversation".into()),
        Phase::WaitingPermission => (Priority::Ongoing, quiet("Waiting"), "Waiting for your permission".into()),
        Phase::NeedsInput => (Priority::Alert, Element::Text { text: "Your turn".into(), tone: Tone::Accent }, "Has a question for you".into()),
        Phase::Done { took } => {
            let mut parts = vec![match took {
                Some(took) => format!("Done in {}", format_duration(*took)),
                None => "Done".to_string(),
            }];
            let usage = session.usage.unwrap_or_default();
            if let Some(tokens) = usage.session.filter(|t| *t > 0) {
                parts.push(format!("{} tokens", format_tokens(tokens)));
            }
            if let Some(today) = usage.today {
                parts.push(format!("{} today", format_tokens(today)));
            }
            (Priority::Alert, Element::Glyph { glyph: GLYPH_DONE, tone: Tone::Secondary }, parts.join(" · "))
        }
        Phase::Failed { reason } => (
            Priority::Alert,
            Element::Glyph { glyph: GLYPH_FAILED, tone: Tone::Negative },
            reason.clone().map(|r| format!("Stopped: {r}")).unwrap_or_else(|| "The turn failed".into()),
        ),
    }
}

fn publish_session(ctx: &ModuleContext, me: &Agentry, id: &str, session: &Session) {
    let Some(phase) = &session.phase else { return };
    let (priority, trailing, status) = present(phase, session);
    ctx.publish(
        &session_key(id),
        priority,
        View {
            accent: Some(me.profile.accent),
            audio: None,
            compact: CompactView { leading: me.mark.clone(), trailing },
            minimal: me.mark.clone(),
            expanded: Some(ExpandedView {
                leading: me.mark.clone(),
                eyebrow: Some(format!("{} · {}", me.profile.display, session.project)),
                // What you asked is the most useful thing to read; the project otherwise.
                headline: Headline::Text(session.prompt.clone().unwrap_or_else(|| session.project.clone())),
                detail: Some(status),
                trailing: Element::Empty,
                wide: false,
                tint: false,
                pager: None,
                open: None,
                rows: Vec::new(),
            }),
        },
    );
}

fn publish_request(ctx: &ModuleContext, me: &Agentry, id: u64, project: &str, tool: &str, input: &Value, can_always: bool) {
    let (glyph, headline, detail) = describe_tool(tool, input);
    let button = |action: &str, label: &str, role| Button { action: action.into(), content: ButtonContent::Text(label.into()), role };
    let mut buttons = vec![button("deny", "Deny", ButtonRole::Destructive)];
    if can_always {
        buttons.push(button("always", "Always", ButtonRole::Secondary));
    }
    buttons.push(button("allow", "Allow", ButtonRole::Primary));
    ctx.publish(
        &permission_key(id),
        Priority::Decision,
        View {
            accent: Some(me.profile.accent),
            audio: None,
            compact: CompactView { leading: me.mark.clone(), trailing: Element::Text { text: "Allow?".into(), tone: Tone::Accent } },
            minimal: me.mark.clone(),
            expanded: Some(ExpandedView {
                leading: Element::Art(Art::Glyph(glyph)),
                eyebrow: Some(format!("{} · {project}", me.profile.display)),
                headline,
                detail: Some(detail),
                trailing: Element::Empty,
                wide: false,
                tint: false,
                pager: None,
                open: None,
                rows: vec![Row::Buttons(buttons)],
            }),
        },
    );
}

/// First non-empty line, without paste markers, cut to a readable length.
fn first_line(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with("<pasted_content"))?;
    Some(match line.char_indices().nth(PROMPT_CHARS) {
        Some((cut, _)) => format!("{}…", &line[..cut]),
        None => line.to_string(),
    })
}

/// What the request is about, readable at a glance. Knows Claude Code's, Codex's and
/// OpenCode's tool names (OpenCode's are lowercase and use `filePath`).
fn describe_tool(tool: &str, input: &Value) -> (char, Headline, String) {
    let text = |k: &str| match input.get(k) {
        Some(Value::String(s)) => s.clone(),
        // Codex passes some commands as an argv array.
        Some(Value::Array(parts)) => parts.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "),
        _ => String::new(),
    };
    let path = || [text("file_path"), text("filePath"), text("path")].into_iter().find(|p| !p.is_empty()).unwrap_or_default();
    let file = |p: &str| Path::new(p).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string());
    let folder = |p: &str| Path::new(p).parent().and_then(Path::file_name).map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    let description = || Some(text("description")).filter(|d| !d.is_empty());
    match tool.to_ascii_lowercase().as_str() {
        "bash" | "powershell" | "shell" | "local_shell" | "exec_command" => {
            let command = text("command").lines().next().unwrap_or_default().to_string();
            (GLYPH_TERMINAL, Headline::Code(command), description().unwrap_or_else(|| "wants to run a command".into()))
        }
        "edit" | "multiedit" | "notebookedit" | "patch" => {
            let p = path();
            (GLYPH_EDIT, Headline::Text(file(&p)), format!("wants to edit a file in {}", folder(&p)))
        }
        "apply_patch" => {
            // The patch names its files: "*** Update File: path".
            let patch = text("input") + &text("patch") + &text("command");
            let target = patch.lines().find_map(|l| l.split_once(" File: ").map(|(_, p)| p.trim().to_string())).unwrap_or_default();
            (GLYPH_EDIT, Headline::Text(if target.is_empty() { "a patch".into() } else { file(&target) }), "wants to apply a patch".into())
        }
        "write" => {
            let p = path();
            (GLYPH_EDIT, Headline::Text(file(&p)), format!("wants to write a file in {}", folder(&p)))
        }
        "read" => {
            let p = path();
            (GLYPH_READ, Headline::Text(file(&p)), format!("wants to read a file in {}", folder(&p)))
        }
        "webfetch" => {
            let url = text("url");
            let host = url.split("://").nth(1).and_then(|r| r.split('/').next()).unwrap_or(&url).to_string();
            (GLYPH_WEB, Headline::Text(host), "wants to fetch a web page".into())
        }
        "websearch" => (GLYPH_SEARCH, Headline::Text(text("query")), "wants to search the web".into()),
        _ => {
            let name = short_tool_name(tool);
            let detail = match tool.strip_prefix("mcp__").and_then(|r| r.split("__").next()) {
                Some(server) => format!("wants to use {server} (MCP)"),
                None => description().unwrap_or_else(|| "wants to use a tool".into()),
            };
            (GLYPH_TOOL, Headline::Text(name), detail)
        }
    }
}

/// `mcp__figma__get_screenshot` → `get_screenshot`.
fn short_tool_name(tool: &str) -> String {
    tool.rsplit("__").next().unwrap_or(tool).to_string()
}

/// Last folder of the working directory (Codex reports `\\?\D:\...` paths).
fn project_name(cwd: &str, fallback: &str) -> String {
    let cwd = cwd.trim_start_matches(r"\\?\").trim_end_matches(['/', '\\']);
    Path::new(cwd).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| fallback.into())
}

fn format_duration(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m {}s", s / 60, s % 60),
        _ => format!("{}h {}m", s / 3600, s % 3600 / 60),
    }
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
    fn tools_are_described_for_humans() {
        let (_, headline, detail) = describe_tool("Bash", &json!({ "command": "cargo build --release\nmore", "description": "Build release" }));
        assert_eq!(headline, Headline::Code("cargo build --release".into()));
        assert_eq!(detail, "Build release");

        let (_, headline, detail) = describe_tool("Edit", &json!({ "file_path": "D:\\ReUI\\crates\\reui-hub\\src\\main.rs" }));
        assert_eq!(headline, Headline::Text("main.rs".into()));
        assert_eq!(detail, "wants to edit a file in src");

        let (_, headline, _) = describe_tool("WebFetch", &json!({ "url": "https://code.claude.com/docs/en/hooks" }));
        assert_eq!(headline, Headline::Text("code.claude.com".into()));

        let (_, headline, detail) = describe_tool("mcp__figma__get_screenshot", &json!({}));
        assert_eq!(headline, Headline::Text("get_screenshot".into()));
        assert_eq!(detail, "wants to use figma (MCP)");
    }

    #[test]
    fn codex_and_opencode_tools_read_the_same_way() {
        // Codex: argv-style command, apply_patch with the file in the patch.
        let (_, headline, _) = describe_tool("shell", &json!({ "command": ["cargo", "test"] }));
        assert_eq!(headline, Headline::Code("cargo test".into()));
        let (_, headline, _) = describe_tool("apply_patch", &json!({ "input": "*** Begin Patch\n*** Update File: src/pill.rs\n@@" }));
        assert_eq!(headline, Headline::Text("pill.rs".into()));
        // OpenCode: lowercase tools, camelCase paths, the plugin passes its title as description.
        let (_, headline, detail) = describe_tool("bash", &json!({ "command": "npm run build", "description": "Build the site" }));
        assert_eq!((headline, detail.as_str()), (Headline::Code("npm run build".into()), "Build the site"));
        let (_, headline, _) = describe_tool("edit", &json!({ "filePath": "D:/site/src/app.ts" }));
        assert_eq!(headline, Headline::Text("app.ts".into()));
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

    #[test]
    fn durations_read_naturally() {
        assert_eq!(format_duration(Duration::from_secs(45)), "45s");
        assert_eq!(format_duration(Duration::from_secs(134)), "2m 14s");
        assert_eq!(format_duration(Duration::from_secs(3780)), "1h 3m");
    }
}
