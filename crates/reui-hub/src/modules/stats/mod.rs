//! System stats (docs/PRD-system-stats.md): CPU, GPU, memory, disk and network in a card
//! summoned by a hotkey or the tray, read only while the card is open.
//!
//! Stats have no OS events, so they're polled by nature; the rule that idle costs nothing
//! holds because there's nothing to poll while the card is closed. Opening it starts the
//! sampler's thread (`sampler.rs`, once a second, like Task Manager); the island's
//! `collapsed` stops it (`card.rs` decides, with deadlines for when the island never
//! opens it or never says it closed). The hub thread only publishes what arrives.
//!
//! Activity (module `stats`): `system` (Passive, summoned), there only while it's open.
//!
//! Commands: `open` (the tray, `reui-bridge stats`, Settings), `reload`. The hotkey
//! (Win+Alt+S by default) lives on the shared system-events window. The card's header
//! button sends `taskmgr` (open Task Manager; the island's click lets it come forward).
//!
//! Files: `card.rs` (open/closed), `sampler.rs` (the thread), `os.rs` (CPU, memory, network
//! reads, Task Manager), `pdh.rs` (GPU and disk counters), `maths.rs` (deltas, the GPU's
//! busiest engine, numbers as text), `history.rs` (the open card's last minute, and when a
//! number has stayed high), `view.rs`, `settings.rs`.

use std::future::Future;
use std::pin::Pin;

use reui_protocol::{Priority, ACTION_COLLAPSED, ACTION_EXPANDED};
use tokio::sync::mpsc;
use tokio::time::{sleep_until, Instant};

mod card;
mod history;
/// Also formats sizes and speeds for the downloads module.
pub(crate) mod maths;
mod os;
mod pdh;
mod sampler;
mod settings;
mod view;

use self::card::{Card, Do};
use self::history::History;
use self::sampler::{Reading, Sampler};
use self::settings::Settings;
use super::hotkey::Hotkey;
use super::system_events::{self, Subscription, SystemEvent};
use super::{supervise, Module};
use crate::hub::{ModuleAction, ModuleContext, ModuleInput};

const KEY: &str = "system";
/// The card's hotkey id on the shared system-events window.
const HOTKEY_ID: i32 = 0x7201;

pub struct SystemStats;

impl Module for SystemStats {
    fn name(&self) -> &'static str {
        "stats"
    }

    fn run(self: Box<Self>, mut ctx: ModuleContext) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async move { supervise("stats", &mut ctx, |ctx| Box::pin(run(ctx))).await })
    }
}

struct State {
    settings: Settings,
    card: Card,
    /// What this opening has read so far; a new opening starts empty.
    history: History,
    /// Running only while the card is open; dropping it ends the thread.
    sampler: Option<Sampler>,
    readings: mpsc::UnboundedSender<Reading>,
    events: Option<Subscription>,
    hotkey: Option<Hotkey>,
}

async fn run(ctx: &mut ModuleContext) -> Result<(), String> {
    let (events_tx, mut events) = mpsc::unbounded_channel();
    // The first subscriber creates the window, which blocks for a moment.
    let subscription = tokio::task::spawn_blocking(move || system_events::subscribe(events_tx)).await.ok().flatten();
    let settings = tokio::task::spawn_blocking(Settings::load).await.unwrap_or_default();
    let (readings_tx, mut readings) = mpsc::unbounded_channel();
    let mut state = State { settings, card: Card::default(), history: History::default(), sampler: None, readings: readings_tx, events: subscription, hotkey: None };
    state.register_hotkey();

    loop {
        let deadline = state.card.deadline().map(Instant::from_std);
        let todo = tokio::select! {
            input = ctx.next() => match input {
                Some(ModuleInput::Action(action)) => state.act(action),
                Some(_) => Do::Nothing,
                None => return Ok(()),
            },
            Some(event) = events.recv() => match event {
                SystemEvent::Hotkey { id: HOTKEY_ID } => state.card.open(std::time::Instant::now()),
                SystemEvent::HotkeyTaken { id: HOTKEY_ID } => {
                    eprintln!("stats: the hotkey is taken by another app");
                    Do::Nothing
                }
                _ => continue,
            },
            Some(reading) = readings.recv() => {
                let todo = state.card.reading(reading.run, std::time::Instant::now());
                if matches!(todo, Do::Publish | Do::PublishAndOpen) {
                    let memory = reading.memory.filter(|(_, total)| *total > 0).map(|(used, total)| used as f32 / total as f32);
                    state.history.push(reading.cpu, memory, reading.gpu, reading.disk);
                    ctx.publish(KEY, Priority::Passive, view::card(&reading, state.settings.show, &state.history));
                }
                todo
            }
            _ = sleep_until(deadline.unwrap_or_else(Instant::now)), if deadline.is_some() => state.card.due(std::time::Instant::now()),
        };
        state.apply(ctx, todo);
    }
}

impl State {
    fn act(&mut self, ModuleAction { key, action }: ModuleAction) -> Do {
        let now = std::time::Instant::now();
        match (key.as_str(), action.as_str()) {
            ("", "open") => self.card.open(now),
            ("", "reload") => {
                self.settings = Settings::load();
                self.register_hotkey();
                // The next open reads what the card now shows.
                self.card.close()
            }
            (KEY, ACTION_EXPANDED) => {
                self.card.expanded(now);
                Do::Nothing
            }
            (KEY, ACTION_COLLAPSED) => self.card.collapsed(),
            (KEY, view::ACTION_TASK_MANAGER) => {
                tokio::task::spawn_blocking(os::open_task_manager);
                Do::Nothing
            }
            _ => Do::Nothing,
        }
    }

    fn apply(&mut self, ctx: &ModuleContext, todo: Do) {
        match todo {
            Do::Nothing | Do::Publish => {}
            Do::Start(run) => {
                self.history = History::default();
                self.sampler = Sampler::start(run, self.settings.show, self.readings.clone());
                if self.sampler.is_none() {
                    eprintln!("stats: couldn't start the sampler");
                    self.card.close();
                }
            }
            Do::PublishAndOpen | Do::Toggle => ctx.focus(KEY),
            Do::Close => {
                self.sampler = None;
                ctx.retract(KEY);
            }
        }
    }

    /// (Re)registers the hotkey from settings; one it can't read is the default.
    fn register_hotkey(&mut self) {
        let wanted = Hotkey::parse(&self.settings.shortcut).or_else(|| Hotkey::parse(settings::DEFAULT_SHORTCUT));
        if wanted == self.hotkey {
            return;
        }
        if let Some(events) = &self.events {
            events.hotkey(HOTKEY_ID, wanted.as_ref());
        }
        self.hotkey = wanted;
    }
}
