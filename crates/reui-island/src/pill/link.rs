//! The hub link: what the pipe delivers (snapshots, images, focus requests), drained once
//! for all the islands ([`drain`]) and applied to each ([`Pill::apply_batch`]), and the
//! snapshot a pill rebuilt at another scale starts from.

use std::collections::HashSet;
use std::sync::mpsc::Receiver;

use reui_protocol::{ActivityId, Destination, Snapshot};

use crate::features::clipboard;
use crate::images;
use crate::ipc::Event;

use super::Pill;

/// What the link delivered since the last look. Only the newest snapshot matters.
#[derive(Default)]
pub struct Batch {
    pub snapshot: Option<Snapshot>,
    pub focus: Option<ActivityId>,
    pub places: Option<Vec<Destination>>,
    /// The hide shortcut, pressed an odd number of times (twice is back where it was).
    pub toggle_hidden: bool,
    /// The open shortcut, likewise.
    pub open: bool,
    /// The hub went away: images are gone too.
    pub disconnected: bool,
}

/// Takes everything queued. Images are kept encoded in the shared store, and evicted in
/// stream order, exactly where the hub does, so images that arrived for a later snapshot
/// survive; each island decodes what it shows.
pub fn drain(events: &Receiver<Event>) -> Batch {
    let mut batch = Batch::default();
    while let Ok(event) = events.try_recv() {
        match event {
            Event::Connected => eprintln!("island: connected to hub"),
            Event::ToggleHidden => batch.toggle_hidden = !batch.toggle_hidden,
            Event::OpenIsland => batch.open = !batch.open,
            Event::Focus(id) => batch.focus = Some(id),
            Event::Destinations(list) => batch.places = Some(list),
            Event::Image(id, bytes) => images::put(id, bytes),
            Event::Snapshot(snapshot) => {
                images::evict(&clipboard::referenced(&snapshot));
                batch.snapshot = Some(snapshot);
            }
            Event::Disconnected => {
                images::clear();
                batch.places = Some(Vec::new());
                batch.snapshot = Some(Snapshot::default());
                batch.disconnected = true;
            }
        }
    }
    batch
}

impl Pill {
    /// The last snapshot, for a pill rebuilt at another scale (see `main::rebuild`).
    pub fn take_snapshot(&mut self) -> Snapshot {
        std::mem::take(&mut self.raw)
    }

    /// The snapshot as received, for an island that's just been added.
    pub fn received(&self) -> &Snapshot {
        &self.raw
    }

    /// Shows `snapshot` on a freshly built pill: its images are decoded again from the
    /// encoded copies the link keeps. `places` are the old pill's (the hub sends them only
    /// when they change).
    pub fn restore(&mut self, snapshot: Snapshot, places: Vec<Destination>) {
        self.tabs.places = places;
        self.decode_images(&snapshot);
        if let Err(e) = self.apply(snapshot) {
            eprintln!("island: render failed: {e}");
        }
    }

    /// Decodes the images `snapshot` shows that this island hasn't yet, and drops the rest.
    fn decode_images(&mut self, snapshot: &Snapshot) {
        let shown = clipboard::referenced(snapshot);
        for &id in &shown {
            if self.renderer.has_image(id) {
                continue;
            }
            if let Some(bytes) = images::get(id) {
                if let Err(e) = self.renderer.add_image(id, &bytes) {
                    eprintln!("island: can't decode image: {e}");
                }
            }
        }
        self.renderer.retain_images(&shown);
    }

    /// Applies what the link delivered. Every island shows the snapshot and the places;
    /// only the `lead` (the island on the pointer's display, or the only one) opens a card
    /// asked for by its hotkey or the open shortcut. The hide shortcut is the caller's: it's
    /// one setting for all of them.
    pub fn apply_batch(&mut self, batch: &Batch, lead: bool) {
        if batch.disconnected {
            self.renderer.retain_images(&HashSet::new());
        }
        if let Some(snapshot) = &batch.snapshot {
            self.decode_images(snapshot);
            if let Err(e) = self.apply(snapshot.clone()) {
                eprintln!("island: render failed: {e}");
            }
        }
        if let Some(list) = &batch.places {
            self.set_places(list.clone());
        }
        if !lead {
            return;
        }
        if let Some(id) = &batch.focus {
            self.focus(id.clone());
        }
        if batch.open {
            self.open_from_shortcut();
        }
    }
}
