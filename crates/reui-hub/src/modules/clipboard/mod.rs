//! Clipboard history: every copy is saved (text with its rich formats, images, files) and
//! confirmed with a toast (the pill shows what was copied). The history itself lives in the
//! clipboard window (`docs/clipboard-window-contract.md`): the hotkey (Win+Alt+V by default)
//! or a click on the toast opens it next to the text caret, searching; picking an item
//! pastes it back into the field the user was typing in. Ctrl+Alt+1…9 paste a recent item
//! without opening anything.
//!
//! History lives in `%APPDATA%\ReUI\clipboard.db` (see [`store`]) and
//! never leaves the machine. Never recorded: content other apps mark private (checked by
//! the OS side), copies from ignored apps (password managers), anything that looks like a
//! secret ([`secrets`]), and everything while paused. Settings are in
//! [`settings`].
//!
//! Copied links get the page's title, site and image (see [`link`]) once they are
//! among the first cards of an open window, unless `--no-link-previews` is passed or the
//! setting is off. Images get their text recognised ([`ocr`]) right after capture,
//! and older ones a few at a time while the window is open.
//!
//! Island actions on the toast: `open` (or `expanded`) opens the window; tray commands:
//! `open`, `pause`, `resume`, `clear`.
//!
//! Files: `os/` (the Windows side), `store/` (the history), `window.rs` (the window's
//! messages), `view.rs` and `panel.rs` (what it shows), `toast.rs` (the island), `stack.rs`
//! (the paste stack), `preview.rs` (what text is), `transform.rs` (paste modes), `link.rs`,
//! `ocr.rs`, `secrets.rs` and `settings.rs`.

use std::collections::{HashSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use reqwest::Client;
use reui_protocol::{Priority, Rgb, ACTION_COLLAPSED, ACTION_EXPANDED};
use tokio::sync::mpsc::UnboundedSender;
use tokio::time::{sleep_until, Instant, MissedTickBehavior};

mod link;
mod ocr;
mod os;
mod panel;
mod preview;
mod secrets;
mod settings;
mod stack;
mod store;
mod toast;
mod transform;
mod view;
mod window;

use self::link::LinkMeta;
pub(crate) use self::os::thumbnail;
use self::os::{Captured, Clipboard, Command, Content, Hotkey, OsEvent, OsStatus, Target};
use self::settings::Settings;
use self::stack::PasteStack;
use self::store::{Db, Kind, Query, Saved, Store};
use self::view::{self as cv, Images, Ui};
use super::Module;
use crate::hub::{ModuleContext, ModuleInput};

const KEY: &str = "clipboard";
/// The paste stack's own activity ("Stack 2/5"), so a copy toast doesn't replace it.
const STACK_KEY: &str = "stack";
const ACCENT: Rgb = Rgb(10, 132, 255);
const TOAST: Duration = Duration::from_millis(2500);
const BLOCKED_TOAST: Duration = Duration::from_secs(3);
/// Cards sent for a view.
const ITEMS: usize = 60;
/// Links among this many first cards get looked up.
const LINK_LOOKAHEAD: usize = 10;
/// A failed link lookup is tried again after this long, not on every open.
const LINK_RETRY_MS: i64 = 24 * 3_600_000;
const UNDO: Duration = Duration::from_secs(5);
const PAUSE: Duration = Duration::from_secs(3600);
const RETENTION_EVERY: Duration = Duration::from_secs(3600);
/// A paste stack nobody uses for this long ends.
const STACK_IDLE: Duration = Duration::from_secs(60);
/// Older images queued for OCR each time the window opens.
const OCR_BATCH: usize = 3;

pub struct ClipboardHistory {
    /// Fetch title, site and image for copied links (the settings file can also turn it off).
    pub link_previews: bool,
}

impl Module for ClipboardHistory {
    fn name(&self) -> &'static str {
        "clipboard"
    }

    fn run(self: Box<Self>, ctx: ModuleContext) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(run(ctx, self.link_previews))
    }
}

/// What the island (the toast) or the tray sends.
#[derive(Debug, PartialEq)]
enum Action {
    Open,
    Pause,
    Resume,
    Clear,
}

impl Action {
    fn parse(action: &str) -> Option<Action> {
        Some(match action {
            "open" | ACTION_EXPANDED => Action::Open,
            "pause" => Action::Pause,
            "resume" => Action::Resume,
            "clear" => Action::Clear,
            _ => return None,
        })
    }
}

