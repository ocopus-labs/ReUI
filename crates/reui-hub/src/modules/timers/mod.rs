//! Timers (docs/PRD-timers-focus.md): countdowns, focus sessions and a stopwatch, started
//! from the launcher, the tray or a command, shown in the pill while they run and
//! impossible to miss when they end.
//!
//! The island runs the clocks it's sent ([`reui_protocol::Clock`]), so the hub never ticks:
//! it sleeps until the next thing is due (a timer or focus phase ending, the finish sound's
//! cap, an unanswered phase end giving up), and wakes early for a command, an action,
//! resume from sleep or a clock change. Truth is the module's monotonic clock (`Instant`,
//! which keeps counting through sleep); the wall clock only stamps what's sent and what's
//! saved (`store.rs`).
//!
//! Activities (module `timers`):
//! - `launcher` (Passive, summoned by the hotkey or the tray): a field that reads what you
//!   type, and the presets;
//! - `timers` (Ongoing): every running or paused countdown, the soonest in front, the rest
//!   a page away; `done` (Decision while it rings, then Alert, until stopped);
//! - `focus` (Ongoing) while a phase runs; `focus-next` (Decision while it rings, then
//!   Alert) when a phase ended and the next waits to be started;
//! - `stopwatch` (Ongoing).
//!
//! Do Not Disturb (opt-in, `dnd.rs`): turned on when a work phase starts if it was off,
//! put back when the work stops (a break, the end), never touched when the user had it on
//! already. A change that doesn't read back turns the feature off for the session. That
//! ReUI turned it on is saved, so a crash doesn't leave it on (E-T14).
//!
//! Commands: `start:<text>` (what the launcher's field takes, `parse.rs`), `launcher`,
//! `reload`. The launcher's hotkey (Win+Ctrl+T by default) lives on the shared
//! system-events window.
//!
//! Files: `parse.rs`, `model.rs` (countdowns), `focus.rs`, `stopwatch.rs`, `dnd.rs`,
//! `store.rs`, `sound.rs`, `view.rs`, `settings.rs`.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;

use reui_protocol::{unix_ms, Priority, Quiet, ACTION_COLLAPSED, ACTION_EXPANDED, ACTION_SEARCH, ACTION_SUBMIT, ACTION_UNDO};
use tokio::sync::mpsc;
use tokio::time::{sleep_until, Instant};

mod dnd;
mod focus;
mod model;
pub mod parse;
mod settings;
mod sound;
mod stopwatch;
mod store;
mod view;

/// Also the privacy indicator's "Since 10:42".
pub(crate) use self::view::time_of_day;
use self::focus::{Focus, Phase, Plan};
use self::model::{Cancelled, Finished, Timers, LATE_MS};
use self::parse::Parsed;
use self::settings::Settings;
use self::stopwatch::Stopwatch;
use super::hotkey::Hotkey;
use super::system_events::{self, Subscription, SystemEvent};
use super::{supervise, Module};
use crate::hub::{ModuleAction, ModuleContext, ModuleInput};

const LAUNCHER: &str = "launcher";
const RUNNING: &str = "timers";
const DONE: &str = "done";
const FOCUS: &str = "focus";
const FOCUS_NEXT: &str = "focus-next";
const STOPWATCH: &str = "stopwatch";
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
        ctx.declare_quiet(quiet);
        Box::pin(async move { supervise("timers", &mut ctx, |ctx| Box::pin(run(ctx))).await })
    }
}

/// A timer that finished quietly ("Finished at …", an alert) can be snoozed; there's no
/// setting to stop timers saying they're done. A ringing one is a decision and never quiet.
fn quiet(key: &str, _: Priority) -> Option<Quiet> {
    (key == DONE).then(Quiet::snooze_only)
}

/// The finish card: what ended, and whether it's still ringing.
#[derive(Default)]
struct Done {
    list: Vec<Finished>,
    /// It's ringing for this card.
    loud: bool,
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
    focus: Option<Focus>,
    /// It's ringing for the focus session's phase end.
    focus_loud: bool,
    stopwatch: Option<Stopwatch>,
    store: PathBuf,
    /// The timer the open card shows (`None`: the soonest).
    page: Option<u32>,
    done: Done,
    /// The finish sound plays until then (module clock), for whichever cards are loud.
    ring_until: Option<u64>,
    /// The last cancel, for Ctrl+Z.
    undo: Option<Cancelled>,
    events: Option<Subscription>,
    /// The hotkey as registered.
    hotkey: Option<Hotkey>,
    launcher: Option<Launcher>,
    /// The last length typed into the launcher: its placeholder, and what a bare Enter
    /// starts (T4).
    last_typed: Option<String>,
    dnd: Dnd,
}

