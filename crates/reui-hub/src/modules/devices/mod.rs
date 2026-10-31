//! Devices: Bluetooth connects and batteries, low-battery alerts, the laptop's charging,
//! external drives with Eject, phones, and an overview of all of it (docs/PRD-devices.md).
//!
//! Event flow, all callbacks; the only polling is one battery read a minute while a battery
//! device is connected:
//! - Bluetooth connects: two `DeviceWatcher`s (`bluetooth.rs`, WinRT thread pool), grouped
//!   by container (`registry.rs`) and filtered by the anti-spam gate (`spam.rs`);
//! - what each device exactly is: its Device ID record, read once per container from the
//!   devnodes (`pnp.rs`, blocking pool), and the bundled catalog (`catalog/`, no network);
//! - Bluetooth batteries: cfgmgr reads (`pnp.rs`) on the blocking pool, 3 s and 10 s after a
//!   connect, then every 60 s while a battery device is connected, and when the overview
//!   opens; AirPods buds and case from short advertisement scans (`airpods.rs`);
//! - controllers: `Windows.Gaming.Input` events and a 60 s battery poll (`controllers.rs`);
//! - power source, battery %, resume, unlock, volume and phone arrivals: the shared
//!   system-events window (`modules/system_events`); charge rate and the slow charger:
//!   `AggregateBattery` / `PowerManager` (`power.rs`);
//! - drives: rescans and Eject (`drives.rs`, `eject.rs`) on the blocking pool.
//!
//! Activities (module `devices`): `device:<id>` connect cards (Brief), `low:<id>` (Brief,
//! Alert at ≤ 10 %), `power` (Brief), `drive:<letter>` (Brief), `phone`, `ups`,
//! `slow-charger` (Brief), and the overview `devices` (Passive). `<id>` is a per-session
//! hash; names, addresses and instance ids are never logged.
//!
//! Files: `peripherals.rs` and `machine.rs` (the module's handling of each source),
//! `view.rs`, `art.rs` (ReUI's device drawings), `moments.rs` (how long things stay),
//! `kinds.rs`, `low.rs`, `handoff.rs` (the OSD hand-off) and `settings.rs`.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reui_protocol::{Priority, Quiet, ACTION_COLLAPSED, ACTION_EXPANDED};
use tokio::sync::mpsc;
use tokio::time::{sleep_until, Instant};
use windows::Gaming::Input::RawGameController;

mod airpods;
mod art;
mod bluetooth;
mod catalog;
mod controllers;
mod drives;
mod eject;
pub mod handoff;
mod kinds;
mod low;
mod machine;
mod moments;
mod peripherals;
mod pnp;
/// Shared with the stats card, which reads the battery's charge rate.
pub(crate) mod power;
mod registry;
mod rehearse;
mod settings;
mod spam;
mod view;

use self::airpods::{Expect, Scan, Sighting};
use self::bluetooth::{BtEvent, Watchers};
use self::controllers::{Controllers, PadEvent, Reading};
use self::drives::Drive;
use self::eject::Refused;
use self::low::LowBattery;
use self::moments::Moments;
use self::power::{Eta, PowerEvent, PowerWatch};
use self::registry::Registry;
use self::settings::Settings;
use self::spam::Gate;
use super::system_events::{self, PowerSource, Subscription, SystemEvent};
use super::{supervise, Module};
use crate::hub::{ModuleAction, ModuleContext, ModuleInput, View};

const OVERVIEW: &str = "devices";
/// The overview goes if it isn't opened this soon after it was asked for.
const OVERVIEW_UNOPENED: Duration = Duration::from_secs(10);
const ACTION_OPEN: &str = "open";

pub struct Devices;

impl Module for Devices {
    fn name(&self) -> &'static str {
        "devices"
    }

    fn run(self: Box<Self>, mut ctx: ModuleContext) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        ctx.declare_quiet(quiet);
        Box::pin(async move { supervise("devices", &mut ctx, |ctx| Box::pin(run(ctx))).await })
    }
}

