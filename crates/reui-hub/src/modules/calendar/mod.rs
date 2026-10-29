//! Calendar (docs/PRD-calendar.md): the next meeting counted down in the pill, its start
//! brought forward with Join, and the day's card as a place in the view switcher.
//!
//! The module sleeps until the next thing is due (a countdown starting, a start, an alert
//! lapsing, an event ending, midnight, a feed's refresh) and wakes early for an action, a
//! feed arriving, resume, a clock or zone change, or the internet coming back. The island
//! runs the countdown ring itself, so nothing ticks in between. Calendar times are wall
//! times, so every deadline is worked out from `unix_ms()` each time it sleeps.
//!
//! Activities (module `calendar`):
//! - `next` (Ongoing) in the last `lead_min` minutes before a timed event;
//! - `now` (Alert) from its start until joined, seen (opened and closed) or 5 min in;
//! - `today` (Passive, summoned by `open`: the view switcher, the tray), the day's card.
//!
//! Sources: secret ICS links (`fetch.rs`, parsed by `ics/`), and Windows' own store
//! (`windows_store.rs`) when turned on. Links and cached feeds are DPAPI-encrypted
//! (`dpapi.rs`); logs never name a link or an event.
//!
//! Commands: `open`, `reload`, `refresh`. Files: `event.rs`, `ics/`, `links.rs`,
//! `schedule.rs`, `fetch.rs`, `network.rs`, `windows_store.rs`, `dpapi.rs`,
//! `settings.rs`, `status.rs`, `view.rs`.

mod dpapi;
mod event;
mod fetch;
mod ics;
mod links;
mod network;
mod schedule;
mod settings;
mod status;
mod view;
mod windows_store;

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use chrono::{Days, Local, NaiveTime};
use reqwest::Client;
use reui_protocol::{unix_ms, Priority, ACTION_COLLAPSED, ACTION_EXPANDED};
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio::time::{sleep_until, Instant};

use self::event::{Event, Source as _};
use self::fetch::{Cached, Fetched, Validators};
use self::ics::Feed;
use self::settings::Settings;
use self::status::{Refresh, Status};
use self::windows_store::Snapshot;
use super::system_events::{self, SystemEvent};
use super::{supervise, Module};
use crate::hub::{ModuleAction, ModuleContext, ModuleInput};

const NEXT: &str = "next";
const NOW: &str = "now";
const TODAY: &str = "today";
const WINDOWS: &str = "windows";
const DAY_MS: u64 = 86_400_000;
/// A refresh asked for again sooner than this (resume, then the network, then unlock)
/// is the same refresh.
const DEBOUNCE_MS: u64 = 60_000;

pub struct Calendar;

impl Module for Calendar {
    fn name(&self) -> &'static str {
        "calendar"
    }

    fn run(self: Box<Self>, mut ctx: ModuleContext) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async move { supervise("calendar", &mut ctx, |ctx| Box::pin(run(ctx))).await })
    }
}

/// What comes back from the blocking and network work.
enum Msg {
    Settings(Settings),
    /// A source's copy from disk, parsed.
    Cached { id: String, feed: Feed, validators: Validators },
    /// A refresh: a new feed, `None` when unchanged, or why it failed.
    Fetched { id: String, result: Result<Option<(Feed, Validators)>, String> },
    Store(Result<Vec<Event>, String>),
}

/// One ICS link and where its refreshing is.
struct Line {
    source: settings::Source,
    feed: Option<Arc<Feed>>,
    validators: Validators,
    failures: u32,
    /// Next refresh (Unix ms).
    due: u64,
    busy: bool,
    tried: u64,
}

struct State {
    settings: Option<Settings>,
    lines: Vec<Line>,
    store: Option<Snapshot>,
    store_due: u64,
    store_busy: bool,
    /// The occurrences of the days around today, and the day they were worked out for.
    events: Vec<Event>,
    expanded_for: Option<(chrono::NaiveDate, usize)>,
    dirty: bool,
    /// Starts dismissed (joined, or opened and closed).
    seen: HashSet<String>,
    /// The `now` card has been opened, so closing it dismisses the start.
    now_opened: bool,
    /// The event each published card is about, for Join.
    shown: HashMap<&'static str, String>,
    today_open: bool,
    status: Status,
    client: Option<Client>,
    tx: UnboundedSender<Msg>,
}

