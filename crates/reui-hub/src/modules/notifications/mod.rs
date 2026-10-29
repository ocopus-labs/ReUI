//! Notifications from apps the user picked, as a Brief on the island
//! (docs/PRD-notifications.md). Off by default: it runs only with `--notifications` or
//! `"enabled": true` in `notifications.json`, and shows only allow-listed apps.
//!
//! Event flow, without package identity (ReUI is a plain per-user exe):
//! - `wnf.rs`: the shell's notification count changes with every toast and removal, a
//!   free trigger with no polling;
//! - `listener.rs` (its own thread): on that trigger, lists the notification centre through
//!   `UserNotificationListener` (documented, works unpackaged) and reads only what's new;
//! - `schedule.rs`: when to list (debounce, a slow safety list, polling with backoff if the
//!   count ever stops working); `diff.rs`: what's new by id.
//!
//! Activities (module `notifications`): `toast` (Brief) for the newest new toast from an
//! allowed app, for `BRIEF_FOR`; not while Do Not Disturb is on. Clicking opens it; Open
//! starts or brings forward the app (its AUMID through `shell:AppsFolder`). Toasts are
//! never removed from the notification centre and nothing about them is stored or logged.
//!
//! Files: `view.rs`, `settings.rs` (`notifications.json`).

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reui_protocol::{ImageId, Priority};
use tokio::sync::mpsc;
use tokio::time::{sleep_until, Instant};

mod diff;
mod listener;
mod schedule;
mod settings;
mod view;
mod wnf;

pub use self::settings::enabled;
use self::listener::{Event, Filter, Listener, Toast};
use self::schedule::Trigger;
use self::settings::Settings;
use super::{supervise, Module};
use crate::hub::{ModuleAction, ModuleContext, ModuleInput};

const KEY: &str = "toast";
/// Long enough to read a title and a line, short enough to be a moment.
const BRIEF_FOR: Duration = Duration::from_secs(6);

pub struct Notifications;

impl Module for Notifications {
    fn name(&self) -> &'static str {
        "notifications"
    }

    fn run(self: Box<Self>, mut ctx: ModuleContext) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async move { supervise("notifications", &mut ctx, |ctx| Box::pin(run(ctx))).await })
    }
}

/// Results of work done off the hub thread.
enum Internal {
    Started(Option<Listener>),
    Settings(Settings),
}

struct State {
    settings: Settings,
    filter: Arc<Mutex<Filter>>,
    internal: mpsc::UnboundedSender<Internal>,
    listener: Option<Listener>,
    /// Logos by AUMID: `None` once it's known there's none.
    icons: HashMap<String, Option<ImageId>>,
    /// The toast on the island: its id and app.
    showing: Option<(u32, String)>,
    brief_until: Option<Instant>,
}

fn filter_of(settings: &Settings) -> Filter {
    Filter { apps: settings.apps.clone(), hide_content: settings.hide_content }
}

async fn run(ctx: &mut ModuleContext) -> Result<(), String> {
    let settings = tokio::task::spawn_blocking(Settings::load).await.unwrap_or_default();
    let filter = Arc::new(Mutex::new(filter_of(&settings)));
    let (internal_tx, mut internal) = mpsc::unbounded_channel();
    let (events_tx, mut events) = mpsc::unbounded_channel();
    let mut n = State { settings, filter: filter.clone(), internal: internal_tx.clone(), listener: None, icons: HashMap::new(), showing: None, brief_until: None };
    tokio::spawn(async move {
        if let Ok(listener) = tokio::task::spawn_blocking(move || listener::start(events_tx, filter)).await {
            _ = internal_tx.send(Internal::Started(listener));
        }
    });

    loop {
        tokio::select! {
            Some(event) = events.recv() => n.on_event(ctx, event),
            Some(message) = internal.recv() => match message {
                Internal::Started(listener) => n.listener = listener,
                Internal::Settings(settings) => n.apply_settings(settings),
            },
            _ = sleep_until(n.brief_until.unwrap_or_else(Instant::now)), if n.brief_until.is_some() => n.hide(ctx),
            input = ctx.next() => match input {
                Some(ModuleInput::Action(action)) => n.act(ctx, action),
                Some(_) => {}
                None => break,
            },
        }
    }
    // Stopping joins the listener thread; keep it off the hub thread.
    let held = n.listener.take();
    tokio::task::spawn_blocking(move || drop(held));
    Ok(())
}

