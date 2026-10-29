//! Screenshots (docs/PRD-downloads-screenshots.md): a Brief card with the shot's thumbnail
//! when the user takes one, "Copied" when it's on the clipboard, Open and Show in folder
//! once it's saved.
//!
//! Two ways a shot arrives, often both (`shots.rs` makes them one card):
//! - **The clipboard.** The clipboard module captures every copy anyway; when Snipping
//!   Tool's (Win+Shift+S, PrtScn) is an image it hands the thumbnail here with [`claim`]
//!   and skips its own copy toast, so a shot is never two cards. The history keeps it.
//! - **`Pictures\Screenshots`** (`FOLDERID_Screenshots`; Win+PrtScn, and Snipping Tool's
//!   automatic save), watched with the shared folder watch: a blocked thread, nothing
//!   between shots. A new image is read once, off the hub thread, for its thumbnail.
//!
//! Island actions: `open`, `show`, and the card opening or closing (it stays while open).
//! `screenshots.json` holds only `enabled`, read by `main.rs` at start.
//!
//! Files: `shots.rs` (one shot from its halves), `view.rs`.

mod shots;
mod view;

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Mutex;

use reui_protocol::{ImageId, Priority, ACTION_COLLAPSED, ACTION_EXPANDED};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::time::{sleep_until, Duration, Instant};

use self::shots::{Shots, Thumb};
use super::folders::{shell, Change, Watch};
use super::Module;
use crate::hub::{ModuleContext, ModuleInput};

const KEY: &str = "shot";
/// Thumbnails of saved shots, on their long side (the island draws them at most 56 DIP).
const THUMB_PX: u32 = 256;
/// The apps whose clipboard images are screenshots: Snipping Tool, and the overlay
/// Win+Shift+S opens (older builds).
const TOOLS: [&str; 2] = ["snippingtool.exe", "screenclippinghost.exe"];

/// Where the clipboard module hands copied shots, while this module runs.
static COPIES: Mutex<Option<UnboundedSender<Thumb>>> = Mutex::new(None);

/// Called by the clipboard module for each image it saved: `true` when it came from a
/// screenshot tool and this module takes it (the clipboard then shows no toast).
pub fn claim(exe: Option<&str>, thumb: &[u8], size: Option<(u32, u32)>) -> bool {
    if !exe.is_some_and(|exe| TOOLS.contains(&exe)) {
        return false;
    }
    let copies = COPIES.lock().unwrap_or_else(|e| e.into_inner());
    copies.as_ref().is_some_and(|tx| tx.send((thumb.to_vec(), size)).is_ok())
}

pub struct Screenshots;

impl Module for Screenshots {
    fn name(&self) -> &'static str {
        "screenshots"
    }

    fn run(self: Box<Self>, ctx: ModuleContext) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(run(ctx))
    }
}

/// What comes back from the blocking pool.
enum Done {
    /// The Screenshots folder, and whether it exists yet.
    Folder(Option<PathBuf>, bool),
    /// A saved shot read: its thumbnail and size, or `None` when it didn't decode.
    Read(PathBuf, Option<Thumb>),
}

struct Run {
    ctx: ModuleContext,
    shots: Shots,
    epoch: Instant,
    /// The Screenshots folder; until it exists, its parent is watched for it (`on_parent`).
    folder: Option<PathBuf>,
    on_parent: bool,
    watch: Option<Watch>,
    changes: (UnboundedSender<Change>, UnboundedReceiver<Change>),
    done: (UnboundedSender<Done>, UnboundedReceiver<Done>),
    /// The current shot's thumbnail in the image store, by shot id.
    thumb: Option<(u64, ImageId)>,
}

