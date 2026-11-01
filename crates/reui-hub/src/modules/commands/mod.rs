//! Long terminal commands (docs/PRD-terminal-commands.md): a shell's hook (installed from
//! Settings → Coding agents; `reui-bridge install powershell|bash`) runs `reui-bridge
//! finished` after a command that took 10 s or more, and the card says it's done or that
//! it failed, with its exit code, how long it took and Open terminal.
//!
//! Only when you aren't looking: a report from the terminal that's in front shows nothing.
//! One card per terminal (a newer command there replaces it); it goes when you open the
//! terminal from it, or after a while (`finished::DONE_STAYS`, `FAILED_STAYS`). Nothing
//! runs between reports but the one timer for the next card to go.
//!
//! Module `commands`. Command: `finished:<json>` (from the bridge). Card action:
//! `terminal`.

mod finished;

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use tokio::time::{sleep_until, Instant};

use self::finished::{Finished, ACTION_TERMINAL};
use super::agent::terminal;
use super::{supervise, Module};
use crate::hub::{ModuleAction, ModuleContext, ModuleInput};

pub struct TerminalCommands;

impl Module for TerminalCommands {
    fn name(&self) -> &'static str {
        "commands"
    }

    fn run(self: Box<Self>, mut ctx: ModuleContext) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        // A command line can hold a host, a path or a token passed as an argument
        // (docs/PRD-share-privacy.md).
        ctx.declare_private();
        Box::pin(async move { supervise("commands", &mut ctx, |ctx| Box::pin(run(ctx))).await })
    }
}

async fn run(ctx: &mut ModuleContext) -> Result<(), String> {
    // Shown cards by key, and when each goes.
    let mut shown: HashMap<String, (Finished, Instant)> = HashMap::new();
    loop {
        let next = shown.values().map(|(_, at)| *at).min();
        tokio::select! {
            input = ctx.next() => match input {
                Some(ModuleInput::Action(ModuleAction { key, action })) => {
                    if key.is_empty() {
                        if let Some(json) = action.strip_prefix("finished:") {
                            arrive(ctx, &mut shown, json).await;
                        }
                    } else if action == ACTION_TERMINAL {
                        if let Some((f, _)) = shown.remove(&key) {
                            ctx.retract(&key);
                            if let Some(origin) = f.origin {
                                _ = tokio::task::spawn_blocking(move || terminal::open(&origin)).await;
                            }
                        }
                    }
                }
                Some(_) => {}
                None => return Ok(()),
            },
            _ = sleep_until(next.unwrap_or_else(Instant::now)), if next.is_some() => {
                let now = Instant::now();
                let gone: Vec<String> = shown.iter().filter(|(_, (_, at))| *at <= now).map(|(k, _)| k.clone()).collect();
                for key in gone {
                    shown.remove(&key);
                    ctx.retract(&key);
                }
            }
        }
    }
}

/// A report from a shell: shown unless its terminal is the window in front.
async fn arrive(ctx: &ModuleContext, shown: &mut HashMap<String, (Finished, Instant)>, json: &str) {
    let Some(f) = Finished::parse(json) else { return };
    let key = f.key();
    let origin = f.origin.clone();
    let looking = match origin {
        Some(origin) => tokio::task::spawn_blocking(move || terminal::in_front(&origin)).await.unwrap_or(false),
        None => false,
    };
    eprintln!("commands: {key} finished (exit {}, {} s){}", f.exit, f.seconds, if looking { ", its terminal is in front" } else { "" });
    if looking {
        // You saw it finish; an older card from that terminal is stale too.
        if shown.remove(&key).is_some() {
            ctx.retract(&key);
        }
        return;
    }
    ctx.publish(&key, f.priority(), finished::view(&f));
    let until = Instant::now() + f.stays();
    shown.insert(key, (f, until));
}
