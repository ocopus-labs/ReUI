//! Bluetooth devices, their batteries, AirPods scans and game controllers, as the module
//! handles them: connect cards (upgraded in place when the battery arrives), the battery
//! read schedule, and low-battery moments.

use std::collections::HashMap;
use std::time::Duration;

use reui_protocol::{Art, Element, Priority};
use tokio::time::Instant;

use super::airpods::{self, Expect, Scan, Sighting};
use super::catalog::{self, DeviceIds, Known};
use super::bluetooth::BtEvent;
use super::controllers::{self, PadEvent, Reading};
use super::kinds::Kind;
use super::low::CRITICAL;
use super::registry::Change;
use super::spam::Verdict;
use super::{art, handoff, pnp, view, Burst, ConnectCard, State, Internal, Pad};
use crate::hub::ModuleContext;

/// A connect card's time (S1)...
const CONNECT: Duration = Duration::from_secs(3);
/// ...and at least this much more once its battery arrives.
const UPGRADED: Duration = Duration::from_secs(2);
/// A battery that arrives this long after the card no longer upgrades it (D-1).
const UPGRADE_WINDOW: Duration = Duration::from_secs(5);
const LOW: Duration = Duration::from_secs(4);
/// Battery reads after a connect (HFP reports only after a while, D-3)...
const READS_AFTER_CONNECT: [Duration; 2] = [Duration::from_secs(3), Duration::from_secs(10)];
/// ...then this often while a battery device is connected.
const BATTERY_POLL: Duration = Duration::from_secs(60);
const PAD_POLL: Duration = Duration::from_secs(60);
/// An AirPods advertisement scan (D4).
const BURST: Duration = Duration::from_secs(20);

fn device_key(key: u64) -> String {
    format!("device:{key:016x}")
}

impl State {
    pub(super) fn on_bluetooth(&mut self, ctx: &ModuleContext, event: BtEvent) {
        let now = Instant::now();
        let change = self.registry.apply(event);
        self.read_ids();
        match change {
            Some(Change::Connected(key)) => {
                self.battery_reads.extend(READS_AFTER_CONNECT.iter().map(|d| now + *d));
                let Some(device) = self.registry.get(key) else { return };
                let (kind, apple, expect) = (device.kind, device.apple_audio(), device.expect());
                if apple && self.settings.airpods {
                    self.start_scan(key, expect);
                }
                if kind.announces() {
                    let verdict = self.gate.connected(key, now);
                    if self.settings.connect_cards {
                        self.announce(ctx, key, verdict);
                    }
                }
                self.refresh_overview(ctx);
            }
            Some(Change::Disconnected(key)) => {
                self.gate.disconnected(&key, now);
                if self.burst.as_ref().is_some_and(|b| b.device == key) {
                    self.stop_scan();
                }
                self.refresh_overview(ctx);
            }
            None => {}
        }
    }

    /// Reads the Device ID records of containers not asked about yet, off the hub thread. At
    /// startup the watchers report every paired device within a moment; they share one read.
    fn read_ids(&mut self) {
        if self.reading_ids {
            return;
        }
        let containers = self.registry.ids_to_read();
        if containers.is_empty() {
            return;
        }
        self.reading_ids = true;
        let tx = self.internal.clone();
        tokio::spawn(async move {
            let read = containers.clone();
            let found = tokio::task::spawn_blocking(move || pnp::device_ids(&read)).await.unwrap_or_default();
            _ = tx.send(Internal::Ids(containers, found));
        });
    }

    /// Device ID records arrived: a device may now have an exact model (the connect card's
    /// detail) or turn out to be Apple audio (its batteries come from adverts).
    pub(super) fn on_ids(&mut self, ctx: &ModuleContext, read: &[u128], found: &HashMap<u128, DeviceIds>) {
        self.reading_ids = false;
        let changed = self.registry.set_ids(read, found);
        for key in &changed {
            let Some(device) = self.registry.get(*key).filter(|d| d.connected) else { continue };
            let (apple, expect) = (device.apple_audio(), device.expect());
            if apple && self.settings.airpods && self.burst.as_ref().is_none_or(|b| b.device == *key) {
                self.start_scan(*key, expect);
            }
            if self.connect.as_ref().is_some_and(|c| c.members.contains(key) && self.moments.is_live(&c.key)) {
                self.publish_connect(ctx, Duration::ZERO);
            }
        }
        if !changed.is_empty() {
            self.refresh_overview(ctx);
        }
        // Containers that appeared while this read ran.
        self.read_ids();
    }

