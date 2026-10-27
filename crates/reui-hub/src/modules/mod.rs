//! Feature modules. Each owns its adapters and state, publishes activities through its
//! [`ModuleContext`], and handles the actions surfaces send back. A disabled module is
//! never constructed, so its adapters never run.

pub mod agent;
mod artwork;
pub mod demo;
pub mod media;
mod usage;

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