/// The one undoable change, until its time is up.
#[derive(Debug, Clone, PartialEq)]
enum Undo {
    Deleted(Vec<i64>),
    /// `fresh`: the edit made a new row (undo removes it); else it reused an existing copy.
    Edited { old: i64, new: i64, fresh: bool },
}

struct App {
    ctx: ModuleContext,
    db: Db,
    os: Clipboard,
    settings: Settings,
    client: Option<Client>,
    links_tx: UnboundedSender<(i64, Option<LinkMeta>)>,
    /// Lookups in flight.
    fetching: HashSet<i64>,
    /// Links among the first cards of the last view that need a lookup.
    shown_links: Vec<(i64, String)>,
    ocr_tx: UnboundedSender<(i64, Option<String>)>,
    ocr_queue: VecDeque<i64>,
    ocr_busy: bool,
    query: Query,
    /// The window is up (we opened it, or the host said so) until the host says closed.
    open: bool,
    /// Where pastes from the window go.
    target: Option<Target>,
    /// The toast (or hint) is retracted then.
    hide_at: Option<Instant>,
    paused_until: Option<Instant>,
    undo: Option<(Undo, Instant)>,
    stack: Option<PasteStack>,
    /// The hotkey couldn't be registered (E-C13). Kept for the tray, which can't ask yet.
    #[allow(dead_code)]
    hotkey_taken: bool,
    /// The view the notch panel shows, to turn its actions back into messages.
    last_view: Option<cv::View>,
}

async fn run(ctx: ModuleContext, link_previews: bool) {
    let settings = Settings::load();
    let Some(db) = Db::start(Store::open).await else { return };
    let hotkey = Hotkey::parse(&settings.hotkey).unwrap_or_else(|| {
        eprintln!("clipboard: hotkey \"{}\" not understood; using the default", settings.hotkey);
        Hotkey::default()
    });
    let (events_tx, mut events) = tokio::sync::mpsc::unbounded_channel();
    let Some(os) = os::start_with(events_tx, hotkey) else { return };
    let (links_tx, mut links) = tokio::sync::mpsc::unbounded_channel();
    let (ocr_tx, mut ocr_done) = tokio::sync::mpsc::unbounded_channel();
    let mut app = App {
        ctx,
        db,
        os,
        settings,
        client: link_previews.then(link::client).flatten(),
        links_tx,
        fetching: HashSet::new(),
        shown_links: Vec::new(),
        ocr_tx,
        ocr_queue: VecDeque::new(),
        ocr_busy: false,
        query: Query::default(),
        open: false,
        target: None,
        hide_at: None,
        paused_until: None,
        undo: None,
        stack: None,
        hotkey_taken: false,
        last_view: None,
    };
    // The first tick is immediate: retention runs at start, then hourly.
    let mut retention = tokio::time::interval(RETENTION_EVERY);
    retention.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            event = events.recv() => match event {
                Some(event) => app.on_os(event).await,
                None => break,
            },
            input = app.ctx.next() => match input {
                Some(ModuleInput::Action(action)) => {
                    if let Some(action) = Action::parse(&action.action) {
                        app.on_action(action).await;
                    } else if action.action == ACTION_COLLAPSED {
                        app.close_window();
                    } else if let Some(message) = app.last_view.as_ref().and_then(|v| panel::to_message(&action.action, v)) {
                        app.on_window(&message).await;
                        // A paste or copy closes the panel.
                        if !app.open {
                            app.close_window();
                        }
                    }
                }
                Some(_) => {}
                None => break,
            },
            Some((id, meta)) = links.recv() => app.on_link(id, meta).await,
            Some((id, text)) = ocr_done.recv() => app.on_ocr(id, text).await,
            _ = sleep_until(app.hide_at.unwrap_or_else(Instant::now)), if app.hide_at.is_some() => {
                app.hide_at = None;
                app.ctx.retract(KEY);
            }
            _ = sleep_until(app.undo.as_ref().map_or_else(Instant::now, |(_, until)| *until)), if app.undo.is_some() => {
                app.undo = None;
                app.db.call(|s| s.purge_deleted()).await;
                app.send_state().await;
            }
            _ = sleep_until(app.stack.as_ref().map_or_else(Instant::now, |s| s.idle_until)), if app.stack.is_some() => app.end_stack(),
            _ = retention.tick() => app.prune().await,
        }
    }
}