/// Each kind of moment turns off with its own switch in `devices.json` (Settings →
/// Devices), found by its key: `device:` a connect card, `low:` a low battery, `drive:` a
/// drive. Being on a UPS has no switch, so it can only be snoozed.
fn quiet(key: &str, _: Priority) -> Option<Quiet> {
    let setting = match key {
        k if k.starts_with("device:") => "connect_cards",
        k if k.starts_with("low:") => "low_battery",
        k if k.starts_with("drive:") => "drives",
        machine::POWER => "charging",
        "phone" => "phones",
        "slow-charger" => "slow_charger",
        "ups" => return Some(Quiet::snooze_only()),
        _ => return None,
    };
    Some(Quiet::setting("devices.json", setting))
}

/// The adapters, started together off the hub thread.
struct Adapters {
    events: Option<Subscription>,
    bluetooth: Option<Watchers>,
    pads: Option<Controllers>,
    power: Option<PowerWatch>,
    has_battery: bool,
}

/// Results of work done off the hub thread.
enum Internal {
    Started(Adapters),
    Settings(Settings),
    Batteries(HashMap<u128, u8>),
    /// Device ID records read for these containers.
    Ids(Vec<u128>, HashMap<u128, catalog::DeviceIds>),
    Pads(Vec<Reading>),
    Drives(Vec<Drive>),
    Ejected(char, Result<(), Refused>),
}

/// A game controller and what's been seen of it.
struct Pad {
    reading: Reading,
    low: LowBattery,
    /// What it is, from its USB ids and name.
    identity: catalog::Identity,
}

/// The current connect card: its activity key and the devices it stands for.
struct ConnectCard {
    key: String,
    members: Vec<u64>,
    since: Instant,
}

/// The current drive card: its key, its drives, and when it started (arrivals merge for 2 s).
struct DriveCard {
    key: String,
    letters: Vec<char>,
    since: Instant,
}

/// An AirPods scan burst: for which device, which family to accept, until when.
struct Burst {
    _scan: Scan,
    device: u64,
    expect: Expect,
    until: Instant,
}

struct State {
    settings: Settings,
    internal: mpsc::UnboundedSender<Internal>,
    sightings: mpsc::UnboundedSender<Sighting>,
    events: Option<Subscription>,
    bluetooth: Option<Watchers>,
    pads_watch: Option<Arc<Mutex<HashMap<u64, RawGameController>>>>,
    _pads_events: Option<Controllers>,
    power: Option<PowerWatch>,
    moments: Moments,

    // Bluetooth.
    gate: Gate<u64>,
    registry: Registry,
    connect: Option<ConnectCard>,
    battery_reads: Vec<Instant>,
    battery_poll: Option<Instant>,
    /// A Device ID read is running (one at a time; containers seen meanwhile wait for it).
    reading_ids: bool,
    burst: Option<Burst>,

    // Controllers.
    pads: HashMap<u64, Pad>,
    pad_poll: Option<Instant>,

    // The laptop.
    has_battery: Option<bool>,
    source: Option<PowerSource>,
    percent: Option<u32>,
    laptop_low: LowBattery,
    eta: Eta,

    // Drives and phones.
    drives: Option<Vec<Drive>>,
    refused: HashMap<char, String>,
    drive_card: Option<DriveCard>,
    rescans: Vec<Instant>,
    phones_shown: HashMap<u64, Instant>,
    /// Each kind's drawing, made once (see `mark`).
    marks: HashMap<kinds::Kind, view::Mark>,
}

