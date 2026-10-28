//! Timers (docs/PRD-timers-focus.md): countdowns started from the launcher, the tray or a
//! command, shown in the pill while they run and impossible to miss when they end.
//!
//! The island runs the clocks it's sent ([`reui_protocol::Clock`]), so the hub never ticks:
//! it sleeps until the soonest timer ends (or the finish sound's cap), and wakes early for a
//! command, an action, resume from sleep or a clock change. Truth is the module's
//! monotonic clock (`Instant`, which keeps counting through sleep); the wall clock only
//! stamps what's sent and what's saved (`store.rs`).
//!
//! Activities (module `timers`): `launcher` (Passive, summoned by the hotkey or the tray:
//! a field that reads what you type, and the presets), `timers` (Ongoing: every running or
//! paused timer, the soonest in front, the rest a page away) and `done` (Decision while it
//! rings, then Alert, until stopped).
//!
//! Commands: `start:<text>` (what the launcher's field takes, `parse.rs`), `launcher`,
//! `reload`. The launcher's hotkey (Win+Ctrl+T by default) lives on the shared
//! system-events window.
//!
//! Files: `parse.rs`, `model.rs`, `store.rs`, `sound.rs`, `view.rs`, `settings.rs`.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;

use reui_protocol::{unix_ms, Priority, ACTION_COLLAPSED, ACTION_EXPANDED, ACTION_SEARCH, ACTION_SUBMIT, ACTION_UNDO};
use tokio::sync::mpsc;
use tokio::time::{sleep_until, Instant};

mod model;
pub mod parse;
mod settings;
mod sound;
mod store;
mod view;

use self::model::{Cancelled, Finished, Timers, LATE_MS};
use self::parse::Parsed;
use self::settings::Settings;
use super::hotkey::Hotkey;
use super::system_events::{self, Subscription, SystemEvent};
use super::{supervise, Module};
use crate::hub::{ModuleAction, ModuleContext, ModuleInput};

const LAUNCHER: &str = "launcher";
const RUNNING: &str = "timers";
const DONE: &str = "done";
const MINUTE: u64 = 60_000;
/// The launcher hotkey's id on the shared system-events window.
const HOTKEY_ID: i32 = 0x7101;
const DEFAULT_HOTKEY: &str = "Win+Ctrl+T";

pub struct TimersModule;

impl Module for TimersModule {
    fn name(&self) -> &'static str {
        "timers"
    }

    fn run(self: Box<Self>, mut ctx: ModuleContext) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async move { supervise("timers", &mut ctx, |ctx| Box::pin(run(ctx))).await })
    }
}

/// The finish card: what ended, and whether it's still ringing.
#[derive(Default)]
struct Done {
    list: Vec<Finished>,
    /// Ringing until then (module clock); `None` once quiet.
    ringing_until: Option<u64>,
    /// They all ended a while ago (sleep, ReUI closed): when, on the wall clock.
    late_at: Option<u64>,
}

/// The open launcher: what's typed, and why the last try didn't start anything.
#[derive(Default)]
struct Launcher {
    query: String,
    refused: Option<&'static str>,
}

struct State {
    epoch: Instant,
    settings: Settings,
    timers: Timers,
    store: PathBuf,
    /// The timer the open card shows (`None`: the soonest).
    page: Option<u32>,
    done: Done,
    /// The last cancel, for Ctrl+Z.
    undo: Option<Cancelled>,
    events: Option<Subscription>,
    /// The hotkey as registered.
    hotkey: Option<Hotkey>,
    launcher: Option<Launcher>,
    /// The last length typed into the launcher: its placeholder, and what a bare Enter
    /// starts (T4).
    last_typed: Option<String>,
}

async fn run(ctx: &mut ModuleContext) -> Result<(), String> {
    let (events_tx, mut events) = mpsc::unbounded_channel();
    // The first subscriber creates the window, which blocks for a moment.
    let subscription = tokio::task::spawn_blocking(move || system_events::subscribe(events_tx)).await.ok().flatten();
    let mut state = State {
        epoch: Instant::now(),
        settings: Settings::load(),
        timers: Timers::default(),
        store: store::path(),
        page: None,
        done: Done::default(),
        undo: None,
        events: subscription,
        hotkey: None,
        launcher: None,
        last_typed: None,
    };
    state.register_hotkey();
    let missed = store::load(&state.store, &mut state.timers, 0, unix_ms());
    if !missed.is_empty() {
        state.finish(ctx, missed);
    }
    state.publish(ctx);

    loop {
        let wake = [state.timers.next_end(), state.done.ringing_until].into_iter().flatten().min();
        let deadline = wake.map(|ms| state.epoch + Duration::from_millis(ms));
        tokio::select! {
            input = ctx.next() => match input {
                Some(ModuleInput::Action(action)) => state.act(ctx, action),
                Some(_) => {}
                None => return Ok(()),
            },
            Some(event) = events.recv() => match event {
                // Waits don't count sleep, so look again now; a clock change re-stamps
                // what the island shows (and the "Ends 3:42 PM" line).
                SystemEvent::Resumed | SystemEvent::TimeChanged => state.publish(ctx),
                SystemEvent::Hotkey { id: HOTKEY_ID } => state.open_launcher(ctx),
                SystemEvent::HotkeyTaken { id: HOTKEY_ID } => eprintln!("timers: the launcher hotkey is taken by another app"),
                _ => continue,
            },
            _ = sleep_until(deadline.unwrap_or_else(Instant::now)), if deadline.is_some() => {}
        }
        state.tick(ctx);
    }
}

