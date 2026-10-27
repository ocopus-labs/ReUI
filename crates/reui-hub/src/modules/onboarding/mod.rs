//! First run and one-time tips (PRD product shell, P2).
//!
//! **First run** (no `onboarding.json`, or `welcomed: false`): the island introduces itself
//! with a 5-second demo (a Now Playing-style card that opens by itself, then folds), then
//! the one-time "Hover me" hint until the first hover (at most 30 s), then the welcome
//! window (`reui-settings.exe --welcome`). `welcomed` is saved as the welcome opens, so
//! closing it with ✕ counts (P-1). A brand-new user also gets "When active" visibility.
//!
//! **Tips and offers**, each once ever: the clipboard hotkey after the first three copies;
//! replacing Windows' volume pop-up at the first volume key; connecting a coding agent
//! when its process is first seen without hooks; synced lyrics at the first song that
//! could have them. Other modules report those moments with one call to [`note`]. Tips
//! queue (never two at once), wait out fullscreen and "Pause ReUI", and are dropped if the
//! setting was changed meanwhile. An offer ignored until it times out counts as "Not now";
//! everything stays reachable in Settings.
//!
//! Files: `state.rs` (the record and the queue, pure), `views.rs` (what the island
//! shows), `system.rs` (fullscreen and pause, agent processes, Connect, the welcome).

mod state;
mod system;
mod views;

use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;
use std::time::Duration;

use reui_protocol::Priority;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::time::{sleep_until, Instant};

use self::state::{Answer, Queue, Record, Tip};
use super::agent::{Profile, CLAUDE, CODEX, OPENCODE};
use super::Module;
use crate::hub::{ModuleContext, ModuleInput};
use crate::paths;

/// A moment another module saw that may deserve a tip.
#[derive(Debug)]
pub enum Moment {
    /// Something was copied (the toast showed). `hotkey` is the history hotkey's label,
    /// `None` when it couldn't be registered.
    Copy { hotkey: Option<String> },
    /// A volume change the HUD showed; `replaced` when Windows' pop-up is already hidden.
    Volume { replaced: bool },
    /// A track that could have synced lyrics started while lyrics are off.
    Song,
}

static MOMENTS: OnceLock<UnboundedSender<Moment>> = OnceLock::new();

/// Reports a moment to onboarding (a no-op when it isn't running). Cheap: modules call it
/// every time; onboarding decides whether anything is still due.
pub fn note(moment: Moment) {
    if let Some(tx) = MOMENTS.get() {
        _ = tx.send(moment);
    }
}

/// Lets the island show up before the demo starts.
const ISLAND_SETTLE: Duration = Duration::from_millis(2500);
/// Demo: compact for a moment, then open, then folded, then gone (5 s in all).
const DEMO_OPEN_AFTER: Duration = Duration::from_millis(700);
const DEMO_FOLD_AFTER: Duration = Duration::from_millis(3600);
const DEMO_GONE_AFTER: Duration = Duration::from_millis(700);
const HINT_MAX: Duration = Duration::from_secs(30);
/// When a tip isn't welcome yet (fullscreen, paused, first run still going), look again.
const RETRY: Duration = Duration::from_secs(10);
/// Between two tips.
const GAP: Duration = Duration::from_secs(3);
const TIP_SHOW: Duration = Duration::from_secs(6);
const OFFER_SHOW: Duration = Duration::from_secs(45);
const CONFIRM_SHOW: Duration = Duration::from_secs(4);
/// How often to look for coding agents' processes, only while some are still to offer.
const AGENT_SCAN: Duration = Duration::from_secs(30);

const DEMO: &str = "demo";
const HINT: &str = "hint";
const TIP: &str = "tip";
const CONFIRM: &str = "confirm";

/// After the moment that triggered it: let the moment's own toast or HUD finish first.
fn settle(tip: &Tip) -> Duration {
    match tip {
        Tip::Hotkey(_) => Duration::from_millis(2800),
        Tip::Flyout => Duration::from_millis(2000),
        Tip::Lyrics => Duration::from_secs(4),
        Tip::Agent(_) => Duration::ZERO,
    }
}