async fn run(ctx: &mut ModuleContext) -> Result<(), String> {
    let settings = tokio::task::spawn_blocking(Settings::load).await.unwrap_or_default();
    let (internal_tx, mut internal) = mpsc::unbounded_channel();
    let (sys_tx, mut sys_rx) = mpsc::unbounded_channel();
    let (bt_tx, mut bt_rx) = mpsc::unbounded_channel();
    let (pad_tx, mut pad_rx) = mpsc::unbounded_channel();
    let (power_tx, mut power_rx) = mpsc::unbounded_channel();
    let (sighting_tx, mut sightings) = mpsc::unbounded_channel();
    let mut d = State {
        settings,
        internal: internal_tx.clone(),
        sightings: sighting_tx,
        events: None,
        bluetooth: None,
        pads_watch: None,
        _pads_events: None,
        power: None,
        moments: Moments::default(),
        gate: Gate::new(Instant::now()),
        registry: Registry::default(),
        connect: None,
        battery_reads: Vec::new(),
        battery_poll: None,
        reading_ids: false,
        burst: None,
        pads: HashMap::new(),
        pad_poll: None,
        has_battery: None,
        source: None,
        percent: None,
        laptop_low: LowBattery::default(),
        eta: Eta::default(),
        drives: None,
        refused: HashMap::new(),
        drive_card: None,
        rescans: Vec::new(),
        phones_shown: HashMap::new(),
        marks: HashMap::new(),
    };
    tokio::spawn(async move {
        let started = tokio::task::spawn_blocking(move || start(sys_tx, bt_tx, pad_tx, power_tx)).await;
        if let Ok(adapters) = started {
            _ = internal_tx.send(Internal::Started(adapters));
        }
    });
    d.rescan_drives(Duration::ZERO);
    let rehearsing = rehearse::enabled();
    let mut rehearsal = tokio::time::interval(rehearse::POLL);

    loop {
        let deadline = d.next_deadline();
        tokio::select! {
            Some(event) = sys_rx.recv() => d.on_system(ctx, event),
            Some(event) = bt_rx.recv() => d.on_bluetooth(ctx, event),
            Some(event) = pad_rx.recv() => d.on_pad(ctx, event),
            Some(event) = power_rx.recv() => d.on_power(ctx, event),
            Some(sighting) = sightings.recv() => d.on_sighting(ctx, sighting),
            Some(message) = internal.recv() => d.on_internal(ctx, message),
            _ = sleep_until(deadline.unwrap_or_else(Instant::now)), if deadline.is_some() => d.tick(ctx),
            _ = rehearsal.tick(), if rehearsing => {
                if rehearse::due().await {
                    d.rehearse_connect(ctx);
                }
            }
            input = ctx.next() => match input {
                Some(ModuleInput::Action(action)) => d.act(ctx, action),
                Some(_) => {}
                None => break,
            },
        }
    }
    // Unregistering can block on WinRT and the window thread; keep it off the hub thread.
    let held = (d.events.take(), d.bluetooth.take(), d._pads_events.take(), d.power.take(), d.burst.take());
    tokio::task::spawn_blocking(move || drop(held));
    Ok(())
}

/// Starts every adapter; each one failing only loses its own events.
fn start(
    sys: mpsc::UnboundedSender<SystemEvent>,
    bt: mpsc::UnboundedSender<BtEvent>,
    pads: mpsc::UnboundedSender<PadEvent>,
    power: mpsc::UnboundedSender<PowerEvent>,
) -> Adapters {
    let has_battery = power::has_battery();
    let events = system_events::subscribe(sys);
    let bluetooth = Watchers::start(bt).inspect_err(|e| eprintln!("devices: no Bluetooth watcher ({e}); no connect cards")).ok();
    let pads = Controllers::start(pads).inspect_err(|e| eprintln!("devices: no controller events ({e})")).ok();
    let power = if has_battery { PowerWatch::start(power).inspect_err(|e| eprintln!("devices: no battery reports ({e}); no time to full")).ok() } else { None };
    Adapters { events, bluetooth, pads, power, has_battery }
}

impl State {
    fn next_deadline(&self) -> Option<Instant> {
        let burst = self.burst.as_ref().map(|b| b.until);
        [self.moments.next_deadline(), self.battery_poll, self.pad_poll, burst].into_iter().flatten().chain(self.battery_reads.iter().copied()).chain(self.rescans.iter().copied()).min()
    }

