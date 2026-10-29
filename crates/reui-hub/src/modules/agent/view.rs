//! How a session and a permission request look on the island: which phases show at all
//! (per `coding.json`), the status per phase, the session strip, and the published
//! activities. What a tool request is about is `describe.rs`.

use std::time::Duration;

use reui_protocol::{Button, ButtonContent, ButtonRole, CardSize, CompactView, Context, Element, ExpandedView, Headline, Priority, Row, Tone};
use serde_json::Value;

use super::describe::describe_tool;
use super::settings::{Settings, ShowSessions};
use super::usage::format_tokens;
use super::{permission_key, session_key, Agentry, Phase, Session};
use crate::hub::{ModuleContext, View};

const GLYPH_DONE: char = '\u{E73E}';
const GLYPH_FAILED: char = '\u{E7BA}';
const GLYPH_QUESTION: char = '\u{E897}';
const GLYPH_DENY: char = '\u{E711}';

/// Whether a session in this phase is in the island at all. By default only what needs
/// you: a question and a failed turn (permissions are their own activities and always
/// show). Working phases and finished turns only when the settings ask for them.
pub(super) fn shown(phase: &Phase, settings: &Settings) -> bool {
    match phase {
        Phase::Thinking | Phase::Tool(_) | Phase::Compacting | Phase::WaitingPermission => settings.show_sessions == ShowSessions::Working,
        Phase::NeedsInput(_) | Phase::Failed { .. } => true,
        Phase::Done { .. } => settings.show_finished,
    }
}

/// (priority, compact trailing, status line) for a working or finished phase. Text is
/// quiet; colour only where it's your move.
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
        Phase::NeedsInput(_) | Phase::Failed { .. } => (Priority::Alert, Element::Empty, String::new()),
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
            (Priority::Brief, Element::Glyph { glyph: GLYPH_DONE, tone: Tone::Secondary }, parts.join(" · "))
        }
    }
}

/// The session strip: the agent's mark, the project, what you asked, how long the turn has
/// run.
fn context(me: &Agentry, session: &Session) -> Context {
    let mut facts: Vec<String> = session.prompt.iter().cloned().collect();
    if let Some(started) = session.turn_started {
        facts.push(short_duration(started.elapsed()));
    }
    Context { mark: me.mark.clone(), title: session.project.clone(), facts }
}

/// A card that needs you: the strip, then eyebrow → hero → detail in the roomy width.
fn moment(me: &Agentry, session: &Session, leading: Element, eyebrow: &str, headline: Headline, detail: Option<String>, rows: Vec<Row>) -> ExpandedView {
    ExpandedView {
        context: Some(context(me, session)),
        leading,
        eyebrow: Some(eyebrow.into()),
        headline,
        detail,
        trailing: Element::Empty,
        rows,
        size: CardSize::Roomy,
        tint: false,
        pager: None,
        open: None,
    }
}

/// Publishes the session's activity, or takes it away when its phase isn't shown.
pub(super) fn publish_session(ctx: &ModuleContext, me: &Agentry, id: &str, session: &Session) {
    let Some(phase) = &session.phase else { return };
    if !shown(phase, &me.settings.lock().unwrap()) {
        ctx.retract(&session_key(id));
        return;
    }
    let (priority, trailing, expanded) = match phase {
        Phase::NeedsInput(question) => (
            Priority::Alert,
            Element::Text { text: "Question".into(), tone: Tone::Accent },
            // Not answerable here yet (agent moments M2): the question, read-only.
            moment(me, session, Element::Glyph { glyph: GLYPH_QUESTION, tone: Tone::Accent }, "Question", Headline::Text(question.clone().unwrap_or_else(|| "Question".into())), None, Vec::new()),
        ),
        Phase::Failed { reason } => (
            Priority::Alert,
            Element::Glyph { glyph: GLYPH_FAILED, tone: Tone::Negative },
            moment(me, session, Element::Glyph { glyph: GLYPH_FAILED, tone: Tone::Negative }, "Stopped", Headline::Text(reason.clone().unwrap_or_else(|| "The turn failed".into())), None, Vec::new()),
        ),
        _ => {
            let (priority, trailing, status) = present(phase, session);
            let view = ExpandedView {
                context: None,
                leading: me.mark.clone(),
                eyebrow: Some(format!("{} · {}", me.profile.display, session.project)),
                // What you asked is the most useful thing to read; the project otherwise.
                headline: Headline::Text(session.prompt.clone().unwrap_or_else(|| session.project.clone())),
                detail: Some(status),
                trailing: Element::Empty,
                size: CardSize::Regular,
                tint: false,
                pager: None,
                open: None,
                rows: Vec::new(),
            };
            (priority, trailing, view)
        }
    };
    ctx.publish(
        &session_key(id),
        priority,
        View {
            accent: Some(me.profile.accent),
            audio: None,
            compact: CompactView { leading: me.mark.clone(), trailing },
            minimal: me.mark.clone(),
            expanded: Some(expanded),
        },
    );
}

