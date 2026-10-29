//! Connection to the hub on a background thread. Snapshots are handed to the UI thread
//! through a channel plus a posted window message; actions go the other way. Reconnects
//! with backoff, so the island survives a hub restart.

use std::io;
use std::sync::mpsc;
use std::time::Duration;

use reui_protocol::{pipe_name, read_frame, write_frame, ActivityId, ClientMsg, Destination, HubMsg, ImageId, Snapshot, SurfaceKind, PROTOCOL_VERSION};
use tokio::io::{ReadHalf, WriteHalf};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

/// Posted to the island window whenever events are waiting in [`Link::events`].
pub const WM_HUB: u32 = WM_APP + 1;

const MIN_BACKOFF: Duration = Duration::from_millis(250);
const MAX_BACKOFF: Duration = Duration::from_secs(4);

pub enum Event {
    Connected,
    Image(ImageId, Vec<u8>),
    Snapshot(Snapshot),
    Focus(ActivityId),
    /// The places to switch between (the view switcher).
    Destinations(Vec<Destination>),
    /// The hide shortcut was pressed: hide the island, or show it again.
    ToggleHidden,
    Disconnected,
}

pub struct Link {
    pub to_hub: UnboundedSender<ClientMsg>,
    pub events: mpsc::Receiver<Event>,
}

#[derive(Clone)]
struct Notifier {
    events: mpsc::Sender<Event>,
    hwnd: isize,
}

impl Notifier {
    fn send(&self, event: Event) {
        if self.events.send(event).is_ok() {
            unsafe { _ = PostMessageW(Some(HWND(self.hwnd as *mut _)), WM_HUB, WPARAM(0), LPARAM(0)) };
        }
    }
}

pub fn spawn(hwnd: HWND) -> Link {
    let (to_hub, outgoing) = unbounded_channel();
    let (events_tx, events) = mpsc::channel();
    let notifier = Notifier { events: events_tx, hwnd: hwnd.0 as isize };
    std::thread::Builder::new()
        .name("hub-link".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio runtime");
            runtime.block_on(run(notifier, outgoing));
        })
        .expect("hub-link thread");
    Link { to_hub, events }
}

async fn run(notifier: Notifier, mut outgoing: UnboundedReceiver<ClientMsg>) {
    let mut backoff = MIN_BACKOFF;
    loop {
        if let Ok(pipe) = ClientOptions::new().open(pipe_name()) {
            backoff = MIN_BACKOFF;
            if let Err(e) = session(pipe, &notifier, &mut outgoing).await {
                eprintln!("island: hub link lost: {e}");
            }
            notifier.send(Event::Disconnected);
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

async fn session(pipe: NamedPipeClient, notifier: &Notifier, outgoing: &mut UnboundedReceiver<ClientMsg>) -> io::Result<()> {
    let (mut reader, mut writer) = tokio::io::split(pipe);
    write_frame(&mut writer, &ClientMsg::Hello { version: PROTOCOL_VERSION, surface: SurfaceKind::Island }).await?;
    match read_frame::<_, HubMsg>(&mut reader).await? {
        HubMsg::Welcome { .. } => notifier.send(Event::Connected),
        HubMsg::Rejected { reason } => return Err(io::Error::other(reason)),
        other => return Err(io::Error::other(format!("expected Welcome, got {other:?}"))),
    }
    // Clicks made while disconnected refer to activities that may no longer exist.
    while outgoing.try_recv().is_ok() {}

    tokio::select! {
        result = read_loop(&mut reader, notifier) => result,
        result = write_loop(&mut writer, outgoing) => result,
    }
}

async fn read_loop(reader: &mut ReadHalf<NamedPipeClient>, notifier: &Notifier) -> io::Result<()> {
    loop {
        match read_frame(reader).await? {
            HubMsg::Snapshot(snapshot) => notifier.send(Event::Snapshot(snapshot)),
            HubMsg::Image { id, bytes } => notifier.send(Event::Image(id, bytes)),
            HubMsg::Focus { id } => notifier.send(Event::Focus(id)),
            HubMsg::Destinations(places) => notifier.send(Event::Destinations(places)),
            HubMsg::ToggleHidden => notifier.send(Event::ToggleHidden),
            _ => {}
        }
    }
}

async fn write_loop(writer: &mut WriteHalf<NamedPipeClient>, outgoing: &mut UnboundedReceiver<ClientMsg>) -> io::Result<()> {
    while let Some(msg) = outgoing.recv().await {
        write_frame(writer, &msg).await?;
    }
    Ok(())
}
