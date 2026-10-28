//! System events shared by modules: one hidden top-level window (see `window.rs`) that
//! receives power settings, resume and unlock, clock changes, device-interface arrivals (volumes, portable
//! devices), brightness keys as Raw Input, the lock keys, Explorer restarts and display
//! changes, and hands them to every subscriber.
//!
//! The window starts with the first subscriber and then lives as long as the hub; a module
//! that restarts just subscribes again. The last value of each power setting is kept, and a
//! new subscriber gets it straight away marked `initial` (as if it had just registered), so
//! the OSD and the Devices module each see the same "first value, not a change" they would
//! with a window of their own.

mod window;

use std::sync::{Mutex, OnceLock};

use tokio::sync::mpsc::UnboundedSender;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockKey {
    Caps,
    Num,
}

/// Where the machine's power comes from (`GUID_ACDC_POWER_SOURCE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerSource {
    Ac,
    Battery,
    /// A short-term source such as a UPS (`PoHot`).
    Ups,
}

impl PowerSource {
    fn from_value(value: u32) -> Self {
        match value {
            0 => Self::Ac,
            2 => Self::Ups,
            _ => Self::Battery,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystemEvent {
    /// The built-in panel's brightness in percent. `initial` for the value Windows sends
    /// right after registering (not a change).
    Brightness { percent: u32, initial: bool },
    /// A brightness key was pressed (only on keyboards that report them as HID).
    BrightnessKey,
    BatterySaver { on: bool, initial: bool },
    Lock { key: LockKey, on: bool },
    /// Explorer (re)started: the taskbar and Windows' flyout host are new.
    TaskbarCreated,
    DisplaysChanged,
    PowerSource { source: PowerSource, initial: bool },
    /// The battery's charge in percent (Windows only sends it on machines with one).
    BatteryPercent { percent: u32, initial: bool },
    /// Back from sleep (`PBT_APMRESUMEAUTOMATIC`).
    Resumed,
    /// The session was unlocked.
    Unlocked,
    /// The system clock or time zone changed (`WM_TIMECHANGE`).
    TimeChanged,
    /// A storage volume arrived or left; the receiver rescans.
    Volume,
    /// A portable device (phone, camera over MTP/PTP) arrived or left. `id` is a per-session
    /// hash of its interface path; `apple` when it's an Apple device (an iPhone or iPad).
    PortableDevice { arrived: bool, apple: bool, id: u64 },
}

/// Power settings whose last value is kept for new subscribers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Setting {
    Brightness,
    BatterySaver,
    PowerSource,
    BatteryPercent,
}

impl Setting {
    const ALL: [Setting; 4] = [Setting::Brightness, Setting::BatterySaver, Setting::PowerSource, Setting::BatteryPercent];

    fn event(self, value: u32, initial: bool) -> SystemEvent {
        match self {
            Setting::Brightness => SystemEvent::Brightness { percent: value.min(100), initial },
            Setting::BatterySaver => SystemEvent::BatterySaver { on: value != 0, initial },
            Setting::PowerSource => SystemEvent::PowerSource { source: PowerSource::from_value(value), initial },
            Setting::BatteryPercent => SystemEvent::BatteryPercent { percent: value.min(100), initial },
        }
    }
}

struct Subscriber {
    id: u64,
    events: UnboundedSender<SystemEvent>,
    lock_keys: bool,
}

/// Subscribers and the last power-setting values; pure, so the replay rules are tested.
struct Broker {
    subscribers: Vec<Subscriber>,
    next_id: u64,
    last: [Option<u32>; 4],
}

impl Broker {
    const fn new() -> Self {
        Self { subscribers: Vec::new(), next_id: 0, last: [None; 4] }
    }

    fn join(&mut self, events: UnboundedSender<SystemEvent>) -> u64 {
        self.next_id += 1;
        for (setting, value) in Setting::ALL.iter().zip(self.last) {
            if let Some(value) = value {
                _ = events.send(setting.event(value, true));
            }
        }
        self.subscribers.push(Subscriber { id: self.next_id, events, lock_keys: false });
        self.next_id
    }

    fn leave(&mut self, id: u64) {
        self.subscribers.retain(|s| s.id != id);
    }

    fn send(&mut self, event: SystemEvent) {
        // A closed receiver is a module that stopped without dropping its subscription yet.
        self.subscribers.retain(|s| s.events.send(event.clone()).is_ok());
    }

    /// The first value of a setting is `initial`; later ones are changes.
    fn setting(&mut self, setting: Setting, value: u32) {
        let slot = &mut self.last[setting as usize];
        let initial = slot.is_none();
        *slot = Some(value);
        self.send(setting.event(value, initial));
    }

    fn lock_keys_wanted(&self) -> bool {
        self.subscribers.iter().any(|s| s.lock_keys)
    }
}

static BROKER: Mutex<Broker> = Mutex::new(Broker::new());
/// The window's handle (as an integer, so it can be shared), or `None` if it couldn't be made.
static WINDOW: OnceLock<Option<isize>> = OnceLock::new();

fn broker<R>(f: impl FnOnce(&mut Broker) -> R) -> R {
    f(&mut BROKER.lock().unwrap_or_else(|e| e.into_inner()))
}

/// From the window thread.
fn dispatch(event: SystemEvent) {
    broker(|b| b.send(event));
}

fn dispatch_setting(setting: Setting, value: u32) {
    broker(|b| b.setting(setting, value));
}

/// A module's subscription; dropping it stops the events (the window stays for others).
pub struct Subscription {
    id: u64,
    hwnd: isize,
}

impl Subscription {
    /// Watches Caps Lock / Num Lock (a low-level keyboard hook) only while some subscriber
    /// wants them.
    pub fn watch_lock_keys(&self, on: bool) {
        let wanted = broker(|b| {
            if let Some(s) = b.subscribers.iter_mut().find(|s| s.id == self.id) {
                s.lock_keys = on;
            }
            b.lock_keys_wanted()
        });
        window::set_lock_hook(self.hwnd, wanted);
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let wanted = broker(|b| {
            b.leave(self.id);
            b.lock_keys_wanted()
        });
        window::set_lock_hook(self.hwnd, wanted);
    }
}

/// Starts receiving system events (the first call creates the window, so it blocks for a
/// moment: call it off the hub thread). `None` if the window couldn't be created.
pub fn subscribe(events: UnboundedSender<SystemEvent>) -> Option<Subscription> {
    let hwnd = (*WINDOW.get_or_init(window::start))?;
    let id = broker(|b| b.join(events));
    Some(Subscription { id, hwnd })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_events_replay_the_last_settings_as_initial() {
        let mut b = Broker::new();
        let (tx1, mut rx1) = tokio::sync::mpsc::unbounded_channel();
        b.join(tx1);
        b.setting(Setting::Brightness, 70);
        b.setting(Setting::Brightness, 60);
        b.setting(Setting::PowerSource, 2);
        assert_eq!(rx1.try_recv().unwrap(), SystemEvent::Brightness { percent: 70, initial: true });
        assert_eq!(rx1.try_recv().unwrap(), SystemEvent::Brightness { percent: 60, initial: false });
        assert_eq!(rx1.try_recv().unwrap(), SystemEvent::PowerSource { source: PowerSource::Ups, initial: true });

        // A later subscriber gets the current values as initial ones, then changes.
        let (tx2, mut rx2) = tokio::sync::mpsc::unbounded_channel();
        let second = b.join(tx2);
        assert_eq!(rx2.try_recv().unwrap(), SystemEvent::Brightness { percent: 60, initial: true });
        assert_eq!(rx2.try_recv().unwrap(), SystemEvent::PowerSource { source: PowerSource::Ups, initial: true });
        assert!(rx2.try_recv().is_err());
        b.send(SystemEvent::Resumed);
        assert_eq!(rx1.try_recv().unwrap(), SystemEvent::Resumed);
        assert_eq!(rx2.try_recv().unwrap(), SystemEvent::Resumed);

        b.leave(second);
        b.send(SystemEvent::Unlocked);
        assert!(rx2.try_recv().is_err());
    }

    #[test]
    fn system_events_lock_keys_follow_whoever_wants_them() {
        let mut b = Broker::new();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let a = b.join(tx.clone());
        b.join(tx);
        assert!(!b.lock_keys_wanted());
        b.subscribers.iter_mut().find(|s| s.id == a).unwrap().lock_keys = true;
        assert!(b.lock_keys_wanted());
        b.leave(a);
        assert!(!b.lock_keys_wanted());
        assert_eq!(PowerSource::from_value(0), PowerSource::Ac);
        assert_eq!(PowerSource::from_value(1), PowerSource::Battery);
    }
}