pub(super) const TEST_KEY: &str = "test";

/// The answer to `reui-bridge test`: "Claude Code · Connected", briefly. Not a session.
pub(super) fn publish_test(ctx: &ModuleContext, me: &Agentry) {
    let done = Element::Glyph { glyph: GLYPH_DONE, tone: Tone::Positive };
    ctx.publish(
        TEST_KEY,
        Priority::Brief,
        View {
            accent: Some(me.profile.accent),
            audio: None,
            compact: CompactView { leading: me.mark.clone(), trailing: done.clone() },
            minimal: me.mark.clone(),
            expanded: Some(ExpandedView {
                leading: me.mark.clone(),
                eyebrow: Some(me.profile.display.to_string()),
                headline: Headline::Text("Connected".into()),
                detail: None,
                trailing: done,
                context: None,
                size: CardSize::Regular,
                tint: false,
                pager: None,
                open: None,
                rows: Vec::new(),
            }),
        },
    );
}

/// A permission request (PRD agent moments §5.3): the strip, "Permission", the tool's
/// glyph and hero, the facts, the body the hero cuts off, and Deny / Always / Allow with
/// Allow as the tinted main action. A short request with nothing more to say and no
/// Always keeps the card small: Deny and Allow as two circles beside the hero.
pub(super) fn publish_request(ctx: &ModuleContext, me: &Agentry, id: u64, session: &Session, tool: &str, input: &Value, can_always: bool) {
    let described = describe_tool(tool, input);
    let text = |action: &str, label: &str, role| Button { action: action.into(), content: ButtonContent::Text(label.into()), role };
    let glyph = |action: &str, glyph: char, role| Button { action: action.into(), content: ButtonContent::Glyph(glyph), role };
    let mut rows = Vec::new();
    if let Some((body, kind)) = described.body.clone() {
        rows.push(Row::Block { text: body, kind });
    }
    let buttons = if rows.is_empty() && !can_always {
        vec![glyph("deny", GLYPH_DENY, ButtonRole::Destructive), glyph("allow", GLYPH_DONE, ButtonRole::Accent)]
    } else {
        let mut b = vec![text("deny", "Deny", ButtonRole::Secondary)];
        if can_always {
            b.push(text("always", "Always", ButtonRole::Secondary));
        }
        b.push(text("allow", "Allow", ButtonRole::Accent));
        b
    };
    rows.push(Row::Buttons(buttons));
    let leading = Element::Glyph { glyph: described.glyph, tone: Tone::Accent };
    ctx.publish(
        &permission_key(id),
        Priority::Decision,
        View {
            accent: Some(me.profile.accent),
            audio: None,
            compact: CompactView { leading: me.mark.clone(), trailing: Element::Text { text: "Allow?".into(), tone: Tone::Accent } },
            minimal: me.mark.clone(),
            expanded: Some(moment(me, session, leading, "Permission", described.headline, described.detail, rows)),
        },
    );
}

fn format_duration(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m {}s", s / 60, s % 60),
        _ => format!("{}h {}m", s / 3600, s % 3600 / 60),
    }
}

/// How long a turn has run, as a fact in the strip: "40s", "4m", "1h 3m".
fn short_duration(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m", s / 60),
        _ => format!("{}h {}m", s / 3600, s % 3600 / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_what_needs_you_shows_by_default() {
        let quiet = Settings::default();
        let working = Settings { show_sessions: ShowSessions::Working, ..Settings::default() };
        let finished = Settings { show_finished: true, ..Settings::default() };
        for phase in [Phase::Thinking, Phase::Tool("Bash".into()), Phase::Compacting, Phase::WaitingPermission] {
            assert!(!shown(&phase, &quiet) && shown(&phase, &working) && !shown(&phase, &finished), "{phase:?}");
        }
        let done = Phase::Done { took: None };
        assert!(!shown(&done, &quiet) && !shown(&done, &working) && shown(&done, &finished));
        for phase in [Phase::Failed { reason: None }, Phase::NeedsInput(None)] {
            assert!(shown(&phase, &quiet), "{phase:?}");
        }
    }

    #[test]
    fn durations_read_naturally() {
        assert_eq!(format_duration(Duration::from_secs(45)), "45s");
        assert_eq!(format_duration(Duration::from_secs(134)), "2m 14s");
        assert_eq!(format_duration(Duration::from_secs(3780)), "1h 3m");
        assert_eq!(short_duration(Duration::from_secs(134)), "2m");
        assert_eq!(short_duration(Duration::from_secs(40)), "40s");
    }
}