async fn run(ctx: &mut ModuleContext) -> Result<(), String> {
    let (tx, mut msgs) = mpsc::unbounded_channel();
    let (events_tx, mut events) = mpsc::unbounded_channel();
    let _subscription = tokio::task::spawn_blocking(move || system_events::subscribe(events_tx)).await.ok().flatten();
    let (online_tx, mut online) = mpsc::unbounded_channel();
    let _watch = network::watch(online_tx);
    let mut state = State {
        settings: None,
        lines: Vec::new(),
        store: None,
        store_due: 0,
        store_busy: false,
        events: Vec::new(),
        expanded_for: None,
        dirty: true,
        seen: HashSet::new(),
        now_opened: false,
        shown: HashMap::new(),
        today_open: false,
        status: Status::new(),
        client: fetch::client(),
        tx,
    };
    state.load_settings();

    loop {
        let deadline = state.step(ctx);
        let sleep = deadline.map(|at| Instant::now() + Duration::from_millis(at.saturating_sub(unix_ms())));
        tokio::select! {
            input = ctx.next() => match input {
                Some(ModuleInput::Action(action)) => state.act(ctx, action),
                Some(_) => {}
                None => return Ok(()),
            },
            Some(msg) = msgs.recv() => state.receive(msg),
            Some(event) = events.recv() => match event {
                // Waits don't count sleep: look again now, and the feeds may have changed.
                SystemEvent::Resumed => state.refresh_all(),
                // All-day and floating times are the PC's zone: work them out again.
                SystemEvent::TimeChanged => state.dirty = true,
                _ => continue,
            },
            Some(()) = online.recv() => state.retry_failed(),
            _ = sleep_until(sleep.unwrap_or_else(Instant::now)), if sleep.is_some() => {}
        }
    }
}

impl State {
    /// Starts what's due, brings the events up to date, publishes, and says when to wake.
    fn step(&mut self, ctx: &ModuleContext) -> Option<u64> {
        let now = unix_ms();
        self.start_due(now);
        self.expand(now);
        let wake = self.publish(ctx, now);
        let refreshes = self.lines.iter().filter(|l| l.source.on && !l.busy).map(|l| l.due);
        let store = self.settings.as_ref().is_some_and(|s| s.windows).then_some(self.store_due).filter(|_| !self.store_busy);
        [wake, refreshes.min(), store].into_iter().flatten().min()
    }

    fn load_settings(&self) {
        let tx = self.tx.clone();
        tokio::task::spawn_blocking(move || _ = tx.send(Msg::Settings(Settings::load())));
    }

    fn apply_settings(&mut self, settings: Settings) {
        let ids: HashSet<&str> = settings.sources.iter().map(|s| s.id.as_str()).collect();
        for gone in self.lines.iter().filter(|l| !ids.contains(l.source.id.as_str())) {
            let id = gone.source.id.clone();
            self.status.remove(&id);
            tokio::task::spawn_blocking(move || fetch::forget_cache(&id));
        }
        self.lines.retain(|l| ids.contains(l.source.id.as_str()));
        for source in &settings.sources {
            match self.lines.iter_mut().find(|l| l.source.id == source.id) {
                // Same id, new link: start over.
                Some(line) if line.source.url != source.url => *line = Line::new(source.clone()),
                Some(line) => line.source = source.clone(),
                None => {
                    self.lines.push(Line::new(source.clone()));
                    let (id, tx) = (source.id.clone(), self.tx.clone());
                    tokio::task::spawn_blocking(move || {
                        if let Some(c) = fetch::load_cache(&id) {
                            let feed = Feed::parse(&id, &c.text);
                            _ = tx.send(Msg::Cached { id, feed, validators: c.validators });
                        }
                    });
                }
            }
        }
        if !settings.windows {
            self.store = None;
            self.status.remove(WINDOWS);
        }
        println!("calendar: {} calendar link(s){}", self.lines.len(), if settings.windows { ", and Windows' store" } else { "" });
        self.settings = Some(settings);
        self.dirty = true;
        self.save_status();
    }