    fn announce(&mut self, ctx: &ModuleContext, key: u64, verdict: Verdict<u64>) {
        match verdict {
            Verdict::Quiet => {}
            Verdict::Show => {
                let card = device_key(key);
                self.connect = Some(ConnectCard { key: card.clone(), members: vec![key], since: Instant::now() });
                self.publish_connect(ctx, CONNECT);
            }
            Verdict::Merge { into, members } => match &mut self.connect {
                Some(card) if card.members.first() == Some(&into) && self.moments.is_live(&card.key) => {
                    card.members = members;
                    self.publish_connect(ctx, Duration::ZERO);
                }
                // The first card already went: this one stands alone.
                _ => {
                    self.connect = Some(ConnectCard { key: device_key(key), members: vec![key], since: Instant::now() });
                    self.publish_connect(ctx, CONNECT);
                }
            },
        }
    }

    /// Publishes the connect card as it is now, keeping it at least `linger` longer.
    fn publish_connect(&mut self, ctx: &ModuleContext, linger: Duration) {
        let Some(card) = &self.connect else { return };
        let key = card.key.clone();
        let members = card.members.clone();
        let devices: Vec<_> = members.iter().filter_map(|k| self.registry.get(*k)).map(|d| (view::DeviceInfo::of(d), d.kind.is_audio())).collect();
        let Some((first, _)) = devices.first() else { return };
        let mark = self.mark(ctx, first.kind);
        let view = match devices.as_slice() {
            [(d, _)] => view::connect_view(d, &mark),
            many => view::merged_view(&mark, many.len()),
        };
        if devices.iter().any(|(_, audio)| *audio) {
            handoff::claim_audio_connect();
        }
        self.show(ctx, &key, Priority::Brief, view, linger);
    }

    /// A device's battery (or buds) changed: upgrade its card if it's new, check the
    /// thresholds.
    fn battery_changed(&mut self, ctx: &ModuleContext, key: u64) {
        let now = Instant::now();
        let upgrade = self.connect.as_ref().filter(|c| c.members == [key] && self.moments.is_live(&c.key)).map(|c| if now < c.since + UPGRADE_WINDOW { UPGRADED } else { Duration::ZERO });
        if let Some(linger) = upgrade {
            self.publish_connect(ctx, linger);
        }
        let Some(device) = self.registry.get_mut(key) else { return };
        let Some(level) = device.level() else { return };
        let charging = device.charging();
        if let Some(threshold) = device.low.update(level, charging) {
            let kind = device.kind;
            let mark = self.mark(ctx, kind);
            self.show_low(ctx, &format!("low:{key:016x}"), &mark, level, threshold);
        }
    }

    /// The drawing for a device of `kind` (ReUI's own, `design/assets/devices`), its glyph
    /// where there's none. Made once per kind: each drawing goes to the island once.
    pub(super) fn mark(&mut self, ctx: &ModuleContext, kind: Kind) -> view::Mark {
        if let Some(mark) = self.marks.get(&kind) {
            return mark.clone();
        }
        let logo = |small| art::svg(kind.id(), small).map(|svg| Element::Art(Art::Logo { image: ctx.image(svg.to_vec()), animated: false }));
        let mark = match (logo(true), logo(false)) {
            (Some(small), Some(full)) => view::Mark { small, full },
            _ => view::Mark::glyph(kind.glyph()),
        };
        self.marks.insert(kind, mark.clone());
        mark
    }

    pub(super) fn show_low(&mut self, ctx: &ModuleContext, key: &str, mark: &view::Mark, level: u8, threshold: u8) {
        if !self.settings.low_battery {
            return;
        }
        // Critical ones also show in "only when it needs me" mode (D8).
        let priority = if threshold <= CRITICAL { Priority::Alert } else { Priority::Brief };
        self.show(ctx, key, priority, view::low_view(mark, level), LOW);
    }

    pub(super) fn read_batteries(&self) {
        let containers: Vec<u128> = self.registry.connected().filter_map(|d| d.container).collect();
        if containers.is_empty() {
            return;
        }
        let tx = self.internal.clone();
        tokio::spawn(async move {
            if let Ok(levels) = tokio::task::spawn_blocking(move || pnp::batteries(&containers)).await {
                _ = tx.send(Internal::Batteries(levels));
            }
        });
    }