    fn tick(&mut self, ctx: &ModuleContext) {
        let now = Instant::now();
        for key in self.moments.due(now) {
            ctx.retract(&key);
            self.forget(&key);
        }
        let reads_due = self.battery_reads.iter().any(|at| now >= *at) || self.battery_poll.is_some_and(|at| now >= at);
        if reads_due {
            self.battery_reads.retain(|at| now < *at);
            self.battery_poll = None;
            self.read_batteries();
        }
        if self.pad_poll.is_some_and(|at| now >= at) {
            self.pad_poll = None;
            self.poll_pads();
        }
        if self.burst.as_ref().is_some_and(|b| now >= b.until) {
            self.stop_scan();
        }
        if self.rescans.iter().any(|at| now >= *at) {
            self.rescans.retain(|at| now < *at);
            self.rescan_drives(Duration::ZERO);
        }
    }

    /// A moment went: drop what pointed at it.
    fn forget(&mut self, key: &str) {
        if self.connect.as_ref().is_some_and(|c| c.key == key) {
            self.connect = None;
        }
        if self.drive_card.as_ref().is_some_and(|c| c.key == key) {
            self.drive_card = None;
        }
        if key == OVERVIEW {
            self.stop_scan();
        }
    }

    fn show(&mut self, ctx: &ModuleContext, key: &str, priority: Priority, view: View, linger: Duration) {
        ctx.publish(key, priority, view);
        self.moments.show(key, Instant::now(), linger);
    }

    fn on_internal(&mut self, ctx: &ModuleContext, message: Internal) {
        match message {
            Internal::Started(a) => {
                (self.events, self.bluetooth, self.power) = (a.events, a.bluetooth, a.power);
                self.pads_watch = a.pads.as_ref().map(Controllers::shared);
                self._pads_events = a.pads;
                self.has_battery = Some(a.has_battery);
            }
            Internal::Settings(settings) => self.apply_settings(ctx, settings),
            Internal::Batteries(levels) => self.on_batteries(ctx, levels),
            Internal::Ids(read, found) => self.on_ids(ctx, &read, &found),
            Internal::Pads(readings) => {
                for r in readings {
                    self.on_pad_reading(ctx, r);
                }
                self.schedule_pad_poll();
                self.refresh_overview(ctx);
            }
            Internal::Drives(list) => self.on_drives(ctx, list),
            Internal::Ejected(letter, result) => self.on_ejected(ctx, letter, result),
        }
    }

    fn act(&mut self, ctx: &ModuleContext, action: ModuleAction) {
        let (key, a) = (action.key.as_str(), action.action.as_str());
        if key.is_empty() {
            if a == "overview" {
                self.open_overview(ctx);
                return;
            }
            // The Settings window wrote devices.json.
            if a == "reload" {
                self.reload_settings();
                return;
            }
            let mut settings = self.settings.clone();
            if settings.apply(a) {
                let saved = settings.clone();
                tokio::task::spawn_blocking(move || saved.save_to(&Settings::path()));
                self.apply_settings(ctx, settings);
            }
            return;
        }
        match a {
            ACTION_EXPANDED => {
                self.moments.opened(key);
                if key.starts_with("device:") {
                    self.read_batteries();
                }
            }
            ACTION_COLLAPSED => {
                let after = if key == OVERVIEW { Duration::from_millis(300) } else { moments::AFTER_CLOSE };
                self.moments.closed(key, Instant::now(), after);
            }
            // A moment without a card was clicked.
            ACTION_OPEN => {
                if key != OVERVIEW && self.moments.remove(key) {
                    ctx.retract(key);
                    self.forget(key);
                }
                self.open_overview(ctx);
            }
            _ => {
                if let Some(letter) = a.strip_prefix("eject:").and_then(|l| l.chars().next()) {
                    self.eject(letter);
                }
            }
        }
    }