    fn receive(&mut self, msg: Msg) {
        let now = unix_ms();
        match msg {
            Msg::Settings(s) => self.apply_settings(s),
            Msg::Cached { id, feed, validators } => {
                if let Some(line) = self.lines.iter_mut().find(|l| l.source.id == id && l.feed.is_none()) {
                    line.feed = Some(Arc::new(feed));
                    line.validators = validators;
                    self.dirty = true;
                }
            }
            Msg::Fetched { id, result } => {
                let Some(line) = self.lines.iter_mut().find(|l| l.source.id == id) else { return };
                line.busy = false;
                let refresh = match result {
                    Ok(fresh) => {
                        if let Some((feed, validators)) = fresh {
                            if feed.unsupported > 0 {
                                println!("calendar: {} recurring event(s) in a feed repeat in a way only their first time is shown for", feed.unsupported);
                            }
                            println!("calendar: a feed refreshed ({} events)", feed.len());
                            line.feed = Some(Arc::new(feed));
                            line.validators = validators;
                            self.dirty = true;
                        }
                        line.failures = 0;
                        Refresh { ok: true, at: now, events: line.feed.as_ref().map_or(0, |f| f.len()), error: None }
                    }
                    Err(why) => {
                        line.failures += 1;
                        eprintln!("calendar: a feed didn't refresh: {why}");
                        Refresh { ok: false, at: now, events: line.feed.as_ref().map_or(0, |f| f.len()), error: Some(why) }
                    }
                };
                line.due = now + fetch::retry_after(line.failures);
                self.status.insert(id, refresh);
                self.save_status();
            }
            Msg::Store(result) => {
                self.store_busy = false;
                self.store_due = now + fetch::REFRESH_MS;
                let refresh = match result {
                    Ok(events) => {
                        let n = events.len();
                        self.store = Some(Snapshot(events));
                        self.dirty = true;
                        Refresh { ok: true, at: now, events: n, error: None }
                    }
                    Err(why) => {
                        eprintln!("calendar: Windows' calendar store can't be read: {why}");
                        Refresh { ok: false, at: now, events: 0, error: Some(why) }
                    }
                };
                if self.settings.as_ref().is_some_and(|s| s.windows) {
                    self.status.insert(WINDOWS.into(), refresh);
                    self.save_status();
                }
            }
        }
    }

    fn save_status(&self) {
        let status = self.status.clone();
        tokio::task::spawn_blocking(move || status::save(&status));
    }

    /// Resume: every source again, now.
    fn refresh_all(&mut self) {
        let now = unix_ms();
        for line in self.lines.iter_mut().filter(|l| now.saturating_sub(l.tried) > DEBOUNCE_MS) {
            line.due = now;
        }
        self.store_due = now;
        self.dirty = true;
    }

    /// The internet is back: the sources that failed, now.
    fn retry_failed(&mut self) {
        let now = unix_ms();
        for line in self.lines.iter_mut().filter(|l| l.failures > 0 && now.saturating_sub(l.tried) > DEBOUNCE_MS) {
            line.due = now;
        }
    }

    fn start_due(&mut self, now: u64) {
        let Some(client) = self.client.clone() else { return };
        for line in self.lines.iter_mut().filter(|l| l.source.on && !l.busy && l.due <= now) {
            line.busy = true;
            line.tried = now;
            let (id, url, known, tx, client) = (line.source.id.clone(), line.source.url.clone(), line.validators.clone(), self.tx.clone(), client.clone());
            tokio::spawn(async move {
                let result = match fetch::get(&client, &url, &known).await {
                    Ok(Fetched::NotModified) => Ok(None),
                    Ok(Fetched::Body { text, validators }) => {
                        let id = id.clone();
                        tokio::task::spawn_blocking(move || {
                            let feed = Feed::parse(&id, &text);
                            fetch::save_cache(&id, &Cached { validators: validators.clone(), text });
                            Some((feed, validators))
                        })
                        .await
                        .map_err(|_| "couldn't be read".to_string())
                    }
                    Err(why) => Err(why),
                };
                _ = tx.send(Msg::Fetched { id, result });
            });
        }
        if self.settings.as_ref().is_some_and(|s| s.windows) && !self.store_busy && self.store_due <= now {
            self.store_busy = true;
            let tx = self.tx.clone();
            tokio::task::spawn_blocking(move || _ = tx.send(Msg::Store(windows_store::read(now.saturating_sub(DAY_MS), 3 * DAY_MS))));
        }
    }

    /// The occurrences from yesterday to the day after tomorrow, worked out again when a
    /// source changed, the day turned or the zone changed.
    fn expand(&mut self, now: u64) {
        let today = Local::now().date_naive();
        let key = (today, self.lines.len());
        if !self.dirty && self.expanded_for == Some(key) {
            return;
        }
        let start = midnight(today);
        let (from, to) = (start.saturating_sub(DAY_MS), start + 3 * DAY_MS);
        let mut events: Vec<Event> = self.lines.iter().filter(|l| l.source.on).filter_map(|l| l.feed.as_ref()).flat_map(|f| f.events(from, to)).collect();
        if let Some(store) = &self.store {
            events.extend(store.events(from, to));
        }
        events.sort_by(|a, b| (a.start, a.end, &a.title).cmp(&(b.start, b.end, &b.title)));
        // Dismissed starts are kept only while their event is still to come or on.
        self.seen.retain(|k| events.iter().any(|e| &e.key == k && e.end > now));
        self.events = events;
        self.expanded_for = Some(key);
        self.dirty = false;
    }

    fn lead(&self) -> u64 {
        self.settings.as_ref().map_or(10 * 60_000, Settings::lead_ms)
    }

