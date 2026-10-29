//! Privacy indicators: which app is using the microphone, camera, screen capture or
//! location right now, like iOS's green and orange dots (docs/PRD-privacy-indicators.md).
//!
//! Event flow, no polling:
//! - the consent store (`store.rs`, its own thread): a registry change notification on
//!   Windows' Capability Access Manager records, plus a wait on every process holding a
//!   use, read into "in use now" (`consent.rs`) and checked against running processes
//!   (`alive.rs`);
//! - the microphone's mute: the shared Core Audio watcher (`modules/audio`), and its
//!   `set_capture_muted` for the card's mute button;
//! - when uses show and go (`tracker.rs`): a screen capture must outlast a screenshot, a
//!   short camera or microphone use stays a moment; one timer for the next of those.
//!
//! Activities (module `privacy`): `in-use` (Ongoing) while anything is in use, the most
//! important kind's colour as its accent; `started` (Brief) when another app or kind
//! starts while it's up. ReUI's own programs never count (the island's visualizer
//! captures loopback audio).
//!
//! Calls (docs/PRD-calls.md) live here rather than in a module of their own: a call is
//! this module's own records read one way further (a call app holding the microphone,
//! `calls.rs`), and the call card has to replace this module's indicator for that app.
//! One module means one consent-store watcher and no hand-off between modules. Each call
//! is `call:<app>` (Ongoing); the indicator leaves those apps out.
//!
//! Files: `view.rs`, `call_view.rs`, `settings.rs` (`privacy.json`).

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use reui_protocol::{ImageId, Priority};
use tokio::sync::mpsc;
use tokio::time::{sleep_until, Instant};

mod alive;
mod call_view;
mod calls;
mod consent;
mod settings;
mod store;
mod tracker;
mod view;

use self::alive::Live;
use self::call_view::CallItem;
use self::calls::Calls;
use self::consent::{App, Kind};
use self::settings::Settings;
use self::store::Watcher;
use self::tracker::{Report, Tracker};
use self::view::Item;
use super::audio::{self, AudioEvent};
use super::{supervise, Module};
use crate::hub::{ModuleAction, ModuleContext, ModuleInput};

const INDICATOR: &str = "in-use";
const STARTED: &str = "started";
/// How long "another app started" stays.
const BRIEF_FOR: Duration = Duration::from_secs(3);
/// What a moment without a card sends when it's clicked.
const ACTION_OPEN: &str = "open";

pub struct Privacy;

impl Module for Privacy {
    fn name(&self) -> &'static str {
        "privacy"
    }

    fn run(self: Box<Self>, mut ctx: ModuleContext) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async move { supervise("privacy", &mut ctx, |ctx| Box::pin(run(ctx))).await })
    }
}

/// Results of work done off the hub thread.
enum Internal {
    Started { store: Option<Watcher>, audio: Option<audio::Watch>, muted: Option<bool> },
    Settings(Settings),
    Icon(String, Option<Vec<u8>>),
}

struct State {
    settings: Settings,
    internal: mpsc::UnboundedSender<Internal>,
    store: Option<Watcher>,
    audio: Option<audio::Watch>,
    tracker: Tracker,
    /// The watcher's latest list, unfiltered, so a kind switched back on shows at once.
    live: Vec<Live>,
    muted: Option<bool>,
    /// Icons by exe path: `None` once it's known there's none.
    icons: HashMap<String, Option<ImageId>>,
    loading: HashSet<String>,
    brief_until: Option<Instant>,
    calls: Calls,
    /// The call activities published, so an ended call is retracted.
    call_keys: HashSet<String>,
}

async fn run(ctx: &mut ModuleContext) -> Result<(), String> {
    let settings = tokio::task::spawn_blocking(Settings::load).await.unwrap_or_default();
    let (internal_tx, mut internal) = mpsc::unbounded_channel();
    let (live_tx, mut live_rx) = mpsc::unbounded_channel();
    let (audio_tx, mut audio_rx) = mpsc::unbounded_channel();
    let mut p = State {
        settings,
        internal: internal_tx.clone(),
        store: None,
        audio: None,
        tracker: Tracker::default(),
        live: Vec::new(),
        muted: None,
        icons: HashMap::new(),
        loading: HashSet::new(),
        brief_until: None,
        calls: Calls::default(),
        call_keys: HashSet::new(),
    };
    tokio::spawn(async move {
        let started = tokio::task::spawn_blocking(move || {
            let store = store::watch(live_tx);
            let audio = audio::watch_events(audio_tx, true);
            Internal::Started { store, audio, muted: audio::capture_muted() }
        });
        if let Ok(started) = started.await {
            _ = internal_tx.send(started);
        }
    });

    loop {
        let deadline = [p.tracker.next_deadline().map(Instant::from_std), p.brief_until, p.calls.next_deadline().map(at_unix)].into_iter().flatten().min();
        tokio::select! {
            Some(live) = live_rx.recv() => {
                p.live = live;
                p.on_live(ctx);
            }
            Some(event) = audio_rx.recv() => {
                if let AudioEvent::CaptureMute { muted } = event {
                    if p.muted != Some(muted) {
                        p.muted = Some(muted);
                        p.refresh(ctx);
                    }
                }
            }
            Some(message) = internal.recv() => p.on_internal(ctx, message),
            _ = sleep_until(deadline.unwrap_or_else(Instant::now)), if deadline.is_some() => p.tick(ctx),
            input = ctx.next() => match input {
                Some(ModuleInput::Action(action)) => p.act(ctx, action),
                Some(_) => {}
                None => break,
            },
        }
    }
    // Stopping joins the watcher threads; keep it off the hub thread.
    let held = (p.store.take(), p.audio.take());
    tokio::task::spawn_blocking(move || drop(held));
    Ok(())
}

