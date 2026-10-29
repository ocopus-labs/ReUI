//! Downloads (docs/PRD-downloads-screenshots.md): a browser's download in progress as an
//! Ongoing activity, then a Brief "Downloaded" card with Open and Show in folder.
//!
//! No browser tells Windows it's downloading, so this watches the Downloads folder (and
//! the extra ones in `downloads.json`) for partial files (`partial.rs`) with the shared
//! folder watch: a blocked thread, nothing at all between downloads. Sizes are read off
//! the hub thread, at most once a second and only after the browser wrote something
//! (`tracker.rs`). Only names and sizes are ever read, never what's in a file.
//!
//! Island actions: `prev` / `next` (the pager), `show:<id>`, `open:<id>`, and the card
//! opening or closing (a Downloaded card stays while it's open). From the Settings window:
//! `reload` (the folder list).
//!
//! Files: `partial.rs` (which files are downloads), `tracker.rs` (what they're doing),
//! `view.rs`, `settings.rs`.

mod partial;
mod settings;
mod tracker;
mod view;

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use reui_protocol::{ImageId, Priority, ACTION_COLLAPSED, ACTION_EXPANDED};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::time::{sleep_until, Duration, Instant};

use self::settings::Settings;
use self::tracker::Tracker;
use super::folders::{shell, Change, Watch};
use super::Module;
use crate::hub::{ModuleContext, ModuleInput};

const ACTIVE: &str = "active";
const DONE: &str = "done";
/// A partial file found by a rescan counts when it was written to within this long.
const RESCAN_RECENT: Duration = Duration::from_secs(10);

pub struct Downloads;

impl Module for Downloads {
    fn name(&self) -> &'static str {
        "downloads"
    }

    fn run(self: Box<Self>, ctx: ModuleContext) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(Run::new(ctx).run())
    }
}

/// What comes back from the blocking pool.
enum Done {
    Folders(Vec<PathBuf>),
    Sizes(Vec<(PathBuf, Option<u64>)>),
    Icon(u64, Option<Vec<u8>>),
}

struct Run {
    ctx: ModuleContext,
    tracker: Tracker,
    /// The tracker's clock starts here.
    epoch: Instant,
    watch: Option<Watch>,
    folders: Vec<PathBuf>,
    changes: (UnboundedSender<Change>, UnboundedReceiver<Change>),
    done: (UnboundedSender<Done>, UnboundedReceiver<Done>),
    /// The pagers' places.
    active_at: usize,
    done_at: usize,
    icons: HashMap<u64, ImageId>,
    asked_icons: HashSet<u64>,
}

impl Run {
    fn new(ctx: ModuleContext) -> Self {
        Self {
            ctx,
            tracker: Tracker::default(),
            epoch: Instant::now(),
            watch: None,
            folders: Vec::new(),
            changes: unbounded_channel(),
            done: unbounded_channel(),
            active_at: 0,
            done_at: 0,
            icons: HashMap::new(),
            asked_icons: HashSet::new(),
        }
    }

    fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }

    async fn run(mut self) {
        self.find_folders();
        loop {
            let now = self.now_ms();
            let wake = self.tracker.wake_at(now).map(|t| self.epoch + Duration::from_millis(t));
            tokio::select! {
                Some(change) = self.changes.1.recv() => {
                    if let Change::Overflow(folder) = &change {
                        self.rescan(folder.clone());
                    }
                    self.tracker.on_change(&change, self.now_ms());
                }
                Some(done) = self.done.1.recv() => match done {
                    Done::Folders(folders) => self.watch_folders(folders),
                    Done::Sizes(sizes) => self.tracker.on_sizes(&sizes, self.now_ms()),
                    Done::Icon(id, png) => {
                        if let Some(png) = png {
                            self.icons.insert(id, self.ctx.image(png));
                        }
                    }
                },
                input = self.ctx.next() => match input {
                    Some(ModuleInput::Action(action)) => self.act(&action.key, &action.action),
                    Some(_) => {}
                    None => break,
                },
                _ = sleep_until(wake.unwrap_or_else(Instant::now)), if wake.is_some() => {}
            }
            self.step();
        }
    }

    /// Reads what's due, lets time pass and shows the result.
    fn step(&mut self) {
        let now = self.now_ms();
        if let Some(paths) = self.tracker.due_reads(now) {
            let tx = self.done.0.clone();
            tokio::task::spawn_blocking(move || {
                // From the file itself (not its folder entry), so exact while it's held open.
                let read = |p: &PathBuf| std::fs::metadata(p).ok().filter(|m| m.is_file()).map(|m| m.len());
                let sizes = paths.into_iter().map(|p| (read(&p), p)).map(|(size, p)| (p, size)).collect();
                _ = tx.send(Done::Sizes(sizes));
            });
        }
        self.tracker.tick(now);
        self.show(now);
    }

    fn show(&mut self, now: u64) {
        let active = self.tracker.showing(now);
        self.active_at = self.active_at.min(active.len().saturating_sub(1));
        if active.is_empty() {
            self.ctx.retract(ACTIVE);
        } else {
            self.ctx.publish(ACTIVE, Priority::Ongoing, view::active(&active, self.active_at, now));
        }
        let done = self.tracker.done();
        self.done_at = self.done_at.min(done.len().saturating_sub(1));
        for d in &done {
            if self.asked_icons.insert(d.id) {
                let (tx, id, path) = (self.done.0.clone(), d.id, d.path.clone());
                tokio::task::spawn_blocking(move || _ = tx.send(Done::Icon(id, shell::icon(&path))));
            }
        }
        match done.get(self.done_at) {
            Some(d) => self.ctx.publish(DONE, Priority::Brief, view::done(&done, self.done_at, self.icons.get(&d.id).copied())),
            None => self.ctx.retract(DONE),
        }
        let shown: HashSet<u64> = done.iter().map(|d| d.id).collect();
        self.icons.retain(|id, _| shown.contains(id));
        self.asked_icons.retain(|id| shown.contains(id));
    }

    fn act(&mut self, key: &str, action: &str) {
        let now = self.now_ms();
        let page = |at: &mut usize, action: &str| match action {
            view::PREV => *at = at.saturating_sub(1),
            view::NEXT => *at += 1,
            _ => {}
        };
        match (key, action) {
            (ACTIVE, view::PREV | view::NEXT) => page(&mut self.active_at, action),
            (DONE, view::PREV | view::NEXT) => page(&mut self.done_at, action),
            (DONE, ACTION_EXPANDED) => self.tracker.hold(true, now),
            (DONE, ACTION_COLLAPSED) => self.tracker.hold(false, now),
            ("", "reload") => self.find_folders(),
            _ => {
                let id = |prefix: &str| action.strip_prefix(prefix).and_then(|id| id.parse::<u64>().ok());
                if let Some(id) = id(view::OPEN) {
                    if let Some(d) = self.tracker.dismiss(id) {
                        shell::open(d.path);
                    }
                } else if let Some(id) = id(view::SHOW) {
                    let path = match self.tracker.dismiss(id) {
                        Some(d) => Some(d.path),
                        None => self.tracker.active_path(id).map(PathBuf::from),
                    };
                    if let Some(path) = path {
                        shell::show_in_folder(path);
                    }
                }
            }
        }
    }

    /// Where Downloads is (a shell call, so off the hub thread) and the extra folders.
    fn find_folders(&self) {
        let tx = self.done.0.clone();
        tokio::task::spawn_blocking(move || _ = tx.send(Done::Folders(Settings::load().watched(shell::known(shell::Known::Downloads)))));
    }

    /// (Re)starts the watch when the folders changed. Partial files already there (ReUI
    /// started mid-download) are picked up as if they'd just appeared.
    fn watch_folders(&mut self, folders: Vec<PathBuf>) {
        if folders == self.folders && self.watch.is_some() {
            return;
        }
        self.watch = None;
        self.watch = Watch::start("reui-downloads", folders.clone(), self.changes.0.clone());
        for folder in &folders {
            self.rescan(folder.clone());
        }
        self.folders = folders;
    }

    /// Lists `folder`'s partial files written to lately as additions: after a dropped
    /// buffer, or at start. Old ones are a paused or abandoned download's, not news.
    fn rescan(&self, folder: PathBuf) {
        let tx = self.changes.0.clone();
        tokio::task::spawn_blocking(move || {
            let Ok(entries) = std::fs::read_dir(&folder) else { return };
            let recent = |entry: &std::fs::DirEntry| entry.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).is_some_and(|age| age < RESCAN_RECENT);
            for entry in entries.flatten() {
                if partial::partial(&entry.file_name().to_string_lossy()).is_some() && recent(&entry) {
                    _ = tx.send(Change::Added(entry.path()));
                }
            }
        });
    }
}