impl State {
    fn now(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }

    /// Whatever is due: timers that reached zero, a ring that has rung long enough.
    fn tick(&mut self, ctx: &ModuleContext) {
        let now = self.now();
        let finished = self.timers.take_finished(now);
        if !finished.is_empty() {
            self.finish(ctx, finished);
            self.save();
            self.publish(ctx);
        }
        if self.done.ringing_until.is_some_and(|until| until <= now) {
            sound::stop();
            self.done.ringing_until = None;
            self.publish_done(ctx);
        }
    }

    /// Adds to the finish card. Anything that ended just now rings (once, however many end
    /// together, E-T5); finishes noticed late (sleep, restart) are quiet (E-T1, E-T2).
    fn finish(&mut self, ctx: &ModuleContext, finished: Vec<Finished>) {
        let loud = finished.iter().any(|f| f.late_ms < LATE_MS);
        let latest = finished.iter().map(|f| f.late_ms).min().unwrap_or(0);
        for f in &finished {
            println!("timers: a {} timer finished{}", view::length_text(f.length_ms), if f.late_ms >= LATE_MS { " (late)" } else { "" });
        }
        let was_empty = self.done.list.is_empty();
        self.done.list.extend(finished);
        if loud {
            self.done.late_at = None;
            if self.done.ringing_until.is_none() {
                sound::play(self.settings.sound.as_deref(), true);
                let ring = if self.settings.ring_for_s == 0 { u64::MAX / 2 } else { u64::from(self.settings.ring_for_s) * 1000 };
                self.done.ringing_until = Some(self.now() + ring);
            }
        } else if was_empty {
            self.done.late_at = Some(unix_ms().saturating_sub(latest));
        }
        self.publish_done(ctx);
    }

    fn act(&mut self, ctx: &ModuleContext, ModuleAction { key, action }: ModuleAction) {
        let now = self.now();
        match (key.as_str(), action.as_str()) {
            ("", "reload") => {
                self.settings = Settings::load();
                self.register_hotkey();
            }
            ("", "launcher") => return self.open_launcher(ctx),
            ("", command) if command.starts_with("start:") => {
                if let Err(why) = self.start(&command["start:".len()..]) {
                    println!("timers: didn't start: {why}");
                }
            }
            (LAUNCHER, _) => return self.act_launcher(ctx, &action),
            (DONE, view::ACTION_STOP) => self.stop_done(ctx),
            (DONE, view::ACTION_MORE) => {
                let label = self.done.list.last().and_then(|d| d.label.clone());
                self.stop_done(ctx);
                if self.timers.start(label, MINUTE, now).is_some() {
                    self.save();
                }
            }
            (RUNNING, ACTION_EXPANDED) => self.page = None,
            (RUNNING, view::ACTION_NEXT | view::ACTION_PREV) => {
                let ids: Vec<u32> = self.timers.ordered(now).iter().map(|t| t.id).collect();
                let at = self.page.and_then(|p| ids.iter().position(|id| *id == p)).unwrap_or(0);
                let step = if action == view::ACTION_NEXT { 1 } else { ids.len() - 1 };
                self.page = ids.get((at + step) % ids.len().max(1)).copied();
            }
            (RUNNING, ACTION_UNDO) => {
                if let Some(c) = self.undo.take() {
                    let id = self.timers.add(c.label, c.length_ms, c.left_ms, c.running.then_some(now));
                    self.page = id;
                    self.save();
                }
            }
            (RUNNING, a) => {
                let Some((verb, id)) = a.split_once(':') else { return };
                let Ok(id) = id.parse::<u32>() else { return };
                match verb {
                    "toggle" => self.timers.toggle(id, now),
                    "cancel" => {
                        self.undo = self.timers.cancel(id, now);
                        if self.page == Some(id) {
                            self.page = None;
                        }
                    }
                    _ => return,
                }
                self.save();
            }
            _ => return,
        }
        self.publish(ctx);
    }

