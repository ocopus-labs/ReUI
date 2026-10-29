//! Updates (PRD product shell, P4): the island's side of them. The networking lives in
//! Settings (`reui-settings --check-updates` asks GitHub for the latest release and notes
//! it in `%LOCALAPPDATA%\ReUI\update.json`; `--install-update` downloads, verifies and
//! starts it); this module decides when to run those and what the island says:
//!
//! - **Daily check**, when "Check automatically" is on (`check_updates` in `settings.json`,
//!   default on): ten minutes after start and then hourly it looks at `checked_ms`, and
//!   when the last check is a day old it runs the check. No network here, none at all
//!   when it's off, and no card either.
//! - **Update card**, an Alert (it waits for an answer but never takes over the island):
//!   when a check finds a version, and on start while one is known. Install runs
//!   `--install-update` and the card follows its printed steps (downloading with a bar,
//!   installing; the setup then restarts ReUI) or says why it failed (Later / Try again).
//!   Later puts that version off for a day (`snoozed`, `snoozed_until`); a newer one shows
//!   at once. The island's tray menu sends `install` for the same flow.
//! - **Updated**, once, on the first start of a new version: a Brief with "What's new"
//!   (unless `whats_new` is off); `version` / `updated_from` tell Settings to show the
//!   CHANGELOG entry.
//!
//! Files: `state.rs` (update.json and the decisions from it), `card.rs` (the card's phases),
//! `view.rs` (what each looks like), `child.rs` (the reui-settings runs).

mod card;
mod child;
mod state;
mod view;

use std::future::{pending, Future};
use std::pin::Pin;
use std::time::Duration;

use reui_protocol::Priority;
use tokio::sync::mpsc;
use tokio::time::{sleep, Instant};

use self::card::Card;
use self::child::Event;
use super::Module;
use crate::hub::{ModuleContext, ModuleInput};

const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The first look waits until the PC has settled after sign-in.
const FIRST_LOOK: Duration = Duration::from_secs(10 * 60);
const LOOK_EVERY: Duration = Duration::from_secs(60 * 60);
const SHOW: Duration = Duration::from_secs(8);
/// The setup normally stops ReUI within seconds of starting; still here after this, it
/// didn't.
const INSTALL_WAIT: Duration = Duration::from_secs(2 * 60);
/// Progress reaches the island at most this often (DESIGN: progress once a second).
const PROGRESS_EVERY: Duration = Duration::from_secs(1);

const AVAILABLE: &str = "available";
const UPDATED: &str = "updated";

pub struct Updates;

impl Module for Updates {
    fn name(&self) -> &'static str {
        "updates"
    }

    fn run(self: Box<Self>, ctx: ModuleContext) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(Run::new(ctx).run())
    }
}

struct Run {
    ctx: ModuleContext,
    card: Option<Card>,
    /// The install's reports while it runs.
    install: Option<mpsc::UnboundedReceiver<Event>>,
    /// Briefs shown until this time, then retracted.
    showing: Vec<(&'static str, Instant)>,
    next_look: Instant,
    /// When the setup took over; a failure if ReUI is still here [`INSTALL_WAIT`] later.
    stall_at: Option<Instant>,
    /// A progress change waiting for its second.
    publish_at: Option<Instant>,
    published: Instant,
}

impl Run {
    fn new(ctx: ModuleContext) -> Self {
        let now = Instant::now();
        Self { ctx, card: None, install: None, showing: Vec::new(), next_look: now + FIRST_LOOK, stall_at: None, publish_at: None, published: now }
    }

    async fn run(mut self) {
        let (pairs, updated_from) = state::version_change(&state::load(), VERSION);
        if !pairs.is_empty() {
            state::write(pairs);
        }
        if updated_from.is_some() && state::setting_on("whats_new") {
            self.ctx.publish(UPDATED, Priority::Brief, view::updated(VERSION));
            self.showing.push((UPDATED, Instant::now() + SHOW));
        }
        self.offer();

        loop {
            let wake = self.showing.iter().map(|(_, t)| *t).chain([Some(self.next_look), self.stall_at, self.publish_at].into_iter().flatten()).min().unwrap_or(self.next_look);
            let install = &mut self.install;
            let reports = async {
                match install.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => pending().await,
                }
            };
            tokio::select! {
                input = self.ctx.next() => match input {
                    Some(ModuleInput::Action(action)) => self.act(&action.key, &action.action),
                    Some(_) => {}
                    None => break,
                },
                event = reports => self.on_install(event),
                _ = sleep(wake.saturating_duration_since(Instant::now())) => self.on_timer().await,
            }
        }
    }

