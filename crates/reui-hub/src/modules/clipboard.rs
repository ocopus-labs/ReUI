//! Clipboard history: every copy is saved (text with its rich formats, images, files) and
//! confirmed with a toast (the pill shows what was copied). The history itself lives in the
//! clipboard window (`docs/clipboard-window-contract.md`): the hotkey (Win+Alt+V by default)
//! or a click on the toast opens it next to the text caret, searching; picking an item
//! pastes it back into the field the user was typing in. Ctrl+Alt+1…9 paste a recent item
//! without opening anything.
//!
//! History lives in `%APPDATA%\ReUI\clipboard.db` (see [`super::clipboard_store`]) and
//! never leaves the machine. Never recorded: content other apps mark private (checked by
//! the OS side), copies from ignored apps (password managers), anything that looks like a
//! secret ([`super::secrets`]), and everything while paused. Settings are in
//! [`super::clipboard_settings`].
//!
//! Copied links get the page's title, site and image (see [`super::link`]) once they are
//! among the first cards of an open window, unless `--no-link-previews` is passed or the
//! setting is off. Images get their text recognised ([`super::ocr`]) right after capture,
//! and older ones a few at a time while the window is open.
//!
//! Island actions on the toast: `open` (or `expanded`) opens the window; tray commands:
//! `open`, `pause`, `resume`, `clear`.

