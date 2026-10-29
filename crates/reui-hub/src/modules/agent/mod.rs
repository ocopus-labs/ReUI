//! Coding agents (Claude Code, Codex, OpenCode): live sessions, the moments that need you
//! and token usage, one module instance per agent with a [`Profile`] for what differs.
//!
//! Fed by `reui-bridge <agent>` hook events. Claude Code and Codex send their own hook
//! JSON; OpenCode's plugin (installed by `reui-bridge install opencode`) translates its
//! events into the same shape, so one state machine serves all three.
//!
//! Every session is tracked through its turn: thinking → running a tool (or compacting,
//! waiting for you) → done or failed. Only what needs you is published by default (a
//! failed turn, and the moments below); working phases and finished turns show only when
//! `coding.json` asks for them (`settings.rs`, PRD agent moments M1), so a busy agent costs
//! the island nothing.
//!
//! The moments (permission requests, questions, Claude's plans and MCP forms and links)
//! wait in one queue and show as one decision card with the session's strip (project,
//! prompt, time in turn), so two sessions never get confused; several page on the strip,
//! oldest first. Answering one replies to its waiting bridge; if the bridge goes away
//! (answered in the agent's own UI, timed out) the moment goes too. Every wait ends before
//! the agent's hook timeout. Codex's questions show read-only: no hook can answer them.
//!
//! Each hook event is also noted (throttled) in `agents.json` for Settings' status line,
//! and `reui-bridge test <agent>` (Settings' "Send test event") gets a short "connected"
//! confirmation instead of a session.
//!
//! Files: `events.rs` (a hook event into sessions and moments), `queue.rs` (the moments
//! in line, the async and blocking duplicates made one), `moments.rs` (each moment's steps
//! and answers), `form.rs` (an MCP form being filled in), `schema.rs` (its schema read as
//! fields), `link.rs` (an MCP link opened), `answer.rs` (the reply JSON), `card.rs` (a
//! moment's card), `plan.rs` (a plan read for its card), `view.rs` (what shows),
//! `describe.rs` (a tool request for humans), `settings.rs` (`coding.json`), `usage.rs`
//! (token usage), `status.rs` (`agents.json`, the test event).

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reui_protocol::{Art, Element, Rgb, ACTION_COLLAPSED, ACTION_EXPANDED, ACTION_SEARCH, ACTION_SUBMIT};
use tokio::sync::oneshot;
use tokio::time::{sleep_until, Instant};

mod answer;
mod card;
mod describe;
mod events;
mod form;
mod link;
mod moments;
mod plan;
mod queue;
mod schema;
mod settings;
mod status;
mod terminal;
mod usage;
mod view;

use self::card::{PAGE_NEXT, PAGE_PREV};
use self::describe::{short_tool_name, GLYPH_TERMINAL};
use self::moments::{Kind, Outcome};
use self::queue::Queue;
use self::settings::Settings;
use self::status::Throttle;
use self::terminal::Origin;
use self::usage::Ledger;
use self::view::{publish_moments, publish_session, publish_test, TEST_KEY};
use super::Module;
use crate::hub::{ModuleContext, ModuleInput};

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
    /// What a sentence calls it ("Tell Claude what to do instead").
    pub short: &'static str,
    pub accent: Rgb,
    pub logo: Option<&'static [u8]>,
    /// Tile glyph when there's no logo.
    pub glyph: char,
    /// Whether the agent accepts "always allow" (Claude Code's `updatedPermissions`,
    /// OpenCode's `always` reply).
    pub can_always: bool,
    pub usage: UsageSource,
}

pub const CLAUDE: Profile = Profile {
    name: "claude",
    display: "Claude Code",
    short: "Claude",
    accent: Rgb(217, 119, 87),
    logo: Some(include_bytes!("../../../../../design/assets/agents/claude.svg")),
    glyph: GLYPH_TERMINAL,
    can_always: true,
    usage: UsageSource::ClaudeTranscripts,
};