    pub(super) fn on_batteries(&mut self, ctx: &ModuleContext, levels: HashMap<u128, u8>) {
        let changed: Vec<u64> = self
            .registry
            .connected()
            .filter(|d| d.container.and_then(|c| levels.get(&c)).copied() != d.battery && d.container.is_some_and(|c| levels.contains_key(&c)))
            .map(|d| d.key)
            .collect();
        for key in changed {
            if let Some(device) = self.registry.get_mut(key) {
                // Only connected devices are read, so a stale value can't land here (D-2).
                device.battery = device.container.and_then(|c| levels.get(&c)).copied();
            }
            self.battery_changed(ctx, key);
        }
        let battery_device = self.registry.connected().any(|d| d.battery.is_some() || d.pods.is_some());
        self.battery_poll = battery_device.then(|| Instant::now() + BATTERY_POLL);
        self.refresh_overview(ctx);
    }

    // ---- AirPods -----------------------------------------------------------------------

    pub(super) fn start_scan(&mut self, device: u64, expect: Expect) {
        if !self.settings.airpods {
            return;
        }
        let until = Instant::now() + BURST;
        if let Some(b) = &mut self.burst {
            (b.device, b.expect, b.until) = (device, expect, until);
            return;
        }
        match Scan::start(self.sightings.clone()) {
            Ok(scan) => self.burst = Some(Burst { _scan: scan, device, expect, until }),
            Err(e) => eprintln!("devices: can't scan for AirPods batteries ({e})"),
        }
    }

    pub(super) fn stop_scan(&mut self) {
        if let Some(burst) = self.burst.take() {
            // Stopping a watcher can wait on the radio stack.
            tokio::task::spawn_blocking(move || drop(burst));
        }
    }

    pub(super) fn on_sighting(&mut self, ctx: &ModuleContext, s: Sighting) {
        let Some(burst) = &self.burst else { return };
        if !airpods::accept(burst.expect, &s.pods, s.rssi) {
            return;
        }
        let key = burst.device;
        let Some(device) = self.registry.get_mut(key).filter(|d| d.connected) else { return };
        if device.pods == Some(s.pods) {
            return;
        }
        device.pods = Some(s.pods);
        // The advert names the model (and its colour) when the ids didn't.
        device.identify();
        self.battery_changed(ctx, key);
        self.refresh_overview(ctx);
    }

    // ---- controllers -------------------------------------------------------------------

    pub(super) fn on_pad(&mut self, ctx: &ModuleContext, event: PadEvent) {
        match event {
            PadEvent::Added(reading) => {
                self.on_pad_reading(ctx, reading);
                self.schedule_pad_poll();
            }
            PadEvent::Removed(key) => {
                self.pads.remove(&key);
                if self.pads.is_empty() {
                    self.pad_poll = None;
                }
            }
        }
        self.refresh_overview(ctx);
    }

    pub(super) fn on_pad_reading(&mut self, ctx: &ModuleContext, reading: Reading) {
        let key = reading.key;
        let identity = catalog::resolve(&Known { ids: reading.ids, name: &reading.name, kind: Kind::Gamepad, ..Default::default() });
        let pad = self.pads.entry(key).or_insert_with(|| Pad { reading: reading.clone(), low: Default::default(), identity });
        pad.reading = reading;
        pad.identity = identity;
        let Some(level) = pad.reading.level else { return };
        if let Some(threshold) = pad.low.update(level, pad.reading.charging) {
            let mark = self.mark(ctx, Kind::Gamepad);
            self.show_low(ctx, &format!("low:pad:{key:016x}"), &mark, level, threshold);
        }
    }

    pub(super) fn schedule_pad_poll(&mut self) {
        self.pad_poll = self.pads.values().any(|p| p.reading.level.is_some()).then(|| Instant::now() + PAD_POLL);
    }

    pub(super) fn poll_pads(&self) {
        let Some(pads) = self.pads_watch.clone() else { return };
        let tx = self.internal.clone();
        tokio::spawn(async move {
            if let Ok(readings) = tokio::task::spawn_blocking(move || controllers::poll_shared(&pads)).await {
                _ = tx.send(Internal::Pads(readings));
            }
        });
    }
}