impl App {
    async fn on_os(&mut self, event: OsEvent) {
        match event {
            OsEvent::Copied(captured) => self.on_copy(captured).await,
            OsEvent::Hotkey(target) => {
                if self.open {
                    self.close_window();
                } else {
                    self.open_window(target).await;
                }
            }
            OsEvent::RecentPaste { n, target } => {
                let now = now_ms();
                let content = self
                    .db
                    .call(move |s| {
                        let Some(id) = s.recent(n)? else { return Ok(None) };
                        s.touch(id, now)?;
                        s.content(id)
                    })
                    .await
                    .flatten();
                if let Some(content) = content {
                    self.os.send(Command::Paste { content, plain: false, target: Some(target) });
                }
            }
            OsEvent::StackPasted => self.advance_stack().await,
            OsEvent::StackCancel => self.end_stack(),
            OsEvent::PasteBlocked => self.press_ctrl_v(),
            OsEvent::Status(OsStatus::HotkeyTaken) => {
                eprintln!("clipboard: the history hotkey is taken by another app");
                self.hotkey_taken = true;
            }
            OsEvent::Status(OsStatus::ListenerFailed) => eprintln!("clipboard: can't listen to the clipboard; nothing is being recorded"),
        }
    }

    async fn on_copy(&mut self, captured: Captured) {
        if self.paused() || captured.exe.as_deref().is_some_and(|exe| self.settings.ignores(exe)) {
            return;
        }
        let image = matches!(captured.content, Content::Image { .. });
        let now = now_ms();
        let saved = self
            .db
            .call(move |s| {
                // Here, not on the hub thread: a big text takes a moment to scan.
                if matches!(&captured.content, Content::Text { text, .. } if secrets::looks_secret(text)) {
                    eprintln!("clipboard: not saving a copy that looks like a secret");
                    return Ok(None);
                }
                s.add(captured, now)
            })
            .await
            .flatten();
        // A new copy of the user's own ends a paste stack (W-6), saved or not.
        if self.stack.is_some() {
            self.end_stack();
        }
        let Some(id) = saved else { return };
        if image {
            self.ocr_queue.retain(|&i| i != id);
            self.ocr_queue.push_front(id);
            self.kick_ocr();
        }
        if self.open {
            // The new card slides in (W-10).
            self.send_state().await;
        } else {
            self.toast().await;
        }
    }

    async fn on_action(&mut self, action: Action) {
        match action {
            Action::Open => {
                if !self.open {
                    // A click on the island: whatever was in front is the target, the
                    // window opens at the mouse.
                    self.open_window(Target::now()).await;
                }
            }
            Action::Pause => self.set_paused(true).await,
            Action::Resume => self.set_paused(false).await,
            Action::Clear => {
                self.finish_undo().await;
                self.db.call(|s| s.clear()).await;
                self.send_state().await;
            }
        }
    }

    async fn open_window(&mut self, target: Target) {
        self.settings = Settings::load();
        self.query = Query::default();
        self.target = Some(target);
        self.open = true;
        self.ctx.retract(KEY);
        self.hide_at = None;
        // Empty history still opens, with its one line. The clipboard lives in the notch:
        // the panel opens there and takes the keyboard while the user's app keeps focus.
        let Some((view, _images)) = self.view().await else { return };
        self.show_panel(view);
        self.ctx.focus(KEY);
        self.fetch_links();
        let pending = self.db.call(|s| s.ocr_pending(OCR_BATCH)).await.unwrap_or_default();
        for id in pending {
            if !self.ocr_queue.contains(&id) {
                self.ocr_queue.push_back(id);
            }
        }
        self.kick_ocr();
    }

    fn close_window(&mut self) {
        self.open = false;
        self.last_view = None;
        self.ctx.retract(KEY);
    }

    /// Publishes the notch panel for `view` (its images are already in the store).
    fn show_panel(&mut self, view: cv::View) {
        self.ctx.publish(KEY, Priority::Passive, panel::panel(&view, ACCENT));
        self.last_view = Some(view);
    }

    async fn on_link(&mut self, id: i64, meta: Option<LinkMeta>) {
        self.fetching.remove(&id);
        let now = now_ms();
        self.db.call(move |s| s.set_link(id, meta.as_ref(), now)).await;
        self.send_state().await;
    }

    async fn on_ocr(&mut self, id: i64, text: Option<String>) {
        self.ocr_busy = false;
        if let Some(text) = text {
            self.db.call(move |s| s.set_ocr(id, &text)).await;
            self.send_state().await;
        }
        self.kick_ocr();
    }