/// Do Not Disturb as far as focus sessions go.
struct Dnd {
    /// ReUI turned it on and hasn't put it back.
    ours: bool,
    /// A change is on its way (it's checked half a second after).
    pending: bool,
    /// A change didn't stick; not tried again until the hub restarts (E-T11).
    failed: bool,
    /// Where a change's outcome comes back: (asked for on, it took).
    results: mpsc::UnboundedSender<(bool, bool)>,
}

async fn run(ctx: &mut ModuleContext) -> Result<(), String> {
    let (events_tx, mut events) = mpsc::unbounded_channel();
    // The first subscriber creates the window, which blocks for a moment.
    let subscription = tokio::task::spawn_blocking(move || system_events::subscribe(events_tx)).await.ok().flatten();
    let settings = Settings::load();
    let mut timers = Timers::default();
    let store = store::path();
    let loaded = store::load(&store, &mut timers, plan(&settings), 0, unix_ms());
    let (dnd_tx, mut dnd_results) = mpsc::unbounded_channel();
    let mut state = State {
        epoch: Instant::now(),
        settings,
        timers,
        focus: loaded.focus,
        focus_loud: false,
        stopwatch: loaded.stopwatch,
        store,
        page: None,
        done: Done::default(),
        ring_until: None,
        undo: None,
        events: subscription,
        hotkey: None,
        launcher: None,
        last_typed: None,
        dnd: Dnd { ours: loaded.dnd_ours, pending: false, failed: false, results: dnd_tx },
    };
    state.register_hotkey();
    // Also puts back a Do Not Disturb that a crashed run left on (E-T14).
    state.sync_dnd();
    if !loaded.finished.is_empty() {
        state.finish(loaded.finished);
    }
    state.publish(ctx);

    loop {
        let due = [state.timers.next_end(), state.focus.as_ref().and_then(Focus::next_due), state.ring_until];
        let deadline = due.into_iter().flatten().min().map(|ms| state.epoch + Duration::from_millis(ms));
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
            Some((on, took)) = dnd_results.recv() => state.dnd_changed(ctx, on, took),
            _ = sleep_until(deadline.unwrap_or_else(Instant::now)), if deadline.is_some() => {}
        }
        state.tick(ctx);
        state.sync_dnd();
    }
}

fn plan(settings: &Settings) -> Plan {
    let f = &settings.focus;
    Plan { work_ms: u64::from(f.work_min) * MINUTE, short_break_ms: u64::from(f.short_break_min) * MINUTE, long_break_ms: u64::from(f.long_break_min) * MINUTE, rounds: f.rounds }
}

impl State {
    fn now(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }

    /// Whatever is due: timers that reached zero, a focus phase ending (or an unanswered
    /// one giving up), a ring that has rung long enough.
    fn tick(&mut self, ctx: &ModuleContext) {
        let now = self.now();
        let mut changed = false;
        let finished = self.timers.take_finished(now);
        if !finished.is_empty() {
            self.finish(finished);
            changed = true;
        }
        let auto = self.settings.focus.auto_start;
        if let Some(f) = self.focus.as_mut() {
            if f.end_phase(now, auto) {
                println!("timers: a focus phase ended");
                if auto {
                    // Straight on: a chime, no card (unless something else is ringing).
                    if self.ring_until.is_none() {
                        sound::play(self.settings.sound.as_deref(), false);
                    }
                } else {
                    self.focus_loud = true;
                    self.ring();
                }
                changed = true;
            } else if f.gave_up(now) {
                println!("timers: focus ended after {} rounds (unanswered)", f.done_work);
                self.end_focus();
                changed = true;
            }
        }
        if self.ring_until.is_some_and(|until| until <= now) {
            sound::stop();
            self.ring_until = None;
            self.done.loud = false;
            self.focus_loud = false;
            changed = true;
        }
        if changed {
            self.save();
            self.publish(ctx);
        }
    }