use std::collections::{HashSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use reqwest::Client;
use reui_protocol::{Art, CompactView, Element, Preview, Priority, Rgb, Tone, ACTION_COLLAPSED, ACTION_EXPANDED};
use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;
use tokio::time::{sleep_until, Instant, MissedTickBehavior};

use super::clipboard_panel;
use super::clipboard_os::{self, Captured, Clipboard, Command, Content, Hotkey, OsEvent, OsStatus, Target};
use super::clipboard_settings::Settings;
use super::clipboard_store::{Db, Kind, Query, Saved, Scope, Store};
use super::clipboard_view::{self as cv, Images, Stack, Ui};
use super::link::{self, LinkMeta};
use super::transform::Mode;
use super::{ocr, secrets, Module};
use crate::hub::{ModuleContext, ModuleInput, View};

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

const GLYPH_TEXT: char = '\u{E8D2}';
const GLYPH_LINK: char = '\u{E71B}';
const GLYPH_FILE: char = '\u{E8A5}';
const GLYPH_COLOR: char = '\u{E790}';
const GLYPH_COPY: char = '\u{E8C8}';
const GLYPH_PASTE: char = '\u{E77F}';

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

/// Items pasting one per Ctrl+V; `at` is the one on the clipboard now.
struct PasteStack {
    items: Vec<Content>,
    at: usize,
    idle_until: Instant,
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
    let Some(os) = clipboard_os::start_with(events_tx, hotkey) else { return };
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
                    } else if let Some(message) = app.last_view.as_ref().and_then(|v| clipboard_panel::to_message(&action.action, v)) {
                        app.on_window(&message).await;
                        // A paste or copy closes the panel.
                        if !app.open {
                            app.close_window();
                        }
                    }
                }
                Some(ModuleInput::Window(message)) => app.on_window(&message).await,
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
        self.ctx.publish(KEY, Priority::Passive, clipboard_panel::panel(&view, ACCENT));
        self.last_view = Some(view);
    }

    async fn on_window(&mut self, message: &str) {
        let Ok(m) = serde_json::from_str::<Value>(message) else {
            eprintln!("clipboard: window sent something that isn't JSON");
            return;
        };
        let now = now_ms();
        match m["type"].as_str().unwrap_or_default() {
            "opened" => {
                self.open = true;
                self.fetch_links();
            }
            "closed" => self.open = false,
            "query" => {
                self.query = Query {
                    text: m["query"].as_str().unwrap_or_default().to_string(),
                    scope: m["collection"].as_str().and_then(Scope::parse).unwrap_or_default(),
                    kind: m["filter"]["type"].as_str().and_then(Kind::parse),
                    app: m["filter"]["app"].as_str().filter(|a| !a.is_empty()).map(String::from),
                };
                self.send_state().await;
            }
            "layout" => {
                if let Some(layout) = m["layout"].as_str().filter(|l| matches!(*l, "strip" | "grid")) {
                    self.settings.layout = layout.to_string();
                    Settings::save_layout(&Settings::path(), layout);
                }
                self.send_state().await;
            }
            kind @ ("paste" | "copy") => {
                // The host hid the window (and, for a paste, gave the target its focus back).
                self.open = false;
                let mode = Mode::parse(m["mode"].as_str().unwrap_or_default());
                let Some((content, plain)) = self.compose(ids(&m), mode).await else { return };
                if kind == "copy" {
                    self.os.send(Command::Copy(content));
                } else if m["focused"].as_bool() == Some(false) {
                    self.os.send(Command::Copy(content));
                    self.press_ctrl_v();
                } else {
                    self.os.send(Command::Paste { content, plain, target: self.target.map(|t| t.hwnd) });
                }
            }
            "stack" => {
                self.open = false;
                self.start_stack(ids(&m)).await;
            }
            kind @ ("pin" | "unpin") => {
                if let Some(id) = m["id"].as_i64() {
                    let pinned = kind == "pin";
                    self.db.call(move |s| s.set_pinned(id, pinned)).await;
                }
                self.send_state().await;
            }
            "delete" => {
                let ids = ids(&m);
                self.finish_undo().await;
                let deleted = self.db.call(move |s| s.delete(&ids, now)).await.unwrap_or_default();
                if !deleted.is_empty() {
                    self.undo = Some((Undo::Deleted(deleted), Instant::now() + UNDO));
                }
                self.send_state().await;
            }
            "undo" => {
                if let Some((undo, _)) = self.undo.take() {
                    self.db
                        .call(move |s| match undo {
                            Undo::Deleted(ids) => s.restore(&ids),
                            Undo::Edited { old, new, fresh } => {
                                s.restore(&[old])?;
                                if fresh {
                                    s.remove(new)?;
                                }
                                Ok(())
                            }
                        })
                        .await;
                }
                self.send_state().await;
            }
            kind @ ("collect" | "uncollect") => {
                let (ids, add) = (ids(&m), kind == "collect");
                match m["collection"].as_str().and_then(Scope::parse) {
                    Some(Scope::Collection(c)) => _ = self.db.call(move |s| s.collect(&ids, c, add)).await,
                    Some(Scope::Pinned) => _ = self.db.call(move |s| ids.iter().try_for_each(|&id| s.set_pinned(id, add))).await,
                    _ => {}
                }
                self.send_state().await;
            }
            "collection.create" => {
                if let Some(name) = name(&m["name"]) {
                    let color = m["color"].as_str().map(String::from);
                    self.db.call(move |s| s.create_collection(&name, color.as_deref())).await;
                }
                self.send_state().await;
            }
            "collection.rename" => {
                if let (Some(id), Some(name)) = (collection_id(&m["id"]), name(&m["name"])) {
                    self.db.call(move |s| s.rename_collection(id, &name)).await;
                }
                self.send_state().await;
            }
            "collection.color" => {
                if let Some(id) = collection_id(&m["id"]) {
                    let color = m["color"].as_str().map(String::from);
                    self.db.call(move |s| s.color_collection(id, color.as_deref())).await;
                }
                self.send_state().await;
            }
            "collection.delete" => {
                if let Some(id) = collection_id(&m["id"]) {
                    self.db.call(move |s| s.delete_collection(id)).await;
                    if self.query.scope == Scope::Collection(id) {
                        self.query.scope = Scope::History;
                    }
                }
                self.send_state().await;
            }
            "collection.reorder" => {
                let order: Vec<i64> = m["ids"].as_array().map(|a| a.iter().filter_map(collection_id).collect()).unwrap_or_default();
                self.db.call(move |s| s.reorder_collections(&order)).await;
                self.send_state().await;
            }
            "edit" => {
                if let (Some(id), Some(text)) = (m["id"].as_i64(), m["text"].as_str().map(String::from)) {
                    self.finish_undo().await;
                    if let Some((new, fresh)) = self.db.call(move |s| s.edit(id, &text, now)).await.flatten() {
                        if new != id {
                            self.undo = Some((Undo::Edited { old: id, new, fresh }, Instant::now() + UNDO));
                        }
                    }
                }
                self.send_state().await;
            }
            "preview" => {
                let Some(id) = m["id"].as_i64() else { return };
                let content = self.db.call(move |s| s.content(id)).await.flatten();
                let message = match content {
                    Some(Content::Text { text, .. }) => cv::preview_message(id, Some(cv::cap_bytes(&text, cv::PREVIEW_BYTES)), None),
                    Some(Content::Files { paths, .. }) => cv::preview_message(id, Some(&paths.join("\n")), None),
                    Some(Content::Image { png, .. }) => {
                        let image = self.ctx.image(png);
                        self.ctx.window(cv::preview_message(id, None, Some(image.0.to_string())), vec![image]);
                        return;
                    }
                    None => cv::preview_message(id, None, None),
                };
                self.ctx.window(message, Vec::new());
            }
            "ocr" => {
                let Some(id) = m["id"].as_i64() else { return };
                let mut text = self.db.call(move |s| s.ocr_text(id)).await.flatten();
                if text.is_none() {
                    if let Some(png) = self.db.call(move |s| s.png(id)).await.flatten() {
                        text = ocr::recognize(png).await;
                        if let Some(found) = text.clone() {
                            self.db.call(move |s| s.set_ocr(id, &found)).await;
                        }
                    }
                }
                if let Some(text) = text.filter(|t| !t.trim().is_empty()) {
                    self.os.send(Command::Copy(Content::Text { text, html: None, rtf: None }));
                }
                self.send_state().await;
            }
            "pause" => self.set_paused(true).await,
            "resume" => self.set_paused(false).await,
            "export" => {
                let Some(id) = m["id"].as_i64() else { return };
                let content = self.db.call(move |s| s.content(id)).await.flatten();
                let (message, images) = match content {
                    Some(Content::Text { text, .. }) => (cv::export_message(id, Some(&text), None, None), Vec::new()),
                    Some(Content::Image { png, .. }) => {
                        let image = self.ctx.image(png);
                        (cv::export_message(id, None, Some(image.0.to_string()), None), vec![image])
                    }
                    Some(Content::Files { paths, .. }) => (cv::export_message(id, None, None, Some(&paths)), Vec::new()),
                    None => (cv::export_message(id, None, None, None), Vec::new()),
                };
                self.ctx.window(message, images);
            }
            other => eprintln!("clipboard: unknown window message \"{other}\""),
        }
    }

    /// What pasting `ids` puts on the clipboard (see [`compose`]); the items move to the top.
    async fn compose(&self, ids: Vec<i64>, mode: Mode) -> Option<(Content, bool)> {
        let now = now_ms();
        let contents = self
            .db
            .call(move |s| {
                let mut contents = Vec::new();
                for &id in &ids {
                    if let Some(content) = s.content(id)? {
                        s.touch(id, now)?;
                        contents.push(content);
                    }
                }
                Ok(contents)
            })
            .await?;
        compose(contents, mode)
    }

    async fn start_stack(&mut self, ids: Vec<i64>) {
        self.end_stack();
        let items = self
            .db
            .call(move |s| {
                let mut items = Vec::new();
                for id in ids {
                    items.extend(s.content(id)?);
                }
                Ok(items)
            })
            .await
            .unwrap_or_default();
        let Some(first) = items.first().cloned() else { return };
        self.os.send(Command::Copy(first));
        self.os.watch_pastes(true);
        self.stack = Some(PasteStack { items, at: 0, idle_until: Instant::now() + STACK_IDLE });
        self.show_stack();
        self.send_state().await;
    }

    /// The user pasted the current item: the next one goes on the clipboard.
    async fn advance_stack(&mut self) {
        let Some(stack) = self.stack.as_mut() else { return };
        stack.at += 1;
        stack.idle_until = Instant::now() + STACK_IDLE;
        match stack.items.get(stack.at).cloned() {
            Some(next) => {
                self.os.send(Command::Copy(next));
                self.show_stack();
            }
            None => self.end_stack(),
        }
        self.send_state().await;
    }

    fn end_stack(&mut self) {
        if self.stack.take().is_some() {
            self.os.watch_pastes(false);
            self.ctx.retract(STACK_KEY);
        }
    }

    /// "Stack 2/5" in the island while a stack is running.
    fn show_stack(&self) {
        let Some(stack) = &self.stack else { return };
        let leading = Element::Art(Art::Glyph(GLYPH_PASTE));
        let text = format!("Stack {}/{}", stack.at + 1, stack.items.len());
        let view = View {
            accent: Some(ACCENT),
            audio: None,
            compact: CompactView { leading: leading.clone(), trailing: Element::Text { text, tone: Tone::Secondary } },
            minimal: leading,
            expanded: None,
        };
        self.ctx.publish(STACK_KEY, Priority::Ongoing, view);
    }

    fn stack_view(&self) -> Stack {
        match &self.stack {
            Some(s) => Stack { active: true, remaining: (s.items.len() - s.at.min(s.items.len())) as u32 },
            None => Stack::default(),
        }
    }

    /// The item is on the clipboard; the target wouldn't take our keystrokes (elevated,
    /// gone, or focus couldn't be given back), so the user pastes it (E-P1, W-2, W-5).
    fn press_ctrl_v(&mut self) {
        let leading = Element::Art(Art::Glyph(GLYPH_PASTE));
        let view = View {
            accent: Some(ACCENT),
            audio: None,
            compact: CompactView { leading: leading.clone(), trailing: Element::Text { text: "Press Ctrl+V".into(), tone: Tone::Secondary } },
            minimal: leading,
            expanded: None,
        };
        self.ctx.publish(KEY, Priority::Alert, view);
        self.hide_at = Some(Instant::now() + BLOCKED_TOAST);
    }

    /// The copy confirmation: the newest item's thumbnail or glyph and a glimpse of it.
    async fn toast(&mut self) {
        let Some(Some(latest)) = self.db.call(|s| s.latest()).await else { return };
        self.ctx.publish(KEY, Priority::Brief, toast_view(&self.ctx, &latest));
        self.hide_at = Some(Instant::now() + TOAST);
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

/// What pasting several items means: one item as it is (or as plain text in a `mode`);
/// several are their text joined by newlines, transformed per item; with no text among
/// them, the first one alone. `bool`: paste as plain text.
fn compose(contents: Vec<Content>, mode: Mode) -> Option<(Content, bool)> {
    let plain = |text: String| Content::Text { text, html: None, rtf: None };
    if contents.len() == 1 {
        let content = contents.into_iter().next()?;
        return Some(match content {
            Content::Text { text, html, .. } if mode != Mode::Normal => (plain(mode.apply(&text, html.as_deref())), true),
            content => (content, false),
        });
    }
    let texts: Vec<String> = contents
        .iter()
        .filter_map(|c| match c {
            Content::Text { text, html, .. } => Some(mode.apply(text, html.as_deref())),
            _ => None,
        })
        .collect();
    if texts.is_empty() {
        return contents.into_iter().next().map(|c| (c, false));
    }
    Some((plain(texts.join("\n")), true))
}

/// Item ids in a window message (`"ids": [12, 13]`).
fn ids(message: &Value) -> Vec<i64> {
    message["ids"].as_array().map(|a| a.iter().filter_map(Value::as_i64).collect()).unwrap_or_default()
}

/// A user collection's id: `"c7"` (as in views) or `7`.
fn collection_id(value: &Value) -> Option<i64> {
    match value {
        Value::String(s) => match Scope::parse(s)? {
            Scope::Collection(id) => Some(id),
            _ => None,
        },
        _ => value.as_i64(),
    }
}

/// A collection name: trimmed, not empty, at most 64 characters.
fn name(value: &Value) -> Option<String> {
    let name = value.as_str()?.trim();
    (!name.is_empty()).then(|| prefix(name, 64).to_string())
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

/// The toast: no panel, just the thing itself (thumbnail or glyph) and a glimpse of it. In
/// the bubble beside something else playing, the copy icon says what happened.
fn toast_view(ctx: &ModuleContext, latest: &Saved) -> View {
    let leading = match &latest.thumb {
        Some(thumb) if latest.category == Kind::Images.name() => Element::Art(Art::Image(ctx.image(thumb.clone()))),
        _ => Element::Art(Art::Glyph(glyph_for(latest))),
    };
    View {
        accent: Some(ACCENT),
        audio: None,
        compact: CompactView { leading, trailing: glimpse(latest).map_or(Element::Empty, |text| Element::Text { text, tone: Tone::Secondary }) },
        minimal: Element::Art(Art::Glyph(GLYPH_COPY)),
        expanded: None,
    }
}

fn glyph_for(saved: &Saved) -> char {
    match saved.category.as_str() {
        "links" => GLYPH_LINK,
        "files" => GLYPH_FILE,
        "colors" => GLYPH_COLOR,
        _ => GLYPH_TEXT,
    }
}

/// A few words of what was copied, for the toast: the start of the text, an image's size,
/// a file's name, a link's site.
fn glimpse(saved: &Saved) -> Option<String> {
    const CHARS: usize = 24;
    let text = match saved.category.as_str() {
        "images" => return saved.size.map(|(w, h)| format!("{w} × {h}")),
        "files" => {
            let name = saved.files.first().map(|f| f.rsplit(['\\', '/']).next().unwrap_or(f))?;
            return Some(match saved.files.len() {
                1 => name.to_string(),
                n => format!("{name} +{}", n - 1),
            });
        }
        "links" => return saved.link.site.clone().or_else(|| saved.text.as_deref().map(|t| host_of(t).to_string())),
        _ => saved.text.as_deref()?.trim().lines().next()?.trim(),
    };
    let cut = prefix(text, CHARS);
    Some(if cut.len() < text.len() { format!("{}…", cut.trim_end()) } else { cut.to_string() })
}

/// `example.com` from `https://www.example.com/path`.
fn host_of(url: &str) -> &str {
    let rest = url.trim().split_once("://").map_or(url, |(_, r)| r);
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    host.strip_prefix("www.").unwrap_or(host)
}

/// The first `chars` characters.
pub(super) fn prefix(text: &str, chars: usize) -> &str {
    text.char_indices().nth(chars).map_or(text, |(i, _)| &text[..i])
}

/// What copied text is, for its card: a colour, a link, code, or plain text. Only the
/// start of a long text is looked at.
pub(super) fn classify(text: &str) -> Preview {
    let text = prefix(text, 16 * 1024);
    let trimmed = text.trim();
    if let Some(rgb) = parse_color(trimmed) {
        return Preview::Color(rgb);
    }
    if !trimmed.contains(char::is_whitespace) {
        if let Some(rest) = trimmed.strip_prefix("https://").or_else(|| trimmed.strip_prefix("http://")) {
            let (host, path) = rest.split_once('/').map_or((rest, ""), |(h, p)| (h, p));
            // Never show `user:password@`.
            let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
            return Preview::Link { host: host.trim_start_matches("www.").to_string(), path: format!("/{path}"), title: None, site: None, image: None };
        }
    }
    let lines: Vec<&str> = text.lines().collect();
    let code_marks = lines.iter().filter(|l| l.trim_end().ends_with([';', '{', '}', ')']) || l.starts_with("    ") || l.starts_with('\t')).count();
    if lines.len() >= 2 && code_marks * 2 >= lines.len() {
        return Preview::Code(text.to_string());
    }
    Preview::Text(trimmed.to_string())
}

/// `#RGB`, `#RRGGBB`, `rgb(r, g, b)`.
fn parse_color(text: &str) -> Option<Rgb> {
    if let Some(hex) = text.strip_prefix('#') {
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let hex: Vec<u8> = match hex.len() {
            3 => hex.chars().map(|c| c.to_digit(16).map(|d| (d * 17) as u8)).collect::<Option<_>>()?,
            6 => (0..3).map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()).collect::<Option<_>>()?,
            _ => return None,
        };
        return Some(Rgb(hex[0], hex[1], hex[2]));
    }
    let inner = text.strip_prefix("rgb(")?.strip_suffix(')')?;
    let parts: Vec<u8> = inner.split(',').map(|p| p.trim().parse().ok()).collect::<Option<_>>()?;
    (parts.len() == 3).then(|| Rgb(parts[0], parts[1], parts[2]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::Hub;

    #[test]
    fn copied_text_is_recognised() {
        assert_eq!(classify("#0A84FF"), Preview::Color(Rgb(10, 132, 255)));
        assert_eq!(classify("#fff"), Preview::Color(Rgb(255, 255, 255)));
        assert_eq!(classify("rgb(48, 209, 88)"), Preview::Color(Rgb(48, 209, 88)));
        assert_eq!(classify("https://www.figma.com/design/abc"), Preview::Link { host: "figma.com".into(), path: "/design/abc".into(), title: None, site: None, image: None });
        assert!(matches!(classify("fn main() {\n    println!(\"hi\");\n}"), Preview::Code(_)));
        assert_eq!(classify("  Minneapolis 55410  "), Preview::Text("Minneapolis 55410".into()));
        assert_eq!(classify("see https://example.com later"), Preview::Text("see https://example.com later".into()));
        assert_eq!(classify("#ab€c"), Preview::Text("#ab€c".into()));
        assert_eq!(classify("#€€"), Preview::Text("#€€".into()));
        assert!(matches!(classify("https://me:hunter2@example.com/x"), Preview::Link { host, .. } if host == "example.com"));
    }

    #[test]
    fn previews_are_cut_on_character_boundaries() {
        assert_eq!(prefix("héllo", 2), "hé");
        assert_eq!(prefix("hi", 10), "hi");
        assert_eq!(prefix(&"€".repeat(5000), 2000).chars().count(), 2000);
    }

    #[test]
    fn island_actions_parse() {
        assert_eq!(Action::parse("open"), Some(Action::Open));
        assert_eq!(Action::parse(ACTION_EXPANDED), Some(Action::Open));
        assert_eq!((Action::parse("pause"), Action::parse("resume"), Action::parse("clear")), (Some(Action::Pause), Some(Action::Resume), Some(Action::Clear)));
        assert_eq!(Action::parse("paste:1"), None);
    }

    fn text(t: &str, html: Option<&str>) -> Content {
        Content::Text { text: t.into(), html: html.map(String::from), rtf: None }
    }

    #[test]
    fn pasting_one_or_several() {
        let rich = text("Hello", Some("<!--StartFragment--><b>Hello</b><!--EndFragment-->"));
        // One item, as is: rich formats kept.
        assert_eq!(compose(vec![rich.clone()], Mode::Normal), Some((rich.clone(), false)));
        // In a mode: plain text, transformed.
        assert_eq!(compose(vec![rich.clone()], Mode::Upper), Some((text("HELLO", None), true)));
        assert_eq!(compose(vec![rich.clone()], Mode::Markdown), Some((text("**Hello**", None), true)));
        // Several: joined by newlines; non-text items left out.
        let image = Content::Image { png: vec![1], thumb: vec![1] };
        let files = Content::Files { paths: vec!["a".into()], cut: false };
        assert_eq!(compose(vec![text("a ", None), image.clone(), text(" b", None)], Mode::Trim), Some((text("a\nb", None), true)));
        assert_eq!(compose(vec![text("a", None), text("b", None)], Mode::Normal), Some((text("a\nb", None), true)));
        // No text among them: the first item alone, as is.
        assert_eq!(compose(vec![image.clone(), files.clone()], Mode::Normal), Some((image.clone(), false)));
        assert_eq!(compose(vec![files.clone()], Mode::Upper), Some((files, false)));
        assert_eq!(compose(Vec::new(), Mode::Normal), None);
    }

    #[test]
    fn window_message_fields() {
        let m: Value = serde_json::from_str(r#"{"type":"collect","ids":[3,"x",4],"collection":"c7","id":"c9","name":"  Prompts  "}"#).unwrap();
        assert_eq!(ids(&m), [3, 4]);
        assert_eq!(collection_id(&m["collection"]), Some(7));
        assert_eq!(collection_id(&m["id"]), Some(9));
        assert_eq!(collection_id(&Value::from(5)), Some(5));
        assert_eq!(collection_id(&Value::from("pinned")), None);
        assert_eq!(name(&m["name"]).as_deref(), Some("Prompts"));
        assert_eq!(name(&Value::from("   ")), None);
        assert_eq!(name(&Value::Null), None);
    }

    fn saved(category: &str) -> Saved {
        Saved { id: 7, category: category.into(), created_ms: 0, ..Default::default() }
    }

    #[test]
    fn toasts_show_the_thing_and_never_a_panel() {
        let hub = Hub::new();
        let ctx = hub.context("clipboard");
        let view = toast_view(&ctx, &Saved { text: Some("Hello there, this is a long first line\nsecond".into()), ..saved("text") });
        assert!(view.expanded.is_none());
        assert_eq!(view.compact.trailing, Element::Text { text: "Hello there, this is a l…".into(), tone: Tone::Secondary });
        assert_eq!(view.minimal, Element::Art(Art::Glyph(GLYPH_COPY)));
        let image = toast_view(&ctx, &Saved { thumb: Some(vec![1, 2]), size: Some((640, 400)), ..saved("images") });
        assert!(matches!(image.compact.leading, Element::Art(Art::Image(_))));
        assert_eq!(glimpse(&Saved { size: Some((640, 400)), ..saved("images") }).as_deref(), Some("640 × 400"));
        assert_eq!(glimpse(&Saved { files: vec![r"C:\a\b.txt".into(), "c".into()], ..saved("files") }).as_deref(), Some("b.txt +1"));
        assert_eq!(glimpse(&Saved { text: Some("https://www.figma.com/x".into()), ..saved("links") }).as_deref(), Some("figma.com"));
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
