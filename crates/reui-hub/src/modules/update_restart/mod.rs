//! Windows Update restart (docs/PRD-update-restart.md): when updates are waiting for a
//! restart, the island says so once, with Schedule (Windows' own page) and Restart now
//! (asks once, then restarts installing them). Settings → General can turn it off
//! (`update-restart.json`, `enabled`).
//!
//! Nothing runs at rest: `watch.rs` sleeps on Windows Update's registry keys; the module
//! sleeps until a reading, an action, or the one deadline `plan.rs` sets (the note's end,
//! the confirm's).
//!
//! Module `update-restart`. Card actions: `schedule`, `restart`, expanded / collapsed.
//! Command: `reload` (Settings).

mod plan;
mod restart;
mod settings;
mod view;
mod watch;

use std::future::Future;
use std::pin::Pin;

use reui_protocol::{Priority, Quiet, ACTION_COLLAPSED, ACTION_EXPANDED};
use tokio::time::{sleep_until, Duration, Instant};

use self::plan::{Plan, State, Step};
use self::settings::Settings;
use super::Module;
use crate::hub::{ModuleAction, ModuleContext, ModuleInput};

const KEY: &str = "restart";

pub struct UpdateRestart;

impl Module for UpdateRestart {
    fn name(&self) -> &'static str {
        "update-restart"
    }

    fn run(self: Box<Self>, mut ctx: ModuleContext) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        ctx.declare_quiet(quiet);
        Box::pin(run(ctx))
    }
}

/// Right-click → off turns `enabled` off in its file.
fn quiet(_key: &str, _priority: Priority) -> Option<Quiet> {
    Some(Quiet::setting(settings::FILE, "enabled"))
}

async fn run(mut ctx: ModuleContext) {
    let mut settings = Settings::load();
    let (tx, mut readings) = tokio::sync::mpsc::unbounded_channel();
    // Kept for the module's life; dropped (stopping the thread) off the hub thread below.
    let Some(watcher) = tokio::task::spawn_blocking(move || watch::watch(tx)).await.ok().flatten() else { return };
    let start = Instant::now();
    let now = || start.elapsed().as_millis() as u64;
    let mut plan = Plan::default();
    let mut state = State::default();
    loop {
        let wake = plan.next().map(|ms| start + Duration::from_millis(ms));
        let step = tokio::select! {
            reading = readings.recv() => match reading {
                Some(reading) => {
                    state = reading;
                    if !state.needed() || settings.enabled { plan.saw(&state, now()) } else { Step::Nothing }
                }
                None => break,
            },
            input = ctx.next() => match input {
                Some(ModuleInput::Action(ModuleAction { action, .. })) => match action.as_str() {
                    "reload" => {
                        settings = Settings::load();
                        if settings.enabled { Step::Nothing } else { plan.scheduled() }
                    }
                    ACTION_EXPANDED => {
                        plan.opened();
                        Step::Nothing
                    }
                    ACTION_COLLAPSED => plan.closed(),
                    view::SCHEDULE => {
                        _ = tokio::task::spawn_blocking(restart::open_settings).await;
                        plan.scheduled()
                    }
                    view::RESTART => {
                        if plan.restart_pressed(now()) {
                            eprintln!("update-restart: restarting to install updates");
                            _ = tokio::task::spawn_blocking(restart::restart).await;
                        }
                        Step::Show
                    }
                    _ => Step::Nothing,
                },
                Some(_) => Step::Nothing,
                None => break,
            },
            _ = sleep_until(wake.unwrap_or_else(Instant::now)), if wake.is_some() => plan.tick(now()),
        };
        match step {
            Step::Show if plan.showing => ctx.publish(KEY, Priority::Brief, view::view(&state, plan.confirming())),
            Step::Retract => ctx.retract(KEY),
            _ => {}
        }
    }
    _ = tokio::task::spawn_blocking(move || drop(watcher)).await;
}