pub const CODEX: Profile = Profile {
    name: "codex",
    display: "Codex",
    short: "Codex",
    accent: Rgb(16, 163, 127),
    logo: Some(include_bytes!("../../../../../design/assets/agents/openai.svg")),
    glyph: GLYPH_TERMINAL,
    // Codex reserves `updatedPermissions` and fails closed on it today.
    can_always: false,
    usage: UsageSource::CodexThreads,
};

pub const OPENCODE: Profile = Profile {
    name: "opencode",
    display: "OpenCode",
    short: "OpenCode",
    accent: Rgb(130, 170, 255),
    logo: Some(include_bytes!("../../../../../design/assets/agents/opencode.svg")),
    glyph: GLYPH_TERMINAL,
    // Replies take "always" for the patterns `permission.asked` offers (the plugin sends
    // them as the suggestion).
    can_always: true,
    usage: UsageSource::OpenCodeSessions,
};

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
    /// A permission, question or plan waits for you (its moment shows it).
    WaitingPermission,
    /// Asking you something we can't show as a moment (another agent's question, an MCP
    /// form); the question when the agent said it.
    NeedsInput(Option<String>),
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
    /// Where its terminal is (the bridge's last parent chain), for "Open terminal".
    origin: Option<Origin>,
}

/// Fresh tokens (cache reads left out where the agent reports them).
#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct Usage {
    session: Option<u64>,
    today: Option<u64>,
}

/// The moments waiting, and the bridges waiting on them (by hook call id).
#[derive(Default)]
struct Moments {
    queue: Queue,
    replies: HashMap<u64, oneshot::Sender<Option<String>>>,
    /// The moment whose card is published (`view::MOMENT_PREFIX`).
    published: Option<u64>,
}

/// Everything a module instance needs besides its sessions.
struct Agentry {
    profile: &'static Profile,
    mark: Element,
    ledger: Arc<Mutex<Ledger>>,
    /// `agents.json` writes.
    status: Mutex<Throttle>,
    /// `coding.json`: what shows besides what needs you.
    settings: Mutex<Settings>,
}

