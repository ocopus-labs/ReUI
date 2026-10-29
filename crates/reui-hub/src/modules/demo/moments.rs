//! The demo's agent moments, so the island's moment components can be seen and used: a
//! permission with a multi-line command in a body block, a three-step question (one step
//! multi-select) and a plan to approve. They answer nothing real; each goes when answered
//! or after a while, and the next kind comes a little later.

use std::time::Duration;

use reui_protocol::{Art, BlockKind, Button, ButtonContent, ButtonRole, CardSize, Choice, CompactView, Context, Element, ExpandedView, Headline, Pager, Priority, Row, Tone};
use tokio::time::Instant;

use super::{AGENT_ACCENT, GLYPH_TERMINAL};
use crate::hub::{ModuleContext, View};

const GLYPH_PLAN: char = '\u{E8FD}';

const PERMISSION_TIMEOUT: Duration = Duration::from_secs(20);
const ASK_TIMEOUT: Duration = Duration::from_secs(60);

/// Longer than the block shows, so it scrolls.
const SCRIPT: &str = "cargo fmt --all --check\ncargo clippy --workspace -- -D warnings\ncargo test --workspace\ncargo build --release\npwsh scripts/package.ps1 -OutDir dist\nGet-ChildItem dist | Format-Table Name, Length\n$sum = Get-FileHash dist\\ReUI-setup.exe -Algorithm SHA256\n$sum.Hash | Set-Content dist\\ReUI-setup.exe.sha256\ngit tag -a v0.9.0 -m \"ReUI 0.9.0\"\nWrite-Host \"Packaged $($sum.Hash.Substring(0, 12))\"";

const PLAN: &str = "Add the timers launcher\n\n1. Parse lengths like \"10m tea\" and \"1h 30m\" in parse.rs, with tests for every form the PRD lists.\n2. Show the field in the island on Win+Ctrl+T, with the presets as tabs.\n3. Start the timer on Enter and fold the card back to compact.\n4. Keep running timers in timer-state.json so a restart doesn't lose them.\n5. Document the shortcut in the guide.\n6. Add Settings → Timers: the shortcut, the finish sound, how long it rings.\n7. Run the timers through the design review sheet and fix what it shows.\n8. Note it in the changelog.";

struct Step {
    question: &'static str,
    header: &'static str,
    multi: bool,
    options: &'static [(&'static str, &'static str)],
}

const STEPS: [Step; 3] = [
    Step { question: "Which framework should the site use?", header: "Framework", multi: false, options: &[("Astro", "Static pages, islands of JavaScript"), ("SvelteKit", "Full-stack, compiled components"), ("Next.js", "React with server components")] },
    Step {
        question: "Which features do you want in the first version?",
        header: "Features",
        multi: true,
        options: &[("Blog", "Markdown posts with tags"), ("Search", "Client-side, no server"), ("Dark mode", "Follows the system"), ("Analytics", "Privacy-friendly, no cookies")],
    },
    Step { question: "Deploy it when it builds?", header: "Release", multi: false, options: &[("Yes", "To the preview URL"), ("Not yet", "Keep it local for now")] },
];

#[derive(Debug)]
enum Kind {
    Permission,
    /// The step showing, and the choices picked on each step so far.
    Question { step: usize, picked: Vec<Vec<bool>> },
    Plan,
}

/// The moment waiting (if any) and when it gives up.
pub(super) struct Moments {
    count: usize,
    current: Option<(String, Kind, Instant)>,
    started: Instant,
}

impl Moments {
    pub(super) fn new() -> Self {
        Self { count: 0, current: None, started: Instant::now() }
    }

    pub(super) fn deadline(&self) -> Option<Instant> {
        self.current.as_ref().map(|(_, _, at)| *at)
    }

    pub(super) fn owns(&self, key: &str) -> bool {
        self.current.as_ref().is_some_and(|(k, ..)| k == key)
    }

    /// Asks the next kind of moment.
    pub(super) fn ask(&mut self, ctx: &ModuleContext) {
        let (name, kind, wait) = match self.count % 3 {
            0 => ("permission", Kind::Permission, PERMISSION_TIMEOUT),
            1 => ("question", Kind::Question { step: 0, picked: STEPS.iter().map(|s| vec![false; s.options.len()]).collect() }, ASK_TIMEOUT),
            _ => ("plan", Kind::Plan, ASK_TIMEOUT),
        };
        self.count += 1;
        self.started = Instant::now();
        let key = format!("{name}-{}", self.count);
        println!("demo: {key} asking");
        self.current = Some((key, kind, Instant::now() + wait));
        self.publish(ctx);
    }

    /// Nobody answered: gone, as a real moment fails open to the agent's own prompt.
    pub(super) fn time_out(&mut self, ctx: &ModuleContext) {
        if let Some((key, ..)) = self.current.take() {
            println!("demo: {key} timed out; failing open to the agent's own prompt");
            ctx.retract(&key);
        }
    }