impl State {
    fn on_event(&mut self, ctx: &ModuleContext, event: Event) {
        match event {
            Event::NoAccess => eprintln!("notifications: no access (Settings > Privacy & security > Notifications); not listening"),
            Event::Trigger(Trigger::Wnf) => println!("notifications: listening (on the notification count)"),
            Event::Trigger(Trigger::Poll) => println!("notifications: listening (polling)"),
            Event::Arrived(toasts) => self.arrived(ctx, toasts),
            Event::Gone(ids) => {
                // Dismissed or opened in the notification centre: done here too.
                if self.showing.as_ref().is_some_and(|(id, _)| ids.contains(id)) {
                    self.hide(ctx);
                }
            }
        }
    }

    fn arrived(&mut self, ctx: &ModuleContext, toasts: Vec<Toast>) {
        // Remember who notified (a picker offers them); saved only when an app is new.
        let mut new_app = false;
        for t in &toasts {
            new_app |= !self.settings.seen.iter().any(|s| s.app == t.app);
            self.settings.saw(&t.app, &t.name);
        }
        if new_app {
            let saved = self.settings.clone();
            tokio::task::spawn_blocking(move || saved.save_to(&Settings::path()));
        }
        let Some(toast) = toasts.into_iter().filter(|t| self.settings.allows(&t.app)).max_by_key(|t| t.id) else { return };
        // Windows holds banners back during Do Not Disturb; so does the island.
        if super::timers::dnd::is_on() == Some(true) {
            return;
        }
        let icon = match self.icons.get(&toast.app) {
            Some(icon) => *icon,
            None => {
                let icon = toast.logo.map(|png| ctx.image(png));
                self.icons.insert(toast.app.clone(), icon);
                icon
            }
        };
        let shown = view::Shown { name: toast.name, title: toast.title, body: toast.body, icon };
        ctx.publish(KEY, Priority::Brief, view::brief(&shown));
        self.showing = Some((toast.id, toast.app));
        self.brief_until = Some(Instant::now() + BRIEF_FOR);
    }

    fn hide(&mut self, ctx: &ModuleContext) {
        self.brief_until = None;
        self.showing = None;
        ctx.retract(KEY);
    }

    fn act(&mut self, ctx: &ModuleContext, action: ModuleAction) {
        let (key, a) = (action.key.as_str(), action.action.as_str());
        if key.is_empty() && a == "reload" {
            let tx = self.internal.clone();
            tokio::spawn(async move {
                if let Ok(settings) = tokio::task::spawn_blocking(Settings::load).await {
                    _ = tx.send(Internal::Settings(settings));
                }
            });
            return;
        }
        if key == KEY && a == view::ACTION_OPEN_APP {
            if let Some((_, app)) = self.showing.clone() {
                tokio::task::spawn_blocking(move || open_app(&app));
            }
            self.hide(ctx);
        }
    }

    fn apply_settings(&mut self, settings: Settings) {
        if let Ok(mut f) = self.filter.lock() {
            *f = filter_of(&settings);
        }
        self.settings = settings;
    }
}

/// Starts the app, or brings it forward, by its AUMID. What the toast itself would open
/// (a chat, a mail) is the app's own activation, which only Windows can invoke.
fn open_app(aumid: &str) {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let target = format!("shell:AppsFolder\\{aumid}");
    unsafe {
        _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let result = ShellExecuteW(None, &HSTRING::from("open"), &HSTRING::from(target.as_str()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
        if result.0 as isize <= 32 {
            eprintln!("notifications: can't open the app ({})", result.0 as isize);
        }
    }
}