    /// The launcher's field and presets. Enter on an empty field starts the last length
    /// typed (the placeholder); anything refused says why in place of the detail line.
    fn act_launcher(&mut self, ctx: &ModuleContext, action: &str) {
        let Some(launcher) = self.launcher.as_mut() else { return };
        let text = if let Some(query) = action.strip_prefix(ACTION_SEARCH) {
            launcher.query = query.to_string();
            launcher.refused = None;
            return self.publish_launcher(ctx);
        } else if action == ACTION_SUBMIT {
            let typed = launcher.query.trim().to_string();
            if typed.is_empty() { self.placeholder() } else { typed }
        } else if let Some(minutes) = action.strip_prefix("preset:").and_then(|m| m.parse::<u32>().ok()) {
            format!("{minutes}m")
        } else if action == ACTION_COLLAPSED {
            // Closed without starting anything.
            self.launcher = None;
            return ctx.retract(LAUNCHER);
        } else {
            return;
        };
        match self.start(&text) {
            Ok(()) => {
                if action == ACTION_SUBMIT {
                    self.last_typed = Some(text);
                }
                self.launcher = None;
                ctx.retract(LAUNCHER);
                self.publish(ctx);
            }
            Err(why) => {
                if let Some(l) = self.launcher.as_mut() {
                    l.refused = Some(why);
                }
                self.publish_launcher(ctx);
            }
        }
    }

    /// Starts what `text` asks for, or says why it can't.
    fn start(&mut self, text: &str) -> Result<(), &'static str> {
        match parse::parse(text) {
            Parsed::Timer { ms, label } => {
                let id = self.timers.start(label, ms, self.now()).ok_or("10 timers is the limit")?;
                println!("timers: started a {} timer", view::length_text(ms));
                self.page = Some(id);
                self.save();
                Ok(())
            }
            Parsed::TooLong => Err("Up to 24 hours"),
            // Focus sessions and the stopwatch come in M3.
            Parsed::Focus | Parsed::Stopwatch => Err("Not yet"),
            Parsed::Empty | Parsed::Unknown => Err("Type a length, like 10m"),
        }
    }

    /// Opens the launcher (the hotkey, the tray's "New timer…"). Pressing the hotkey again
    /// while it's open closes it (the island does that).
    fn open_launcher(&mut self, ctx: &ModuleContext) {
        self.launcher.get_or_insert_with(Launcher::default);
        self.publish_launcher(ctx);
        ctx.focus(LAUNCHER);
    }

    /// What an empty field offers: the last length typed, else the longest preset.
    fn placeholder(&self) -> String {
        self.last_typed.clone().unwrap_or_else(|| format!("{}m", self.settings.presets.iter().max().copied().unwrap_or(25)))
    }

    fn publish_launcher(&self, ctx: &ModuleContext) {
        let Some(l) = &self.launcher else { return };
        // An empty field reads as what Enter would start.
        let parsed = match parse::parse(&l.query) {
            Parsed::Empty => parse::parse(&self.placeholder()),
            parsed => parsed,
        };
        ctx.publish(LAUNCHER, Priority::Passive, view::launcher(&l.query, &parsed, &self.placeholder(), &self.settings.presets, l.refused));
    }

    /// (Re)registers the launcher hotkey from settings; one it can't read is the default.
    fn register_hotkey(&mut self) {
        let wanted = Hotkey::parse(&self.settings.shortcut).or_else(|| Hotkey::parse(DEFAULT_HOTKEY));
        if wanted == self.hotkey {
            return;
        }
        if let Some(events) = &self.events {
            events.hotkey(HOTKEY_ID, wanted.as_ref());
        }
        self.hotkey = wanted;
    }

    fn stop_done(&mut self, ctx: &ModuleContext) {
        if self.done.ringing_until.take().is_some() {
            sound::stop();
        }
        self.done = Done::default();
        ctx.retract(DONE);
    }

    fn save(&self) {
        store::save(&self.store, &self.timers, self.now(), unix_ms());
    }

    fn publish(&self, ctx: &ModuleContext) {
        match view::running(&self.timers, self.page, self.now(), unix_ms()) {
            Some(v) => ctx.publish(RUNNING, Priority::Ongoing, v),
            None => ctx.retract(RUNNING),
        }
        self.publish_done(ctx);
    }

    fn publish_done(&self, ctx: &ModuleContext) {
        if self.done.list.is_empty() {
            return;
        }
        // A decision while it rings: it opens itself and comes through a paused ReUI (E-T3).
        let priority = if self.done.ringing_until.is_some() { Priority::Decision } else { Priority::Alert };
        ctx.publish(DONE, priority, view::finished(&self.done.list, self.done.late_at));
    }
}