    /// Publishes what the schedule says now; returns when it next changes.
    fn publish(&mut self, ctx: &ModuleContext, now: u64) -> Option<u64> {
        let today = Local::now().date_naive();
        let (day_end, tomorrow_end) = (midnight(today + Days::new(1)), midnight(today + Days::new(2)));
        let lead = self.lead();
        let plan = schedule::plan(&self.events, now, lead, &self.seen, day_end, tomorrow_end);
        let after = |e: &Event| -> Vec<&Event> { self.events.iter().filter(|x| !x.all_day && x.start >= e.start && x.key != e.key && x.start < day_end).take(schedule::DAY_LINES - 1).collect() };
        let mut shown = HashMap::new();
        match plan.soon {
            Some(e) => {
                ctx.publish(NEXT, Priority::Ongoing, view::countdown(e, &after(e), now, lead));
                shown.insert(NEXT, e.key.clone());
            }
            None => ctx.retract(NEXT),
        }
        match plan.starting {
            Some(e) => {
                ctx.publish(NOW, Priority::Alert, view::starting(e, &after(e), now, lead));
                shown.insert(NOW, e.key.clone());
            }
            None => ctx.retract(NOW),
        }
        if shown.get(NOW) != self.shown.get(NOW) {
            self.now_opened = false;
        }
        if self.today_open {
            let none = self.lines.is_empty() && !self.settings.as_ref().is_some_and(|s| s.windows);
            ctx.publish(TODAY, Priority::Passive, view::day(&plan.day, plan.tomorrow, now, lead, none));
            if let Some(e) = plan.day.first() {
                shown.insert(TODAY, e.key.clone());
            }
        }
        self.shown = shown;
        plan.wake
    }

    fn act(&mut self, ctx: &ModuleContext, ModuleAction { key, action }: ModuleAction) {
        match (key.as_str(), action.as_str()) {
            ("", "reload") => self.load_settings(),
            ("", "refresh") => self.refresh_all(),
            ("", "open") => self.open(ctx),
            (_, view::ACTION_JOIN) => self.join(&key),
            (_, view::ACTION_SETTINGS) => open_settings(),
            (NOW, ACTION_EXPANDED) => self.now_opened = true,
            (NOW, ACTION_COLLAPSED) if self.now_opened => {
                if let Some(k) = self.shown.get(NOW) {
                    self.seen.insert(k.clone());
                }
            }
            (TODAY, ACTION_COLLAPSED) => {
                self.today_open = false;
                self.shown.remove(TODAY);
                ctx.retract(TODAY);
            }
            _ => {}
        }
    }

    /// The view switcher's tab or the tray: the meeting that's counting down or starting
    /// if there is one (it's the calendar's card already), else the day's.
    fn open(&mut self, ctx: &ModuleContext) {
        if self.shown.contains_key(NOW) {
            return ctx.focus(NOW);
        }
        if self.shown.contains_key(NEXT) {
            return ctx.focus(NEXT);
        }
        self.today_open = true;
        // Published by the step that follows; focus is sent after the snapshot has it.
        let now = unix_ms();
        self.expand(now);
        self.publish(ctx, now);
        ctx.focus(TODAY);
    }

    /// Join: the card's event's meeting link to Windows; its start is answered.
    fn join(&mut self, key: &str) {
        let Some(k) = self.shown.get(key).cloned() else { return };
        let Some(link) = self.events.iter().find(|e| e.key == k).and_then(|e| e.link.clone()) else { return };
        println!("calendar: joining a {} meeting", link.service.name());
        self.seen.insert(k);
        tokio::task::spawn_blocking(move || open_link(&link.url));
    }
}

impl Line {
    fn new(source: settings::Source) -> Self {
        Self { source, feed: None, validators: Validators::default(), failures: 0, due: 0, busy: false, tried: 0 }
    }
}

/// Local midnight starting `day` (Unix ms).
fn midnight(day: chrono::NaiveDate) -> u64 {
    ics::zones::local_ms(day.and_time(NaiveTime::MIN)).max(0) as u64
}

/// Hands a meeting link (https only, checked by `links`) to its default handler: the
/// browser, which passes it on to Teams or Zoom when they're installed. Blocking.
fn open_link(url: &str) {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    if !url.starts_with("https://") {
        return;
    }
    let result = unsafe { ShellExecuteW(None, &HSTRING::from("open"), &HSTRING::from(url), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
    if result.0 as isize <= 32 {
        eprintln!("calendar: can't open the meeting link ({})", result.0 as isize);
    }
}

/// "Add a calendar": Settings on its Calendar page (beside the hub, as the welcome is).
fn open_settings() {
    let Ok(exe) = std::env::current_exe() else { return };
    if let Err(e) = std::process::Command::new(exe.with_file_name("reui-settings.exe")).args(["--page", "calendar"]).spawn() {
        eprintln!("calendar: can't open Settings: {e}");
    }
}
