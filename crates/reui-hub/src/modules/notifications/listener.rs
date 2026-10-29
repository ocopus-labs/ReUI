//! The OS side: Windows' `UserNotificationListener` on a thread of its own (every call
//! blocks on a cross-process RPC to the notification service, ~8 ms each), listing the
//! notification centre when `schedule.rs` says so and handing the module what's new.
//!
//! Without package identity (ReUI is a plain per-user exe) the listener can read and
//! remove, but not subscribe: `NotificationChanged` and `ClearNotifications` fail with
//! 0x80070490. Access is the user's "Notification access" privacy switch; asking for it
//! never shows a prompt to an unpackaged app (measured: answered in ~4 ms).
//!
//! What's read, and when: every list reads only ids. For a new id, its app (AUMID and
//! display name) is read, so the user can pick it; its title and text only when the app is
//! allowed and content isn't hidden. Nothing here is logged or written down.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::sync::mpsc::UnboundedSender;
use windows::core::Interface;
use windows::Foundation::Size;
use windows::Storage::Streams::DataReader;
use windows::UI::Notifications::Management::{UserNotificationListener, UserNotificationListenerAccessStatus};
use windows::UI::Notifications::{KnownNotificationBindings, NotificationKinds, UserNotification};

use super::diff::Seen;
use super::schedule::{Schedule, Trigger};
use super::wnf;

/// A logo bigger than this isn't an app logo.
const MAX_LOGO: u32 = 1024 * 1024;
const LOGO_PX: u32 = 64;
/// FILETIME of the Unix epoch.
const UNIX_EPOCH_FILETIME: i64 = 116_444_736_000_000_000;

/// What the module needs to know about a new toast.
#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    pub id: u32,
    /// The AUMID: what the allow-list holds, and how the app is opened.
    pub app: String,
    pub name: String,
    /// Read only for an allowed app with content shown.
    pub title: Option<String>,
    pub body: Option<String>,
    pub created_ms: u64,
    /// A packaged app's logo as a PNG (desktop apps have none through this API).
    pub logo: Option<Vec<u8>>,
}

#[derive(Debug)]
pub enum Event {
    /// The user's "Notification access" switch is off (or the listener isn't there).
    NoAccess,
    Trigger(Trigger),
    Arrived(Vec<Toast>),
    Gone(Vec<u32>),
}

/// What the thread may read: the module replaces it when the settings change.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub apps: Vec<String>,
    pub hide_content: bool,
}

enum Command {
    Changed,
    Stop,
}

pub struct Listener {
    commands: Sender<Command>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Listener {
    /// Joins the thread (a list in flight finishes first): drop it off the hub thread.
    fn drop(&mut self) {
        _ = self.commands.send(Command::Stop);
        if let Some(t) = self.thread.take() {
            _ = t.join();
        }
    }
}

pub fn start(events: UnboundedSender<Event>, filter: Arc<Mutex<Filter>>) -> Option<Listener> {
    let (commands, rx) = mpsc::channel();
    let signal = commands.clone();
    let thread = std::thread::Builder::new().name("reui-notifications".into()).spawn(move || run(events, filter, rx, signal)).ok()?;
    Some(Listener { commands, thread: Some(thread) })
}

fn run(events: UnboundedSender<Event>, filter: Arc<Mutex<Filter>>, commands: Receiver<Command>, signal: Sender<Command>) {
    unsafe {
        _ = windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED);
    }
    let Some(listener) = access() else {
        _ = events.send(Event::NoAccess);
        return;
    };
    let subscription = wnf::subscribe(
        wnf::NOTIFICATION_COUNT,
        Box::new(move |_| {
            _ = signal.send(Command::Changed);
        }),
    );
    let trigger = if subscription.is_some() { Trigger::Wnf } else { Trigger::Poll };
    _ = events.send(Event::Trigger(trigger));

    let mut seen = Seen::default();
    match list(&listener) {
        Ok(toasts) => _ = seen.update(&ids(&toasts)),
        Err(e) => eprintln!("notifications: can't list ({e})"),
    }
    let mut schedule = Schedule::new(trigger, Instant::now());
    loop {
        let now = Instant::now();
        match commands.recv_timeout(schedule.next(now).saturating_duration_since(now)) {
            Ok(Command::Changed) => schedule.signal(Instant::now()),
            Ok(Command::Stop) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }
        let now = Instant::now();
        if now < schedule.next(now) {
            continue;
        }
        // A locked screen shows no island: a timed poll waits (a count change still lists).
        if schedule.trigger() == Trigger::Poll && !schedule.signalled(now) && locked() {
            schedule.listed(now, false);
            continue;
        }
        let toasts = match list(&listener) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("notifications: can't list ({e})");
                schedule.listed(now, false);
                continue;
            }
        };
        let change = seen.update(&ids(&toasts));
        if schedule.listed(now, !change.added.is_empty()) {
            eprintln!("notifications: the count missed a toast; polling from now on");
            _ = events.send(Event::Trigger(Trigger::Poll));
        }
        if !change.added.is_empty() {
            let filter = filter.lock().map(|f| f.clone()).unwrap_or_default();
            let new: Vec<Toast> = toasts.iter().filter(|t| t.Id().is_ok_and(|id| change.added.contains(&id))).filter_map(|t| read(t, &filter)).collect();
            _ = events.send(Event::Arrived(new));
        }
        if !change.removed.is_empty() {
            _ = events.send(Event::Gone(change.removed));
        }
    }
    drop(subscription);
}

