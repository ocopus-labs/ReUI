//! The file shelf (docs/PRD-file-shelf.md): things dropped on the island are held here
//! (files by reference, never copied or moved; content with no file saved in ReUI's
//! folder) until the user drags them out into another app, opens them or removes them.
//! While something is held, the pill shows the shelf's mark and how many; the shelf is a
//! place (`destinations.rs`) and the one that takes drops.
//!
//! The island reads a drop and sends it as `ModuleInput::Drop`; it also does the dragging
//! out, from what each card says it carries. Island actions on a card: `open:<id>`,
//! `show:<id>`, `copy:<id>`, `remove:<id>`, and `clear`. Commands (tray, view switcher,
//! Settings): `open`, `clear`, `reload`.
//!
//! Files: `store.rs` (what's held, `held.json`, expiry and the cap), `intake.rs` (what a
//! drop becomes), `card.rs` (what the island shows), `os.rs` (the shell, the clipboard, the
//! disk) and `settings.rs` (`shelf.json`).

mod card;
mod intake;
mod os;
mod settings;
mod store;

use std::collections::HashSet;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use reui_protocol::{Dropped, Priority, ACTION_COLLAPSED, ACTION_EXPANDED};

use self::card::Facts;
use self::os::Looks;
use self::settings::{Keep, Settings};
use self::store::{Held, Shelf, What};
use super::Module;
use crate::hub::{ModuleAction, ModuleContext, ModuleInput};

const KEY: &str = "shelf";
/// A card's Open opens at most this many of its files, so a drop of a whole folder's
/// contents can't start a hundred apps.
const OPEN_AT_MOST: usize = 10;

pub struct FileShelf;

impl Module for FileShelf {
    fn name(&self) -> &'static str {
        "shelf"
    }

    fn run(self: Box<Self>, ctx: ModuleContext) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(run(ctx))
    }
}

struct State {
    ctx: ModuleContext,
    settings: Settings,
    shelf: Shelf,
    root: PathBuf,
    /// The card was opened as a place (empty or not); it shows until it closes.
    opened: bool,
    looks: Arc<Mutex<Looks>>,
}

fn now_ms() -> i64 {
    reui_protocol::unix_ms() as i64
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    tokio::task::spawn_blocking(f).await.ok()
}

async fn run(ctx: ModuleContext) {
    let root = store::folder();
    let (settings, shelf) = {
        let root = root.clone();
        blocking(move || {
            let shelf = store::load(&root);
            store::sweep(&root, &shelf);
            (Settings::load(), shelf)
        })
        .await
        .unwrap_or_default()
    };
    let mut state = State { ctx, settings, shelf, root, opened: false, looks: Arc::default() };
    if state.settings.keep == Keep::Restart {
        let gone = state.shelf.clear();
        state.forget(gone).await;
    }
    state.tidy().await;
    state.publish().await;

    loop {
        match state.ctx.next().await {
            Some(ModuleInput::Drop(items)) => state.take(items).await,
            Some(ModuleInput::Action(action)) => state.act(action).await,
            Some(_) => {}
            None => break,
        }
    }
}

impl State {
    /// Expiry and the cap, written down if anything went (PRD §5.3).
    async fn tidy(&mut self) {
        let mut gone = self.shelf.expire(self.settings.keep, now_ms());
        gone.extend(self.shelf.over_cap(self.settings.saved_cap_bytes()));
        if !gone.is_empty() {
            self.forget(gone).await;
        }
    }

    /// Deletes what ReUI owned of removed cards and saves the list.
    async fn forget(&mut self, gone: Vec<Held>) {
        let (root, shelf) = (self.root.clone(), self.shelf.clone());
        blocking(move || {
            store::forget(&root, &gone);
            store::save(&root, &shelf);
        })
        .await;
    }

    /// The pill and the card as things are now; nothing held and not open: nothing shown.
    async fn publish(&mut self) {
        if self.shelf.items.is_empty() && !self.opened {
            self.ctx.retract(KEY);
            return;
        }
        let (looks, items) = (self.looks.clone(), self.shelf.items.clone());
        let seen = blocking(move || looks.lock().map(|mut l| l.look(&items)).unwrap_or_default()).await.unwrap_or_default();
        let facts = Facts {
            missing: seen.missing.into_iter().collect::<HashSet<_>>(),
            thumbs: seen.thumbs.into_iter().map(|(id, png)| (id, self.ctx.image(png))).collect(),
            icons: seen.icons.into_iter().map(|(id, png)| (id, self.ctx.image(png))).collect(),
        };
        self.ctx.publish(KEY, Priority::Passive, card::view(&self.shelf.items, &facts, now_ms()));
    }

    /// A drop on the island: its cards, written where they need to be, then held.
    async fn take(&mut self, items: Vec<Dropped>) {
        let plans = intake::plan(items, &intake::temp_roots());
        if plans.is_empty() {
            return;
        }
        let jobs: Vec<(u64, intake::Plan)> = plans.into_iter().map(|p| (self.shelf.reserve(), p)).collect();
        let root = self.root.clone();
        let made = blocking(move || jobs.into_iter().filter_map(|(id, plan)| intake::carry_out(plan, id, &root).map(|what| (id, what))).collect::<Vec<_>>()).await.unwrap_or_default();
        let now = now_ms();
        for (id, what) in made {
            self.shelf.insert(Held { id, added_ms: now, what });
        }
        let gone = self.shelf.over_cap(self.settings.saved_cap_bytes());
        self.forget(gone).await;
        self.tidy().await;
        self.publish().await;
    }

    async fn act(&mut self, action: ModuleAction) {
        match (action.key.as_str(), action.action.as_str()) {
            // Commands: the tray, a view switcher tab, Settings.
            ("", "open") => {
                self.opened = true;
                self.tidy().await;
                self.publish().await;
                self.ctx.focus(KEY);
            }
            ("", "reload") => {
                self.settings = blocking(Settings::load).await.unwrap_or_default();
                self.tidy().await;
                self.publish().await;
            }
            (_, "clear") => {
                let gone = self.shelf.clear();
                self.forget(gone).await;
                self.publish().await;
            }
            (KEY, ACTION_EXPANDED) => {
                // Whether files are still there is asked when the card opens.
                self.opened = true;
                self.publish().await;
            }
            (KEY, ACTION_COLLAPSED) => {
                self.opened = false;
                if self.shelf.items.is_empty() {
                    self.ctx.retract(KEY);
                }
            }
            (KEY, other) => {
                if let Some((verb, id)) = card::parse(other) {
                    self.on_card(verb, id).await;
                }
            }
            _ => {}
        }
    }

    async fn on_card(&mut self, verb: &str, id: u64) {
        if verb == "remove" {
            if let Some(held) = self.shelf.remove(id) {
                self.forget(vec![held]).await;
                self.publish().await;
            }
            return;
        }
        let Some(held) = self.shelf.get(id).cloned() else { return };
        let verb = verb.to_string();
        blocking(move || match (verb.as_str(), &held.what) {
            ("open", What::Files { paths, .. }) => paths.iter().filter(|p| std::fs::metadata(p).is_ok()).take(OPEN_AT_MOST).for_each(|p| os::open(p)),
            ("open", What::Link { url }) => os::open(url),
            ("open" | "copy", What::Text { text }) => os::copy_text(text),
            ("show", What::Files { paths, .. }) => os::show_in_folder(paths),
            ("copy", What::Files { paths, .. }) => os::copy_text(&paths.join("\r\n")),
            ("copy", What::Link { url }) => os::copy_text(url),
            _ => {}
        })
        .await;
    }
}
