//! Live activities from scripts (docs/PRD-reminders-activities.md §5.4): a build, a backup,
//! `winget upgrade` says how far it is with `reui-bridge activity <id> --progress 0.42`,
//! and the island shows it like any other activity. Each is an Ongoing activity keyed by
//! its id while it runs, then a Brief when it's done or failed.
//!
//! Commands (from `reui-bridge activity`, over the per-user pipe): `update:<json>`
//! (`request.rs` reads and checks it), `reload`. Nothing is polled: the module sleeps until
//! the next finished one should go, a silent one should time out, or the pace allows the
//! next publish (`board.rs`). Activities aren't saved; after a hub restart the script's
//! next update brings its activity back.
//!
//! Files: `request.rs`, `board.rs`, `view.rs`, `settings.rs`.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use reui_protocol::{unix_ms, Priority};
use tokio::time::{sleep_until, Instant};

mod board;
mod request;
mod settings;
mod view;

use self::board::{Board, Limits, Pace};
use self::request::{Phase, Request};
use self::settings::Settings;
use super::{supervise, Module};
use crate::hub::{ModuleAction, ModuleContext, ModuleInput};

pub struct ScriptActivities;

impl Module for ScriptActivities {
    fn name(&self) -> &'static str {
        "activities"
    }

    fn run(self: Box<Self>, mut ctx: ModuleContext) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async move { supervise("activities", &mut ctx, |ctx| Box::pin(run(ctx))).await })
    }
}

struct State {
    epoch: Instant,
    settings: Settings,
    board: Board,
    pace: Pace,
    /// Ids published last, so ones that went are retracted.
    shown: Vec<String>,
}

async fn run(ctx: &mut ModuleContext) -> Result<(), String> {
    let mut state = State { epoch: Instant::now(), settings: Settings::load(), board: Board::default(), pace: Pace::default(), shown: Vec::new() };
    loop {
        let limits = state.limits();
        let due = [state.board.next_due(limits), state.pace.due()].into_iter().flatten().min();
        let deadline = due.map(|ms| state.epoch + Duration::from_millis(ms));
        tokio::select! {
            input = ctx.next() => match input {
                Some(ModuleInput::Action(action)) => state.act(ctx, action),
                Some(_) => {}
                None => return Ok(()),
            },
            _ = sleep_until(deadline.unwrap_or_else(Instant::now)), if deadline.is_some() => {
                let now = state.now();
                let gone = state.board.expire(limits, now);
                for id in &gone {
                    println!("activities: {id} ended");
                }
                if !gone.is_empty() || state.pace.due().is_some_and(|at| at <= now) {
                    state.publish(ctx, now);
                }
            }
        }
    }
}

impl State {
    fn now(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }

    fn limits(&self) -> Limits {
        Limits { per_source: self.settings.per_source as usize, stale_ms: u64::from(self.settings.stale_min) * 60_000 }
    }

    fn act(&mut self, ctx: &ModuleContext, ModuleAction { key, action }: ModuleAction) {
        // The cards have no buttons; only commands come in.
        if !key.is_empty() {
            return;
        }
        let now = self.now();
        if action == "reload" {
            self.settings = Settings::load();
            if !self.settings.enabled {
                self.board.clear();
            }
            return self.publish(ctx, now);
        }
        let Some(json) = action.strip_prefix("update:") else { return };
        if !self.settings.enabled {
            return;
        }
        match request::read(json) {
            Err(why) => eprintln!("activities: refused an update: {why}"),
            Ok(Request::End { id }) => {
                if self.board.end(&id) {
                    self.publish(ctx, now);
                }
            }
            Ok(Request::Update(u)) => {
                if self.settings.blocked.contains(&u.source) {
                    return;
                }
                let phase = u.phase;
                let id = u.id.clone();
                let applied = self.board.apply(u, self.limits(), now);
                for gone in &applied.evicted {
                    println!("activities: {gone} made way for {id}");
                }
                // A finish shows at once; progress keeps to the pace.
                if applied.finished || phase != Phase::Running || !self.shown.contains(&id) || !applied.evicted.is_empty() {
                    self.publish(ctx, now);
                } else if self.pace.ask(now) {
                    self.publish_now(ctx, now);
                }
            }
        }
    }

    fn publish(&mut self, ctx: &ModuleContext, now: u64) {
        self.pace.published(now);
        self.publish_now(ctx, now);
    }

    fn publish_now(&mut self, ctx: &ModuleContext, now: u64) {
        let now_unix = unix_ms();
        let mut shown = Vec::new();
        for e in self.board.all() {
            // Never above Alert: a script can't ask anything or open the card by itself.
            let priority = if e.phase == Phase::Running { Priority::Ongoing } else { Priority::Brief };
            ctx.publish(&e.id, priority, view::view(e, now, now_unix));
            shown.push(e.id.clone());
        }
        for id in self.shown.iter().filter(|id| !shown.contains(id)) {
            ctx.retract(id);
        }
        self.shown = shown;
    }
}