    /// An action on the moment. `true` once it's settled (retracted).
    pub(super) fn act(&mut self, ctx: &ModuleContext, action: &str) -> bool {
        let Some((key, kind, _)) = &mut self.current else { return false };
        let done = match kind {
            Kind::Permission | Kind::Plan => true,
            Kind::Question { step, picked } => match action {
                "terminal" => true,
                "back" => {
                    *step = step.saturating_sub(1);
                    false
                }
                // The pager's › and the Next / Send button: only once this step has an answer.
                "next" if picked[*step].iter().any(|p| *p) => {
                    *step += 1;
                    *step == STEPS.len()
                }
                "next" => false,
                pick => match pick.strip_prefix("pick:").and_then(|i| i.parse::<usize>().ok()) {
                    Some(i) if i < picked[*step].len() => {
                        if STEPS[*step].multi {
                            picked[*step][i] = !picked[*step][i];
                            false
                        } else {
                            picked[*step] = (0..picked[*step].len()).map(|j| j == i).collect();
                            *step += 1;
                            *step == STEPS.len()
                        }
                    }
                    _ => false,
                },
            },
        };
        println!("demo: {key} answered from the island: {action}");
        if done {
            if let Some((key, Kind::Question { picked, .. }, _)) = &self.current {
                let answers: Vec<String> = STEPS.iter().zip(picked).map(|(s, p)| s.options.iter().zip(p).filter(|(_, on)| **on).map(|(o, _)| o.0).collect::<Vec<_>>().join(", ")).collect();
                println!("demo: {key} answers: {answers:?}");
            }
            let (key, ..) = self.current.take().unwrap();
            ctx.retract(&key);
        } else {
            self.publish(ctx);
        }
        done
    }

    fn publish(&self, ctx: &ModuleContext) {
        let Some((key, kind, _)) = &self.current else { return };
        let text = |action: &str, label: &str, role| Button { action: action.into(), content: ButtonContent::Text(label.into()), role };
        let mark = Element::Art(Art::Glyph(GLYPH_TERMINAL));
        let elapsed = self.started.elapsed().as_secs() / 60 + 3;
        let context = |prompt: &str| Some(Context { mark: mark.clone(), title: "ReUI".into(), facts: vec![prompt.into(), format!("{elapsed}m")] });
        let glyph = |g| Element::Glyph { glyph: g, tone: Tone::Accent };
        let (compact, expanded) = match kind {
            Kind::Permission => (
                "Allow?",
                ExpandedView {
                    context: context("get the release build green"),
                    leading: glyph(GLYPH_TERMINAL),
                    eyebrow: Some("Permission".into()),
                    // The body has the script, so the hero says what it's for (as
                    // `agent::describe` does).
                    headline: Headline::Text("Check, test and package the release".into()),
                    detail: Some(format!("PowerShell · {} lines", SCRIPT.lines().count())),
                    trailing: Element::Empty,
                    rows: vec![
                        Row::Block { text: SCRIPT.into(), kind: BlockKind::Code },
                        Row::Buttons(vec![text("deny", "Deny", ButtonRole::Secondary), text("always", "Always", ButtonRole::Secondary), text("allow", "Allow", ButtonRole::Accent)]),
                    ],
                    size: CardSize::Roomy,
                    tint: false,
                    pager: None,
                    open: None,
                },
            ),
            Kind::Question { step, picked } => {
                let s = &STEPS[*step];
                let options = s
                    .options
                    .iter()
                    .zip(&picked[*step])
                    .enumerate()
                    .map(|(i, ((label, detail), on))| Choice { label: (*label).into(), detail: Some((*detail).into()), selected: *on, action: format!("pick:{i}") })
                    .collect();
                let mut buttons = vec![text("terminal", "Answer in terminal", ButtonRole::Plain)];
                if s.multi {
                    buttons.push(text("next", if *step + 1 == STEPS.len() { "Send" } else { "Next" }, ButtonRole::Accent));
                }
                (
                    "Question",
                    ExpandedView {
                        context: context("set up the project site"),
                        // The eyebrow says it's a question; no glyph needed.
                        leading: Element::Empty,
                        eyebrow: Some("Question".into()),
                        headline: Headline::Text(s.question.into()),
                        detail: Some(s.header.into()),
                        trailing: Element::Empty,
                        rows: vec![Row::Choices { options, multi: s.multi }, Row::Buttons(buttons)],
                        size: CardSize::Roomy,
                        tint: false,
                        // The steps: "1/3", ‹ back with answers kept.
                        pager: Some(Pager { index: *step as u32, count: STEPS.len() as u32, prev: "back".into(), next: "next".into() }),
                        open: None,
                    },
                )
            }
            Kind::Plan => (
                "Plan",
                ExpandedView {
                    context: context("add the timers launcher"),
                    leading: glyph(GLYPH_PLAN),
                    eyebrow: Some("Plan".into()),
                    headline: Headline::Text(PLAN.lines().next().unwrap_or("Plan").into()),
                    detail: Some("plan.md · 8 steps".into()),
                    trailing: Element::Empty,
                    rows: vec![
                        Row::Block { text: PLAN.lines().skip(2).collect::<Vec<_>>().join("\n"), kind: BlockKind::Prose },
                        Row::Buttons(vec![text("keep", "Keep planning", ButtonRole::Secondary), text("approve", "Approve", ButtonRole::Accent)]),
                    ],
                    size: CardSize::Roomy,
                    tint: false,
                    pager: None,
                    open: None,
                },
            ),
        };
        ctx.publish(
            key,
            Priority::Decision,
            View {
                accent: Some(AGENT_ACCENT),
                audio: None,
                compact: CompactView { leading: mark.clone(), trailing: Element::Text { text: compact.into(), tone: Tone::Accent } },
                minimal: mark,
                expanded: Some(expanded),
            },
        );
    }
}