    /// Adds to the finish card. Anything that ended just now rings (once, however many end
    /// together, E-T5); finishes noticed late (sleep, restart) are quiet (E-T1, E-T2).
    fn finish(&mut self, finished: Vec<Finished>) {
        let loud = finished.iter().any(|f| f.late_ms < LATE_MS);
        let latest = finished.iter().map(|f| f.late_ms).min().unwrap_or(0);
        for f in &finished {
            println!("timers: a {} timer finished{}", view::length_text(f.length_ms), if f.late_ms >= LATE_MS { " (late)" } else { "" });
        }
        let was_empty = self.done.list.is_empty();
        self.done.list.extend(finished);
        if loud {
            self.done.late_at = None;
            self.done.loud = true;
            self.ring();
        } else if was_empty {
            self.done.late_at = Some(unix_ms().saturating_sub(latest));
        }
    }

    /// Starts the finish sound, unless it's already playing (one sound for everything).
    fn ring(&mut self) {
        if self.ring_until.is_some() {
            return;
        }
        sound::play(self.settings.sound.as_deref(), true);
        let ring = if self.settings.ring_for_s == 0 { u64::MAX / 4 } else { u64::from(self.settings.ring_for_s) * 1000 };
        self.ring_until = Some(self.now() + ring);
    }

    /// Stops the sound once no card is ringing any more.
    fn hush(&mut self) {
        if !self.done.loud && !self.focus_loud && self.ring_until.take().is_some() {
            sound::stop();
        }
    }

    fn act(&mut self, ctx: &ModuleContext, ModuleAction { key, action }: ModuleAction) {
        let now = self.now();
        match (key.as_str(), action.as_str()) {
            ("", "reload") => {
                self.settings = Settings::load();
                self.register_hotkey();
                self.sync_dnd();
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
                self.timers.start(label, MINUTE, now);
            }
            (RUNNING, ACTION_EXPANDED) => self.page = None,
            (RUNNING, view::ACTION_NEXT | view::ACTION_PREV) => {
                let ids: Vec<u32> = self.timers.ordered(now).iter().map(|t| t.id).collect();
                let at = self.page.and_then(|p| ids.iter().position(|id| *id == p)).unwrap_or(0);
                let step = if action == view::ACTION_NEXT { 1 } else { ids.len() - 1 };
                self.page = ids.get((at + step) % ids.len().max(1)).copied();
                return self.publish(ctx);
            }
            (RUNNING, ACTION_UNDO) => {
                let Some(c) = self.undo.take() else { return };
                self.page = self.timers.add(c.label, c.length_ms, c.left_ms, c.running.then_some(now));
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
            }
            (FOCUS | FOCUS_NEXT, a) => {
                let Some(f) = self.focus.as_mut() else { return };
                match a {
                    view::ACTION_TOGGLE => f.toggle(now),
                    view::ACTION_NEXT_PHASE => f.start_next(now),
                    view::ACTION_SKIP => f.skip_break(now),
                    view::ACTION_END => self.end_focus(),
                    _ => return,
                }
                // Answered: the phase end stops ringing.
                if key == FOCUS_NEXT {
                    self.focus_loud = false;
                    self.hush();
                }
            }
            (STOPWATCH, a) => {
                let Some(s) = self.stopwatch.as_mut() else { return };
                match a {
                    view::ACTION_TOGGLE => s.toggle(now),
                    view::ACTION_LAP => _ = s.lap(now),
                    view::ACTION_RESET => self.stopwatch = None,
                    _ => return,
                }
            }
            _ => return,
        }
        self.save();
        self.sync_dnd();
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
                if action == ACTION_SUBMIT && matches!(parse::parse(&text), Parsed::Timer { .. }) {
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
        let now = self.now();
        match parse::parse(text) {
            Parsed::Timer { ms, label } => {
                let id = self.timers.start(label, ms, now).ok_or("10 timers is the limit")?;
                println!("timers: started a {} timer", view::length_text(ms));
                self.page = Some(id);
            }
            Parsed::Focus => {
                if self.focus.is_some() {
                    return Err("A focus session is already on");
                }
                println!("timers: focus session started");
                self.focus = Some(Focus::start(plan(&self.settings), now));
            }
            // There's one stopwatch; asking again just shows it.
            Parsed::Stopwatch => _ = self.stopwatch.get_or_insert_with(|| Stopwatch::start(now)),
            Parsed::TooLong => return Err("Up to 24 hours"),
            Parsed::Empty | Parsed::Unknown => return Err("Type a length, like 10m"),
        }
        self.save();
        Ok(())
    }

    /// Asks for the Do Not Disturb a focus session wants now: on while working (unless the
    /// user has it on already, or it failed before), back off when ReUI turned it on and the
    /// work stopped. A user who turns it off by hand mid-phase keeps it off (E-T12).
    fn sync_dnd(&mut self) {
        if self.dnd.pending {
            return;
        }
        let working = self.focus.as_ref().is_some_and(|f| f.phase == Phase::Work && !f.waiting());
        let wanted = working && self.settings.focus.dnd;
        if wanted && !self.dnd.ours && !self.dnd.failed && dnd::is_on() == Some(false) {
            self.request_dnd(true);
        } else if !wanted && self.dnd.ours {
            self.request_dnd(false);
        }
    }

    fn request_dnd(&mut self, on: bool) {
        self.dnd.pending = true;
        let results = self.dnd.results.clone();
        tokio::spawn(async move {
            let asked = tokio::task::spawn_blocking(move || dnd::set(on)).await.unwrap_or_else(|_| Err("panicked".into()));
            if let Err(e) = &asked {
                eprintln!("timers: can't change Do Not Disturb: {e}");
            }
            // The shell takes a moment to apply it.
            tokio::time::sleep(Duration::from_millis(500)).await;
            let took = asked.is_ok() && dnd::is_on() == Some(on);
            _ = results.send((on, took));
        });
    }

    fn dnd_changed(&mut self, ctx: &ModuleContext, on: bool, took: bool) {
        self.dnd.pending = false;
        println!("timers: Do Not Disturb {} {}", if on { "on" } else { "off" }, if took { "(checked)" } else { "didn't take" });
        if on && !took {
            self.dnd.failed = true;
        }
        // Off is ours no more even if it didn't read back: the user may have changed it.
        self.dnd.ours = on && took;
        self.save();
        self.publish(ctx);
    }

    /// The focus card's Do Not Disturb line, while working.
    fn dnd_line(&self) -> Option<&'static str> {
        if !self.settings.focus.dnd {
            return None;
        }
        if self.dnd.failed {
            return Some("Couldn't turn on Do Not Disturb");
        }
        (dnd::is_on() == Some(true)).then_some("Do Not Disturb on")
    }