    fn act(&mut self, key: &str, action: &str) {
        match (key, action) {
            (UPDATED, view::OPEN) => {
                child::open_about();
                self.ctx.retract(UPDATED);
                self.showing.retain(|(k, _)| *k != UPDATED);
            }
            (AVAILABLE, view::INSTALL) | ("", view::INSTALL) => self.install(),
            (AVAILABLE, view::LATER) => {
                if let Some(card) = self.card.take_if(|c| !c.busy()) {
                    state::write(state::snooze(&card.offer.version, reui_protocol::unix_ms()));
                    self.ctx.retract(AVAILABLE);
                }
            }
            // Settings changed: "Check automatically" off takes an unanswered card away,
            // back on brings it back.
            ("", "reload") => {
                if state::setting_on("check_updates") {
                    self.offer();
                } else if self.card.take_if(|c| !c.busy()).is_some() {
                    self.ctx.retract(AVAILABLE);
                }
            }
            _ => {}
        }
    }

    /// Install, Try again or the tray: start the install, unless one is running.
    fn install(&mut self) {
        if self.card.as_ref().is_some_and(Card::busy) {
            return;
        }
        // The tray asks without a card in front (it may have been put off): install what's known.
        let Some(mut card) = self.card.take().or_else(|| state::known(&state::load(), VERSION).map(Card::new)) else { return };
        card.start();
        match child::start_install() {
            Ok(rx) => self.install = Some(rx),
            Err(e) => card.step(reui_protocol::install::InstallStep::Error(e)),
        }
        self.card = Some(card);
        self.show();
    }

    fn on_install(&mut self, event: Option<Event>) {
        let Some(card) = self.card.as_mut() else {
            self.install = None;
            return;
        };
        let before = card.phase.clone();
        match event {
            Some(Event::Step(step)) => card.step(step),
            Some(Event::Exited(success)) => {
                card.exited(success);
                self.install = None;
                if card.phase == card::Phase::Installing {
                    self.stall_at = Some(Instant::now() + INSTALL_WAIT);
                }
            }
            None => {
                card.exited(false);
                self.install = None;
            }
        }
        let progress_only = matches!((&before, &card.phase), (card::Phase::Downloading(_), card::Phase::Downloading(_)));
        if !progress_only {
            self.show();
        } else if self.publish_at.is_none() {
            self.publish_at = Some((self.published + PROGRESS_EVERY).max(Instant::now()));
        }
    }

    async fn on_timer(&mut self) {
        let now = Instant::now();
        let ctx = &self.ctx;
        self.showing.retain(|(key, until)| {
            let done = *until <= now;
            if done {
                ctx.retract(key);
            }
            !done
        });
        if self.publish_at.is_some_and(|t| t <= now) {
            self.show();
        }
        if self.stall_at.is_some_and(|t| t <= now) {
            self.stall_at = None;
            if let Some(card) = self.card.as_mut() {
                card.stalled();
            }
            self.show();
        }
        if self.next_look > now {
            return;
        }
        self.next_look = now + LOOK_EVERY;
        if state::check_due(&state::load(), reui_protocol::unix_ms(), state::setting_on("check_updates")) {
            child::run_check().await;
        }
        // Also brings a put-off card back once its day is over.
        self.offer();
    }

    /// Shows the card for what `update.json` knows, when automatic checks are on and it
    /// isn't put off; an unanswered card for an older version gives way to it.
    fn offer(&mut self) {
        if self.card.as_ref().is_some_and(Card::busy) || !state::setting_on("check_updates") {
            return;
        }
        let Some(offer) = state::offer(&state::load(), VERSION, reui_protocol::unix_ms()) else { return };
        // A card for this version already waits for its answer (perhaps a failure's).
        if self.card.as_ref().is_some_and(|c| c.offer.version == offer.version) {
            return;
        }
        self.card = Some(Card::new(offer));
        self.show();
    }

    fn show(&mut self) {
        self.publish_at = None;
        self.published = Instant::now();
        if let Some(card) = &self.card {
            self.ctx.publish(AVAILABLE, Priority::Alert, view::card(card));
        }
    }
}