/// A Unix-ms deadline as an Instant (calls run on the store's clock).
fn at_unix(ms: u64) -> Instant {
    Instant::now() + Duration::from_millis(ms.saturating_sub(reui_protocol::unix_ms()))
}

fn app_name(app: &App) -> String {
    match app {
        App::Exe(path) => super::clipboard::app_name(path),
        App::Packaged(family) => view::packaged_name(family),
    }
}

impl State {
    fn on_live(&mut self, ctx: &ModuleContext) {
        let all: Vec<Report> = self.live.iter().map(|l| Report { kind: l.using.kind, app: l.using.app.clone(), since: l.using.since, exe: l.exe.clone() }).collect();
        let settings = &self.settings;
        self.calls.update(&all, |f| settings.calls_from(f), reui_protocol::unix_ms());
        let reports: Vec<Report> = all.into_iter().filter(|r| self.settings.shows(r.kind)).collect();
        for exe in reports.iter().filter_map(|r| r.exe.clone()) {
            self.load_icon(exe);
        }
        let started = self.tracker.update(&reports, std::time::Instant::now());
        self.refresh(ctx);
        self.brief(ctx, started);
    }

    fn tick(&mut self, ctx: &ModuleContext) {
        let now = Instant::now();
        if self.brief_until.is_some_and(|at| now >= at) {
            self.brief_until = None;
            ctx.retract(STARTED);
        }
        let started = self.tracker.tick(now.into_std());
        self.calls.tick(reui_protocol::unix_ms());
        self.refresh(ctx);
        self.brief(ctx, started);
    }

    /// What the indicator shows: everything but the apps in a call, whose card says it.
    fn items(&self) -> Vec<Item> {
        self.tracker
            .shown()
            .into_iter()
            .filter(|s| !self.calls.has_call(&s.app))
            .map(|s| Item {
                kind: s.kind,
                name: app_name(&s.app),
                icon: s.exe.as_ref().and_then(|exe| self.icons.get(exe).copied().flatten()),
                since: super::timers::time_of_day(consent::unix_ms(s.since)),
                app: s.app,
            })
            .collect()
    }

    fn refresh(&mut self, ctx: &ModuleContext) {
        match view::indicator(&self.items(), self.muted) {
            Some(v) => ctx.publish(INDICATOR, Priority::Ongoing, v),
            None => ctx.retract(INDICATOR),
        }
        let mut keys = HashSet::new();
        for call in self.calls.calls() {
            let key = call_view::key(&call);
            if let Some(exe) = call.exe.clone() {
                self.load_icon(exe);
            }
            let icon = call.exe.as_ref().and_then(|exe| self.icons.get(exe).copied().flatten());
            let item = CallItem { name: app_name(&call.app), icon, call };
            ctx.publish(&key, Priority::Ongoing, call_view::call(&item, self.muted));
            keys.insert(key);
        }
        for gone in self.call_keys.difference(&keys) {
            ctx.retract(gone);
        }
        self.call_keys = keys;
    }

    /// The most important of what just started, for a moment.
    fn brief(&mut self, ctx: &ModuleContext, started: Vec<(Kind, App)>) {
        let Some((kind, app)) = started.into_iter().min_by_key(|(k, _)| *k) else { return };
        // An app in a call says what it starts on its own card.
        let Some(item) = self.items().into_iter().find(|i| i.kind == kind && i.app == app) else { return };
        ctx.publish(STARTED, Priority::Brief, view::started(&item));
        self.brief_until = Some(Instant::now() + BRIEF_FOR);
    }

