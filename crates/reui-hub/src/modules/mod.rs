//! Feature modules. Each owns its adapters and state, publishes activities through its
//! [`ModuleContext`], and handles the actions surfaces send back. A disabled module is
//! never constructed, so its adapters never run.

pub mod agent;
pub mod calendar;
mod audio;
pub mod clipboard;
pub mod demo;
pub mod destinations;
pub mod devices;
pub mod hide;
pub mod hotkey;
pub mod media;
pub mod onboarding;
pub mod osd;
pub mod privacy;
pub mod stats;
mod system_events;
pub mod timers;
pub mod updates;

use std::future::Future;
use std::pin::Pin;

use crate::hub::{Hub, ModuleContext};

pub trait Module: Send + 'static {
    fn name(&self) -> &'static str;
    fn run(self: Box<Self>, ctx: ModuleContext) -> Pin<Box<dyn Future<Output = ()> + Send>>;
}

pub fn start(hub: &Hub, modules: Vec<Box<dyn Module>>) {
    for module in modules {
        let ctx = hub.context(module.name());
        println!("module started: {}", module.name());
        tokio::spawn(module.run(ctx));
    }
}

/// A module body borrowing its context, so it can be run again after it fails.
pub type Body<'a, E> = Pin<Box<dyn Future<Output = Result<(), E>> + Send + 'a>>;

/// Runs a module body and starts it again (after 1 s, backing off to 30 s) when it returns
/// an error or panics, so one bad event from an OS API never ends a feature until the hub
/// restarts. `Ok` means the hub is shutting down.
pub async fn supervise<E: std::fmt::Display>(name: &str, ctx: &mut ModuleContext, mut body: impl for<'a> FnMut(&'a mut ModuleContext) -> Body<'a, E>) {
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::task::Poll;
    let mut wait = std::time::Duration::from_secs(1);
    loop {
        let mut run = body(ctx);
        let outcome = std::future::poll_fn(|cx| match catch_unwind(AssertUnwindSafe(|| run.as_mut().poll(cx))) {
            Ok(Poll::Ready(result)) => Poll::Ready(result.map_err(|e| e.to_string())),
            Ok(Poll::Pending) => Poll::Pending,
            Err(_) => Poll::Ready(Err("panicked".to_string())),
        })
        .await;
        drop(run);
        match outcome {
            Ok(()) => return,
            Err(e) => eprintln!("{name}: stopped ({e}); restarting in {}s", wait.as_secs()),
        }
        tokio::time::sleep(wait).await;
        wait = (wait * 2).min(std::time::Duration::from_secs(30));
    }
}