fn profile(agent: &str) -> &'static Profile {
    match agent {
        "codex" => &CODEX,
        "opencode" => &OPENCODE,
        _ => &CLAUDE,
    }
}

pub struct Onboarding {
    record: Record,
    /// Run the first-run demo and welcome.
    first_run: bool,
    /// The coding agent modules run, so offering them makes sense.
    agents: bool,
    moments: UnboundedReceiver<Moment>,
}

impl Onboarding {
    /// Reads what's been shown and, for a brand-new user, writes the "When active" default
    /// before the island reads its settings. `force_first_run` replays the demo and welcome.
    pub fn new(agents: bool, force_first_run: bool) -> Self {
        let config = paths::config();
        let record = Record::load(&config.join("onboarding.json"));
        if !record.welcomed && system::new_user_defaults(&config) {
            println!("onboarding: new user; island shows when active");
        }
        let (tx, moments) = unbounded_channel();
        _ = MOMENTS.set(tx);
        Self { first_run: force_first_run || !record.welcomed, record, agents, moments }
    }
}

impl Module for Onboarding {
    fn name(&self) -> &'static str {
        "onboarding"
    }

    fn run(self: Box<Self>, ctx: ModuleContext) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(run(ctx, *self))
    }
}

/// Where the first run is.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    Start,
    /// The demo is in compact; it opens next.
    Demo,
    DemoOpen,
    DemoFolded,
    Hint,
    Done,
}

struct App {
    ctx: ModuleContext,
    record: Record,
    queue: Queue,
    phase: Phase,
    phase_at: Option<Instant>,
    demo_started: u64,
    /// When to try showing the next waiting tip.
    next_try: Option<Instant>,
    /// When the tip on screen goes (an offer unanswered by then counts as "Not now").
    hide_at: Option<Instant>,
    confirm_until: Option<Instant>,
    agents: bool,
    next_scan: Option<Instant>,
}

async fn run(ctx: ModuleContext, me: Onboarding) {
    let Onboarding { record, first_run, agents, mut moments } = me;
    let now = Instant::now();
    let mut app = App {
        ctx,
        record,
        queue: Queue::default(),
        phase: if first_run { Phase::Start } else { Phase::Done },
        phase_at: first_run.then(|| now + ISLAND_SETTLE),
        demo_started: 0,
        next_try: None,
        hide_at: None,
        confirm_until: None,
        agents,
        next_scan: None,
    };
    app.schedule_scan(now + ISLAND_SETTLE);
    loop {
        let (phase_at, next_try, hide_at, confirm_until, next_scan) = (app.phase_at, app.next_try, app.hide_at, app.confirm_until, app.next_scan);
        tokio::select! {
            Some(moment) = moments.recv() => app.on_moment(moment),
            input = app.ctx.next() => match input {
                Some(ModuleInput::Action(action)) => app.on_action(&action.key, &action.action).await,
                Some(_) => {}
                None => break,
            },
            _ = sleep_until(phase_at.unwrap_or_else(Instant::now)), if phase_at.is_some() => app.step_first_run().await,
            _ = sleep_until(next_try.unwrap_or_else(Instant::now)), if next_try.is_some() => app.try_show().await,
            _ = sleep_until(hide_at.unwrap_or_else(Instant::now)), if hide_at.is_some() => app.timed_out().await,
            _ = sleep_until(confirm_until.unwrap_or_else(Instant::now)), if confirm_until.is_some() => {
                app.confirm_until = None;
                app.ctx.retract(CONFIRM);
            }
            _ = sleep_until(next_scan.unwrap_or_else(Instant::now)), if next_scan.is_some() => app.scan_agents().await,
        }
    }
}

impl App {
    async fn save(&self) {
        let record = self.record.clone();
        let path = paths::config().join("onboarding.json");
        let saved = tokio::task::spawn_blocking(move || record.save_to(&path)).await;
        if let Ok(Err(e)) = saved {
            eprintln!("onboarding: can't save: {e}");
        }
    }

    // ----- First run -----