    /// Recognises the next queued image, one at a time, in the background.
    fn kick_ocr(&mut self) {
        if self.ocr_busy {
            return;
        }
        let Some(id) = self.ocr_queue.pop_front() else { return };
        self.ocr_busy = true;
        let (db, tx) = (self.db.clone(), self.ocr_tx.clone());
        tokio::spawn(async move {
            // Copied again since it was queued: it may have its text already.
            let png = db.call(move |s| Ok(if s.ocr_text(id)?.is_none() { s.png(id)? } else { None })).await.flatten();
            let text = match png {
                Some(png) => ocr::recognize(png).await,
                None => None,
            };
            _ = tx.send((id, text));
        });
    }

    async fn prune(&mut self) {
        let days = i64::from(self.settings.retention_days);
        if days == 0 {
            return;
        }
        let cutoff = now_ms() - days * 86_400_000;
        if self.db.call(move |s| s.prune_before(cutoff)).await.is_some_and(|n| n > 0) {
            self.send_state().await;
        }
    }

    async fn set_paused(&mut self, paused: bool) {
        self.paused_until = paused.then(|| Instant::now() + PAUSE);
        self.send_state().await;
    }

    fn paused(&self) -> bool {
        self.paused_until.is_some_and(|until| Instant::now() < until)
    }

    /// Makes a pending delete or edit final (only one can be undone at a time).
    async fn finish_undo(&mut self) {
        if self.undo.take().is_some() {
            self.db.call(|s| s.purge_deleted()).await;
        }
    }

    /// The window's view for the current query, and the images it refers to.
    async fn view(&mut self) -> Option<(cv::View, Vec<reui_protocol::ImageId>)> {
        let query = self.query.clone();
        let page = self.db.call(move |s| s.page(&query, ITEMS)).await?;
        let now = now_ms();
        self.shown_links = page
            .items
            .iter()
            .take(LINK_LOOKAHEAD)
            .filter(|s| needs_lookup(s, now))
            .filter_map(|s| Some((s.id, s.text.as_deref()?.trim().to_string())))
            .collect();
        let ui = Ui { query: self.query.clone(), layout: self.settings.layout.clone(), paused: self.paused(), undo: self.undo.is_some(), stack: self.stack_view() };
        let ctx = &self.ctx;
        let add = |bytes: Vec<u8>| ctx.image(bytes);
        let mut images = Images::new(&add);
        let view = cv::view(&page, &ui, now, &mut images);
        Some((view, images.ids))
    }

    /// New data for the open window; nothing when it's closed.
    async fn send_state(&mut self) {
        if !self.open {
            return;
        }
        let Some((view, _images)) = self.view().await else { return };
        self.show_panel(view);
        self.fetch_links();
    }

    fn fetch_links(&mut self) {
        let Some(client) = self.client.as_ref().filter(|_| self.settings.link_previews && self.open) else { return };
        for (id, url) in &self.shown_links {
            if self.fetching.insert(*id) {
                let (client, tx, id, url) = (client.clone(), self.links_tx.clone(), *id, url.clone());
                tokio::spawn(async move { _ = tx.send((id, link::fetch(&client, &url).await)) });
            }
        }
    }
}

fn now_ms() -> i64 {
    reui_protocol::unix_ms() as i64
}

/// A link that was never looked up, or whose lookup found nothing a day or more ago.
fn needs_lookup(saved: &Saved, now_ms: i64) -> bool {
    saved.category == Kind::Links.name()
        && match saved.link_fetched {
            None => true,
            Some(at) => saved.link == LinkMeta::default() && now_ms - at >= LINK_RETRY_MS,
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn island_actions_parse() {
        assert_eq!(Action::parse("open"), Some(Action::Open));
        assert_eq!(Action::parse(ACTION_EXPANDED), Some(Action::Open));
        assert_eq!((Action::parse("pause"), Action::parse("resume"), Action::parse("clear")), (Some(Action::Pause), Some(Action::Resume), Some(Action::Clear)));
        assert_eq!(Action::parse("paste:1"), None);
    }

    fn saved(category: &str) -> Saved {
        Saved { id: 7, category: category.into(), created_ms: 0, ..Default::default() }
    }

    #[test]
    fn links_are_looked_up_once_and_failures_daily() {
        let link = Saved { text: Some("https://example.com".into()), ..saved("links") };
        assert!(needs_lookup(&link, 1_000));
        let failed = Saved { link_fetched: Some(1_000), ..link.clone() };
        assert!(!needs_lookup(&failed, 1_000 + LINK_RETRY_MS - 1));
        assert!(needs_lookup(&failed, 1_000 + LINK_RETRY_MS));
        let found = Saved { link: LinkMeta { title: Some("Example".into()), ..Default::default() }, ..failed };
        assert!(!needs_lookup(&found, i64::MAX));
        assert!(!needs_lookup(&saved("text"), 0));
    }
}