    fn reload_settings(&self) {
        let tx = self.internal.clone();
        tokio::spawn(async move {
            if let Ok(settings) = tokio::task::spawn_blocking(Settings::load).await {
                _ = tx.send(Internal::Settings(settings));
            }
        });
    }

    fn apply_settings(&mut self, ctx: &ModuleContext, settings: Settings) {
        if settings == self.settings {
            return;
        }
        let old = std::mem::replace(&mut self.settings, settings);
        let retract_prefixed = |this: &mut Self, prefix: &str| {
            let keys: Vec<String> = this.moments.keys().filter(|k| k.starts_with(prefix)).cloned().collect();
            for k in keys {
                this.moments.remove(&k);
                ctx.retract(&k);
                this.forget(&k);
            }
        };
        if old.connect_cards && !self.settings.connect_cards {
            retract_prefixed(self, "device:");
        }
        if old.low_battery && !self.settings.low_battery {
            retract_prefixed(self, "low:");
        }
        if old.charging && !self.settings.charging {
            retract_prefixed(self, "power");
        }
        if old.drives && !self.settings.drives {
            retract_prefixed(self, "drive:");
        }
        if old.airpods && !self.settings.airpods {
            self.stop_scan();
        }
    }

    /// S9: the overview, opened from the tray or a moment. Fresh batteries, drives and (for
    /// AirPods) a scan come in while it's open.
    fn open_overview(&mut self, ctx: &ModuleContext) {
        self.read_batteries();
        self.poll_pads();
        self.rescan_drives(Duration::ZERO);
        self.reload_settings();
        let apple = self.registry.connected().find(|d| d.apple_audio()).map(|d| (d.key, d.expect()));
        if let Some((key, expect)) = apple {
            self.start_scan(key, expect);
        }
        let view = self.overview();
        self.show(ctx, OVERVIEW, Priority::Passive, view, OVERVIEW_UNOPENED);
        ctx.focus(OVERVIEW);
    }

    fn overview(&self) -> View {
        let devices: Vec<view::DeviceInfo> = self
            .registry
            .connected()
            .map(view::DeviceInfo::of)
            .chain(self.pads.values().map(|p| view::DeviceInfo { name: p.reading.name.clone(), kind: p.identity.kind, model: p.identity.name(), level: p.reading.level, pods: None }))
            .collect();
        let laptop = match (self.has_battery, self.percent) {
            (Some(true), Some(level)) => Some(view::Laptop { level: level.min(100) as u8, charging: self.source == Some(PowerSource::Ac) }),
            _ => None,
        };
        let drives: Vec<(Drive, Option<String>)> = self.drives.iter().flatten().map(|d| (d.clone(), self.refused.get(&d.letter).cloned())).collect();
        view::overview_view(&devices, laptop, &drives)
    }

    fn refresh_overview(&self, ctx: &ModuleContext) {
        if self.moments.is_live(OVERVIEW) {
            ctx.publish(OVERVIEW, Priority::Passive, self.overview());
        }
    }
}

#[cfg(test)]
mod quiet_tests {
    use super::*;

    #[test]
    fn each_kind_of_moment_maps_to_its_switch() {
        let setting = |key: &str| quiet(key, Priority::Brief).map(|q| q.setting.map(|s| (s.file, s.key)));
        assert_eq!(setting("device:00000000000000ff"), Some(Some(("devices.json".into(), "connect_cards".into()))));
        assert_eq!(setting("low:laptop"), Some(Some(("devices.json".into(), "low_battery".into()))));
        assert_eq!(setting("drive:E"), Some(Some(("devices.json".into(), "drives".into()))));
        assert_eq!(setting("power"), Some(Some(("devices.json".into(), "charging".into()))));
        assert_eq!(setting("ups"), Some(None), "snooze only");
        assert_eq!(setting(OVERVIEW), None, "the overview is a place, not a moment");
    }
}