async fn run(ctx: ModuleContext) {
    let (copies_tx, mut copies) = unbounded_channel();
    *COPIES.lock().unwrap_or_else(|e| e.into_inner()) = Some(copies_tx);
    let mut run = Run { ctx, shots: Shots::default(), epoch: Instant::now(), folder: None, on_parent: false, watch: None, changes: unbounded_channel(), done: unbounded_channel(), thumb: None };
    run.find_folder();
    loop {
        let wake = run.shots.wake_at().map(|t| run.epoch + Duration::from_millis(t));
        tokio::select! {
            Some(change) = run.changes.1.recv() => run.on_change(change),
            Some((thumb, size)) = copies.recv() => {
                let now = run.now_ms();
                run.shots.copied(thumb, size, now);
            }
            Some(done) = run.done.1.recv() => match done {
                Done::Folder(folder, exists) => run.watch_folder(folder, exists),
                Done::Read(path, thumb) => {
                    let now = run.now_ms();
                    run.shots.file_read(path, thumb, now);
                }
            },
            input = run.ctx.next() => match input {
                Some(ModuleInput::Action(action)) => run.act(&action.action),
                Some(_) => {}
                None => break,
            },
            _ = sleep_until(wake.unwrap_or_else(Instant::now)), if wake.is_some() => {}
        }
        run.step();
    }
    *COPIES.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

impl Run {
    fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }

    fn on_change(&mut self, change: Change) {
        let now = self.now_ms();
        if self.on_parent {
            // Only the folder itself appearing (the first Win+PrtScn makes it) matters in
            // its parent, Pictures, whose own images aren't screenshots.
            if let Change::Added(path) | Change::Renamed { to: path, .. } = change {
                if Some(&path) == self.folder.as_ref() {
                    self.watch_folder(Some(path), true);
                }
            }
            return;
        }
        match change {
            Change::Added(path) | Change::Modified(path) => self.shots.file_changed(&path, now),
            Change::Renamed { from, to } => {
                self.shots.file_gone(&from);
                self.shots.file_changed(&to, now);
            }
            Change::Removed(path) => self.shots.file_gone(&path),
            Change::Overflow(_) => {}
        }
    }

    /// Reads a settled file, lets time pass and shows the card.
    fn step(&mut self) {
        let now = self.now_ms();
        if let Some(path) = self.shots.due_read(now) {
            let tx = self.done.0.clone();
            tokio::task::spawn_blocking(move || {
                let thumb = std::fs::read(&path).ok().and_then(|bytes| Some((crate::modules::clipboard::thumbnail(&bytes, THUMB_PX)?, shots::png_size(&bytes))));
                _ = tx.send(Done::Read(path, thumb));
            });
        }
        self.shots.tick(now);
        self.show();
    }

    fn show(&mut self) {
        let Some(shot) = self.shots.current() else {
            self.thumb = None;
            self.ctx.retract(KEY);
            return;
        };
        if self.thumb.is_none_or(|(id, _)| id != shot.id) {
            self.thumb = shot.thumb.clone().map(|png| (shot.id, self.ctx.image(png)));
        }
        let view = view::shot(shot, self.thumb.map(|(_, image)| image));
        self.ctx.publish(KEY, Priority::Brief, view);
    }

    fn act(&mut self, action: &str) {
        let now = self.now_ms();
        match action {
            ACTION_EXPANDED => self.shots.hold(true, now),
            ACTION_COLLAPSED => self.shots.hold(false, now),
            view::OPEN | view::SHOW => {
                let Some(file) = self.shots.current().and_then(|s| s.file.clone()) else { return };
                self.shots.dismiss();
                if action == view::OPEN {
                    shell::open(file);
                } else {
                    shell::show_in_folder(file);
                }
            }
            _ => {}
        }
    }

    /// Where Screenshots is and whether it exists (shell and disk calls, off the hub thread).
    fn find_folder(&self) {
        let tx = self.done.0.clone();
        tokio::task::spawn_blocking(move || {
            let folder = shell::known(shell::Known::Screenshots);
            let exists = folder.as_deref().is_some_and(std::path::Path::is_dir);
            _ = tx.send(Done::Folder(folder, exists));
        });
    }

    fn watch_folder(&mut self, folder: Option<PathBuf>, exists: bool) {
        self.watch = None;
        self.folder = folder.clone();
        self.on_parent = !exists;
        let Some(folder) = folder else { return };
        let target = if exists { Some(folder) } else { folder.parent().map(PathBuf::from) };
        self.watch = target.and_then(|t| Watch::start("reui-screenshots", vec![t], self.changes.0.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_screenshot_tools_are_claimed_and_only_while_running() {
        assert!(!claim(Some("snippingtool.exe"), &[1], None), "the module isn't running");
        let (tx, mut rx) = unbounded_channel();
        *COPIES.lock().unwrap() = Some(tx);
        assert!(!claim(Some("mspaint.exe"), &[1], None));
        assert!(!claim(None, &[1], None));
        assert!(claim(Some("screenclippinghost.exe"), &[1, 2], Some((4, 3))));
        assert_eq!(rx.try_recv().ok(), Some((vec![1, 2], Some((4, 3)))));
        *COPIES.lock().unwrap() = None;
    }
}