    async fn step_first_run(&mut self) {
        let now = Instant::now();
        self.phase_at = None;
        match self.phase {
            Phase::Start => {
                if tokio::task::spawn_blocking(system::quiet).await.unwrap_or(false) {
                    self.phase_at = Some(now + RETRY);
                    return;
                }
                self.demo_started = reui_protocol::unix_ms();
                self.ctx.publish(DEMO, Priority::Ongoing, views::demo(true, self.demo_started));
                self.go(Phase::Demo, DEMO_OPEN_AFTER);
            }
            Phase::Demo => {
                self.ctx.focus(DEMO);
                self.go(Phase::DemoOpen, DEMO_FOLD_AFTER);
            }
            Phase::DemoOpen => {
                // Without its card the island folds back to compact.
                self.ctx.publish(DEMO, Priority::Ongoing, views::demo(false, self.demo_started));
                self.go(Phase::DemoFolded, DEMO_GONE_AFTER);
            }
            Phase::DemoFolded => {
                self.ctx.retract(DEMO);
                // Alert, so it takes the pill even beside music (a brief would wait in the bubble).
                self.ctx.publish(HINT, Priority::Alert, views::hint());
                self.go(Phase::Hint, HINT_MAX);
            }
            Phase::Hint => self.welcome("no hover").await,
            Phase::Done => {}
        }
    }

    fn go(&mut self, phase: Phase, after: Duration) {
        self.phase = phase;
        self.phase_at = Some(Instant::now() + after);
    }

    /// The hint is done (hovered, clicked or timed out): open the welcome, once ever.
    async fn welcome(&mut self, why: &str) {
        println!("onboarding: hint done ({why}); opening the welcome");
        self.ctx.retract(HINT);
        self.phase = Phase::Done;
        self.phase_at = None;
        self.record.welcomed = true;
        self.save().await;
        tokio::task::spawn_blocking(system::open_welcome);
        if self.queue.is_waiting() {
            self.next_try = Some(Instant::now() + GAP);
        }
    }

    // ----- Tips -----

    fn on_moment(&mut self, moment: Moment) {
        let tip = match moment {
            Moment::Copy { hotkey: Some(label) } => Tip::Hotkey(label),
            Moment::Volume { replaced: false } => Tip::Flyout,
            Moment::Song => Tip::Lyrics,
            _ => return,
        };
        self.enqueue(tip);
    }

    fn enqueue(&mut self, tip: Tip) {
        let at = Instant::now() + settle(&tip);
        if self.queue.push(tip, &self.record) {
            self.next_try = Some(self.next_try.map_or(at, |t| t.max(at)));
        }
    }

    async fn try_show(&mut self) {
        self.next_try = None;
        if self.phase != Phase::Done {
            self.next_try = Some(Instant::now() + RETRY);
            return;
        }
        let quiet = tokio::task::spawn_blocking(system::quiet).await.unwrap_or(true);
        while let Some(tip) = self.queue.next(quiet, &self.record) {
            if self.still_relevant(&tip).await {
                self.show(&tip);
                self.record.shown(&tip);
                self.save().await;
                return;
            }
            self.queue.finish();
        }
        if quiet && self.queue.is_waiting() {
            self.next_try = Some(Instant::now() + RETRY);
        }
    }

    /// The setting may have been changed in Settings since the moment.
    async fn still_relevant(&self, tip: &Tip) -> bool {
        let tip = tip.clone();
        tokio::task::spawn_blocking(move || match tip {
            Tip::Hotkey(_) => true,
            Tip::Flyout => !super::osd::replaces_windows_flyout(),
            Tip::Lyrics => !super::media::lyrics_on(),
            Tip::Agent(agent) => !system::connected(agent, &paths::home()),
        })
        .await
        .unwrap_or(false)
    }

