//! The hub link: what the pipe delivers (snapshots, images, focus requests) applied to the
//! pill as it arrives, and handing the link over to a pill rebuilt at another scale.

use std::collections::HashSet;
use std::sync::mpsc::Receiver;

use reui_protocol::{ClientMsg, Destination, Snapshot};
use tokio::sync::mpsc::UnboundedSender;

use crate::features::clipboard;
use crate::images;
use crate::ipc::Event;

use super::Pill;

impl Pill {
    /// Hands over the hub link and the last snapshot, for a pill rebuilt at another scale
    /// (see `main::rebuild`). This pill is left without a link.
    pub fn take_link(&mut self) -> (UnboundedSender<ClientMsg>, Receiver<Event>, Snapshot) {
        let to_hub = std::mem::replace(&mut self.to_hub, tokio::sync::mpsc::unbounded_channel().0);
        let events = std::mem::replace(&mut self.events, std::sync::mpsc::channel().1);
        (to_hub, events, std::mem::take(&mut self.raw))
    }

    /// Shows `snapshot` on a freshly built pill: its images are decoded again from the
    /// encoded copies the link keeps. `places` are the old pill's (the hub sends them only
    /// when they change).
    pub fn restore(&mut self, snapshot: Snapshot, places: Vec<Destination>) {
        self.tabs.places = places;
        for id in clipboard::referenced(&snapshot) {
            if let Some(bytes) = images::get(id) {
                if let Err(e) = self.renderer.add_image(id, &bytes) {
                    eprintln!("island: can't decode image: {e}");
                }
            }
        }
        if let Err(e) = self.apply(snapshot) {
            eprintln!("island: render failed: {e}");
        }
    }

    /// Apply everything the hub link queued; only the newest snapshot matters. Messages for
    /// module windows are returned, in order, for the caller to deliver outside this borrow.
    pub fn drain_hub(&mut self) {
        let mut latest = None;
        let mut focus = None;
        let mut places = None;
        let mut toggles = 0;
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Connected => eprintln!("island: connected to hub"),
                Event::ToggleHidden => toggles += 1,
                Event::Focus(id) => focus = Some(id),
                Event::Destinations(list) => places = Some(list),
                // Kept encoded; the pill decodes what a snapshot shows (below).
                Event::Image(id, bytes) => images::put(id, bytes),
                // Evict in stream order, exactly where the hub does, so images that arrived
                // for a later snapshot survive.
                Event::Snapshot(snapshot) => {
                    let shown = clipboard::referenced(&snapshot);
                    images::evict(&shown);
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
                    latest = Some(snapshot);
                }
                Event::Disconnected => {
                    places = Some(Vec::new());
                    images::clear();
                    self.renderer.retain_images(&HashSet::new());
                    latest = Some(Snapshot::default());
                }
            }
        }
        if let Some(snapshot) = latest {
            if let Err(e) = self.apply(snapshot) {
                eprintln!("island: render failed: {e}");
            }
        }
        if let Some(list) = places {
            self.set_places(list);
        }
        if let Some(id) = focus {
            self.focus(id);
        }
        // Pressed twice before we got to it: back where it was.
        if toggles % 2 == 1 {
            self.toggle_hidden();
        }
    }
}