    fn end_focus(&mut self) {
        self.focus = None;
        self.focus_loud = false;
        self.hush();
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
        self.done = Done::default();
        self.hush();
        ctx.retract(DONE);
    }

    fn save(&self) {
        let state = store::State { timers: &self.timers, focus: self.focus.as_ref(), stopwatch: self.stopwatch.as_ref(), dnd_ours: self.dnd.ours };
        store::save(&self.store, &state, self.now(), unix_ms());
    }

    fn publish(&self, ctx: &ModuleContext) {
        let (now, now_unix) = (self.now(), unix_ms());
        match view::running(&self.timers, self.page, now, now_unix) {
            Some(v) => ctx.publish(RUNNING, Priority::Ongoing, v),
            None => ctx.retract(RUNNING),
        }
        // A decision while it rings: it opens itself and comes through a paused ReUI (E-T3).
        let loud = |loud: bool| if loud { Priority::Decision } else { Priority::Alert };
        if self.done.list.is_empty() {
            ctx.retract(DONE);
        } else {
            ctx.publish(DONE, loud(self.done.loud), view::finished(&self.done.list, self.done.late_at));
        }
        match &self.focus {
            Some(f) if f.waiting() => {
                ctx.retract(FOCUS);
                ctx.publish(FOCUS_NEXT, loud(self.focus_loud), view::phase_end(f));
            }
            Some(f) => {
                ctx.retract(FOCUS_NEXT);
                let dnd = if f.phase == Phase::Work { self.dnd_line() } else { None };
                ctx.publish(FOCUS, Priority::Ongoing, view::focus(f, now, now_unix, dnd));
            }
            None => {
                ctx.retract(FOCUS);
                ctx.retract(FOCUS_NEXT);
            }
        }
        match &self.stopwatch {
            Some(s) => ctx.publish(STOPWATCH, Priority::Ongoing, view::stopwatch(s, now, now_unix)),
            None => ctx.retract(STOPWATCH),
        }
    }
}