    fn show(&mut self, tip: &Tip) {
        println!("onboarding: showing {}", tip.id());
        let view = match tip {
            Tip::Hotkey(label) => views::hotkey(label),
            Tip::Flyout => views::flyout(),
            Tip::Lyrics => views::lyrics(),
            Tip::Agent(agent) => {
                let p = profile(agent);
                let home = paths::home();
                let file = system::short(&system::agent_file(agent, &home), &home);
                views::agent(p.display, p.accent, p.logo.map(|l| self.ctx.image(l.to_vec())), p.glyph, &file)
            }
        };
        if tip.is_offer() {
            // A question only the user can answer: the island opens it once, folds it back
            // to compact (still asking) if it isn't engaged, and it goes after a while.
            // Never shown while paused or fullscreen (`quiet`), so it can't jump a pause.
            self.ctx.publish(TIP, Priority::Decision, view);
            self.hide_at = Some(Instant::now() + OFFER_SHOW);
        } else {
            self.ctx.publish(TIP, Priority::Brief, view);
            self.hide_at = Some(Instant::now() + TIP_SHOW);
        }
    }

    /// The tip on screen is done with; the next one may come after a gap.
    fn finish(&mut self) {
        self.ctx.retract(TIP);
        self.queue.finish();
        self.hide_at = None;
        if self.queue.is_waiting() {
            self.next_try = Some(Instant::now() + GAP);
        }
    }

    async fn timed_out(&mut self) {
        if let Some(tip) = self.queue.showing.clone() {
            self.record.answer(&tip, Answer::Declined);
            self.save().await;
        }
        self.finish();
    }

    async fn on_action(&mut self, key: &str, action: &str) {
        match (key, action) {
            // The hint: the first hover (or a click) is all it asked for.
            (HINT, "hovered" | "open") if self.phase == Phase::Hint => self.welcome(action).await,
            (TIP, views::OPEN_HISTORY) => {
                self.ctx.command("clipboard", "open");
                self.finish();
            }
            (TIP, views::ACCEPT | views::DECLINE) => {
                let Some(tip) = self.queue.showing.clone() else { return };
                let accepted = action == views::ACCEPT;
                self.record.answer(&tip, if accepted { Answer::Accepted } else { Answer::Declined });
                self.save().await;
                self.finish();
                if accepted {
                    self.accept(&tip).await;
                }
            }
            _ => {}
        }
    }

    /// Does what the offer offered, through the same commands the tray and Settings use.
    async fn accept(&mut self, tip: &Tip) {
        match tip {
            Tip::Flyout => self.ctx.command("osd", "flyout:on"),
            Tip::Lyrics => self.ctx.command("media", "lyrics:on"),
            Tip::Agent(agent) => {
                let agent: &'static str = agent;
                let result = tokio::task::spawn_blocking(move || system::connect(agent)).await.unwrap_or_else(|e| Err(e.to_string()));
                if let Err(e) = &result {
                    eprintln!("onboarding: connecting {agent} failed: {e}");
                }
                let p = profile(agent);
                let logo = p.logo.map(|l| self.ctx.image(l.to_vec()));
                self.ctx.publish(CONFIRM, Priority::Brief, views::connected(p.display, p.accent, logo, p.glyph, result));
                self.confirm_until = Some(Instant::now() + CONFIRM_SHOW);
            }
            Tip::Hotkey(_) => {}
        }
    }

    // ----- Coding agents -----

    /// Agents still to offer (not answered, not already connected).
    fn agents_due(&self) -> Vec<&'static str> {
        if !self.agents {
            return Vec::new();
        }
        [CLAUDE.name, CODEX.name, OPENCODE.name].into_iter().filter(|a| self.record.due(&Tip::Agent(a))).collect()
    }

    fn schedule_scan(&mut self, at: Instant) {
        self.next_scan = (!self.agents_due().is_empty()).then_some(at);
    }

    async fn scan_agents(&mut self) {
        let due = self.agents_due();
        let found = tokio::task::spawn_blocking(move || {
            let home = paths::home();
            let running = system::running_agents();
            due.into_iter().map(|a| (a, system::connected(a, &home), running.contains(&a))).collect::<Vec<_>>()
        })
        .await
        .unwrap_or_default();
        let mut changed = false;
        for (agent, connected, running) in found {
            if connected {
                // Already set up: nothing to offer, ever.
                self.record.answer(&Tip::Agent(agent), Answer::Accepted);
                changed = true;
            } else if running {
                self.enqueue(Tip::Agent(agent));
            }
        }
        if changed {
            self.save().await;
        }
        self.schedule_scan(Instant::now() + AGENT_SCAN);
    }
}