/// The listener, if the user allows ReUI to read notifications.
fn access() -> Option<UserNotificationListener> {
    let listener = UserNotificationListener::Current().ok()?;
    let status = match listener.GetAccessStatus() {
        Ok(UserNotificationListenerAccessStatus::Allowed) => UserNotificationListenerAccessStatus::Allowed,
        // Unspecified: never asked. Unpackaged, this answers from the privacy switch
        // without showing anything.
        _ => listener.RequestAccessAsync().and_then(|op| op.join()).ok()?,
    };
    (status == UserNotificationListenerAccessStatus::Allowed).then_some(listener)
}

fn list(listener: &UserNotificationListener) -> windows::core::Result<Vec<UserNotification>> {
    let found = listener.GetNotificationsAsync(NotificationKinds::Toast)?.join()?;
    Ok((&found).into_iter().collect())
}

fn ids(toasts: &[UserNotification]) -> Vec<u32> {
    toasts.iter().filter_map(|t| t.Id().ok()).collect()
}

/// A new toast's app always; its text and logo only when the filter allows them.
fn read(toast: &UserNotification, filter: &Filter) -> Option<Toast> {
    let info = toast.AppInfo().ok()?;
    let app = info.AppUserModelId().ok()?.to_string();
    let display = info.DisplayInfo().ok();
    let name = display.as_ref().and_then(|d| d.DisplayName().ok()).map(|n| n.to_string()).unwrap_or_default();
    let created_ms = toast.CreationTime().map(|t| (t.UniversalTime - UNIX_EPOCH_FILETIME).max(0) as u64 / 10_000).unwrap_or(0);
    let allowed = filter.apps.iter().any(|a| a.eq_ignore_ascii_case(&app));
    let (mut title, mut body) = (None, None);
    if allowed && !filter.hide_content {
        let lines = texts(toast);
        title = lines.first().cloned();
        body = lines.get(1).cloned();
    }
    let logo = if allowed { display.as_ref().and_then(logo) } else { None };
    Some(Toast { id: toast.Id().ok()?, app, name, title, body, created_ms, logo })
}

/// The toast's text lines (ToastGeneric): the first is its title.
fn texts(toast: &UserNotification) -> Vec<String> {
    let binding = toast.Notification().and_then(|n| n.Visual()).and_then(|v| v.GetBinding(&KnownNotificationBindings::ToastGeneric()?));
    let Ok(binding) = binding else { return Vec::new() };
    let Ok(elements) = binding.GetTextElements() else { return Vec::new() };
    (&elements).into_iter().filter_map(|e| e.Text().ok()).map(|t| t.to_string().trim().to_owned()).filter(|t| !t.is_empty()).collect()
}

/// A packaged app's logo, scaled to a small PNG. Desktop apps get a null reference.
fn logo(display: &windows::ApplicationModel::AppDisplayInfo) -> Option<Vec<u8>> {
    let stream = display.GetLogo(Size { Width: LOGO_PX as f32, Height: LOGO_PX as f32 }).ok()?.OpenReadAsync().ok()?.join().ok()?;
    let size = stream.Size().ok()? as u32;
    if size == 0 || size > MAX_LOGO {
        return None;
    }
    let reader = DataReader::CreateDataReader(&stream).ok()?;
    let loaded = reader.LoadAsync(size).ok()?.cast::<windows_future::IAsyncOperation<u32>>().ok()?.join().ok()?;
    let mut bytes = vec![0u8; loaded as usize];
    reader.ReadBytes(&mut bytes).ok()?;
    super::super::clipboard::thumbnail(&bytes, LOGO_PX)
}

/// Whether the session is locked (the island can't be seen, so a timed poll can wait).
fn locked() -> bool {
    use windows::core::PWSTR;
    use windows::Win32::System::RemoteDesktop::{WTSFreeMemory, WTSQuerySessionInformationW, WTSSessionInfoEx, WTSINFOEXW, WTS_CURRENT_SERVER_HANDLE, WTS_CURRENT_SESSION, WTS_SESSIONSTATE_LOCK};
    unsafe {
        let (mut buffer, mut bytes) = (PWSTR::null(), 0u32);
        if WTSQuerySessionInformationW(Some(WTS_CURRENT_SERVER_HANDLE), WTS_CURRENT_SESSION, WTSSessionInfoEx, &mut buffer, &mut bytes).is_err() || buffer.is_null() {
            return false;
        }
        let info = &*(buffer.0 as *const WTSINFOEXW);
        let locked = info.Level == 1 && info.Data.WTSInfoExLevel1.SessionFlags as u32 == WTS_SESSIONSTATE_LOCK;
        WTSFreeMemory(buffer.0.cast());
        locked
    }
}