async fn run(profile: &'static Profile, mut ctx: ModuleContext) {
    let mark = match profile.logo {
        Some(svg) => Element::Art(Art::Logo { image: ctx.image(svg.to_vec()), animated: false }),
        None => Element::Art(Art::Glyph(profile.glyph)),
    };
    let me = Agentry { profile, mark, ledger: Arc::new(Mutex::new(Ledger::default())), status: Mutex::new(Throttle::default()), settings: Mutex::new(Settings::load()) };
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
    let mut moments = Moments::default();
    let mut test_until: Option<Instant> = None;

    loop {
        let next_deadline = moments
            .queue
            .deadline()
            .into_iter()
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
                    events::on_hook(&ctx, &me, call, &mut sessions, &mut moments).await
                }
                Some(ModuleInput::HookGone(id)) => {
                    moments.replies.remove(&id);
                    if let Some(m) = moments.queue.remove(id) {
                        // Answered elsewhere: a permitted tool most likely runs now.
                        let next = match &m.kind {
                            Kind::Permission { tool, .. } => Phase::Tool(short_tool_name(tool)),
                            _ => Phase::Thinking,
                        };
                        resume(&ctx, &me, &mut sessions, &m.session, next);
                        publish_moments(&ctx, &me, &mut moments, &sessions);
                    }
                }
                Some(ModuleInput::Action(action)) if action.key.is_empty() && action.action == "reload" => {
                    *me.settings.lock().unwrap() = Settings::load();
                    for (id, session) in &sessions {
                        publish_session(&ctx, &me, id, session);
                    }
                }
                Some(ModuleInput::Action(action)) if action.key.starts_with(view::MOMENT_PREFIX) => {
                    on_moment_action(&ctx, &me, &action.action, &mut sessions, &mut moments);
                }
                // A failed turn's Open terminal: there, and the alert goes.
                Some(ModuleInput::Action(action)) if action.action == view::ACTION_TERMINAL => {
                    let id = action.key.strip_prefix("session-").unwrap_or_default();
                    if let Some(session) = sessions.get_mut(id) {
                        open_terminal(session);
                        session.phase = None;
                        session.retract_at = None;
                        ctx.retract(&action.key);
                    }
                }
                Some(ModuleInput::Action(_)) => {}
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
                let expired = moments.queue.expire(now);
                for m in &expired {
                    // Silence: the agent falls back to its own prompt.
                    println!("{}: moment {} timed out; no decision", profile.name, m.id);
                    moments.replies.remove(&m.id);
                }
                if !expired.is_empty() {
                    publish_moments(&ctx, &me, &mut moments, &sessions);
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

/// An action on the moments card: the pager, typing in its field, or a moment's own
/// (`<id>:allow`, `alt:<id>:deny`); `submit` is Enter in the field of the one in front.
fn on_moment_action(ctx: &ModuleContext, me: &Agentry, action: &str, sessions: &mut HashMap<String, Session>, moments: &mut Moments) {
    if action == ACTION_EXPANDED || action == ACTION_COLLAPSED {
        return;
    }
    // What was typed stays out of the log.
    if !action.starts_with(ACTION_SEARCH) {
        println!("{}: moment action {action}", me.profile.name);
    }
    let (id, outcome) = if action == PAGE_PREV || action == PAGE_NEXT {
        if moments.queue.page(if action == PAGE_NEXT { 1 } else { -1 }) {
            publish_moments(ctx, me, moments, sessions);
        }
        return;
    } else if let Some(text) = action.strip_prefix(ACTION_SEARCH) {
        // Echoed back as the field's query, so the island knows the module has it.
        let Some(m) = moments.queue.current_mut() else { return };
        m.set_text(text);
        (m.id, Outcome::Changed)
    } else if action == ACTION_SUBMIT {
        let Some(m) = moments.queue.current_mut() else { return };
        (m.id, m.act("submit"))
    } else {
        let Some((id, verb)) = Queue::parse(action) else { return };
        let Some(m) = moments.queue.get_mut(id) else { return };
        let outcome = m.act(&verb);
        // Answer in terminal: the agent's own prompt comes, so bring its window forward.
        if verb.trim_start_matches("alt:") == view::ACTION_TERMINAL && outcome != Outcome::Nothing {
            if let Some(session) = sessions.get(&m.session) {
                open_terminal(session);
            }
        }
        // Open link: the browser first, then the accept (Claude won't open it itself).
        if let (Kind::Link { url, .. }, Outcome::Reply(Some(_))) = (&m.kind, &outcome) {
            if verb == "open" {
                let url = url.clone();
                tokio::task::spawn_blocking(move || link::open(&url));
            }
        }
        (id, outcome)
    };
    match outcome {
        Outcome::Nothing => return,
        Outcome::Changed => {}
        Outcome::Reply(output) => {
            let Some(m) = moments.queue.remove(id) else { return };
            println!("{}: moment {id} answered from the island: {action}", me.profile.name);
            let allowed = output.as_deref().is_some_and(|o| o.contains("\"allow\""));
            if let Some(reply) = moments.replies.remove(&id) {
                _ = reply.send(output);
            }
            let next = match &m.kind {
                Kind::Permission { tool, .. } if allowed => Phase::Tool(short_tool_name(tool)),
                _ => Phase::Thinking,
            };
            resume(ctx, me, sessions, &m.session, next);
        }
        Outcome::Dismissed => {
            moments.queue.remove(id);
        }
    }
    publish_moments(ctx, me, moments, sessions);
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

/// Brings the window the session's agent runs in forward, when the bridge told us where.
/// The island allowed it with the click (`AllowSetForegroundWindow`).
fn open_terminal(session: &Session) {
    if let Some(origin) = session.origin.clone() {
        tokio::task::spawn_blocking(move || terminal::open(&origin));
    }
}

/// Moves a session on after its moment is settled.
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

fn session_key(id: &str) -> String {
    format!("session-{id}")
}