    fn load_icon(&mut self, exe: String) {
        if self.icons.contains_key(&exe) || !self.loading.insert(exe.clone()) {
            return;
        }
        let tx = self.internal.clone();
        tokio::spawn(async move {
            let path = exe.clone();
            let png = tokio::task::spawn_blocking(move || super::clipboard::exe_icon(&path)).await.ok().flatten();
            _ = tx.send(Internal::Icon(exe, png));
        });
    }

    fn on_internal(&mut self, ctx: &ModuleContext, message: Internal) {
        match message {
            Internal::Started { store, audio, muted } => {
                if store.is_none() {
                    eprintln!("privacy: no consent store watch; no indicators");
                }
                (self.store, self.audio, self.muted) = (store, audio, muted);
                self.refresh(ctx);
            }
            Internal::Settings(settings) => self.apply_settings(ctx, settings),
            Internal::Icon(exe, png) => {
                self.loading.remove(&exe);
                self.icons.insert(exe, png.map(|b| ctx.image(b)));
                self.refresh(ctx);
            }
        }
    }

    fn act(&mut self, ctx: &ModuleContext, action: ModuleAction) {
        let (key, a) = (action.key.as_str(), action.action.as_str());
        if key.is_empty() {
            // The Settings window wrote privacy.json.
            if a == "reload" {
                let tx = self.internal.clone();
                tokio::spawn(async move {
                    if let Ok(settings) = tokio::task::spawn_blocking(Settings::load).await {
                        _ = tx.send(Internal::Settings(settings));
                    }
                });
                return;
            }
            let mut settings = self.settings.clone();
            if settings.apply(a) {
                let saved = settings.clone();
                tokio::task::spawn_blocking(move || saved.save_to(&Settings::path()));
                self.apply_settings(ctx, settings);
            }
            return;
        }
        match a {
            view::ACTION_MUTE | view::ACTION_UNMUTE => {
                let muted = a == view::ACTION_MUTE;
                // Shown at once; the mute event confirms it (or puts it back).
                self.muted = Some(muted);
                self.refresh(ctx);
                tokio::task::spawn_blocking(move || {
                    if !audio::set_capture_muted(muted) {
                        eprintln!("privacy: can't change the microphone's mute");
                    }
                });
            }
            view::ACTION_SETTINGS => {
                let kind = self.tracker.shown().first().map_or(Kind::Microphone, |s| s.kind);
                tokio::task::spawn_blocking(move || open_privacy_settings(kind));
            }
            call_view::ACTION_OPEN_APP => {
                let Some(call) = self.calls.calls().into_iter().find(|c| call_view::key(c) == key) else { return };
                let pids: Vec<u32> = self.live.iter().filter(|l| l.using.app == call.app).flat_map(|l| l.pids.clone()).collect();
                let file = call.app.file_name().or_else(|| call.exe.as_deref().and_then(|e| App::Exe(e.into()).file_name()));
                tokio::task::spawn_blocking(move || open_app(&pids, file.as_deref()));
            }
            ACTION_OPEN if key == STARTED => {
                self.brief_until = None;
                ctx.retract(STARTED);
                ctx.focus(INDICATOR);
            }
            _ => {}
        }
    }

    fn apply_settings(&mut self, ctx: &ModuleContext, settings: Settings) {
        if settings == self.settings {
            return;
        }
        self.settings = settings;
        for kind in Kind::ALL.into_iter().filter(|k| !self.settings.shows(*k)) {
            self.tracker.drop_kind(kind);
        }
        self.on_live(ctx);
    }
}

/// Brings a call app's window forward: one of the processes holding the microphone, else
/// any process of the same exe (Teams' window and its audio may be different processes).
/// The island allowed the foreground with the click.
fn open_app(pids: &[u32], file: Option<&str>) {
    use super::media::player;
    let windows = player::app_windows();
    let by_pid = windows.iter().find(|(_, pid)| pids.contains(pid));
    let by_exe = || windows.iter().find(|(_, pid)| file.is_some() && player::exe_name(*pid).as_deref() == file);
    match by_pid.or_else(by_exe) {
        Some(&(hwnd, _)) => player::focus(hwnd),
        None => eprintln!("privacy: no window to open for {file:?}"),
    }
}

/// Settings › Privacy & security, on the page for what's in use.
fn open_privacy_settings(kind: Kind) {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let page = match kind {
        Kind::Microphone => "ms-settings:privacy-microphone",
        Kind::Camera => "ms-settings:privacy-webcam",
        Kind::Location => "ms-settings:privacy-location",
        Kind::Screen => "ms-settings:privacy-graphicscaptureprogrammatic",
    };
    unsafe {
        _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let result = ShellExecuteW(None, &HSTRING::from("open"), &HSTRING::from(page), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
        if result.0 as isize <= 32 {
            eprintln!("privacy: can't open {page} ({})", result.0 as isize);
        }
    }
}
