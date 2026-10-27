//! The Bluetooth devices as the user thinks of them: endpoints grouped by `ContainerId`, so
//! a dual-mode device (AirPods, most headsets: a Classic and an LE endpoint) is one device,
//! connected while any of its endpoints is. Pure; fed by the watcher's events.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use super::airpods::Pods;
use super::bluetooth::{BtEvent, Endpoint};
use super::kinds::{self, Kind};
use super::low::LowBattery;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Connected(u64),
    Disconnected(u64),
}

#[derive(Debug, Clone, Default)]
struct EndpointState {
    device: u64,
    name: Option<String>,
    connected: bool,
    cod: Option<(u32, u32)>,
    appearance: Option<u32>,
}

/// One device. `key` is a per-session hash (of the container, or of the endpoint id
/// without one): it names activities and is the only thing ever logged.
#[derive(Debug, Clone, Default)]
pub struct Device {
    pub key: u64,
    pub container: Option<u128>,
    pub name: String,
    pub kind: Kind,
    pub connected: bool,
    /// From the PnP battery property; `None` when unknown (never a placeholder).
    pub battery: Option<u8>,
    /// From AirPods advertisements, when read.
    pub pods: Option<Pods>,
    pub low: LowBattery,
}

impl Device {
    /// The level that matters: the buds' lowest when read, else the PnP value.
    pub fn level(&self) -> Option<u8> {
        self.pods.and_then(|p| p.buds()).or(self.battery)
    }

    pub fn charging(&self) -> bool {
        self.pods.is_some_and(|p| p.buds_charging())
    }

    pub fn apple_audio(&self) -> bool {
        kinds::is_apple_audio(&self.name, self.kind)
    }
}

pub fn hash<T: Hash>(value: T) -> u64 {
    let mut h = DefaultHasher::new();
    value.hash(&mut h);
    h.finish()
}

#[derive(Debug, Default)]
pub struct Registry {
    endpoints: HashMap<String, EndpointState>,
    devices: HashMap<u64, Device>,
}

impl Registry {
    pub fn apply(&mut self, event: BtEvent) -> Option<Change> {
        match event {
            BtEvent::Seen(e) => self.seen(e),
            BtEvent::Removed(id) => {
                let ep = self.endpoints.remove(&id)?;
                let change = self.refresh(ep.device);
                if !self.endpoints.values().any(|e| e.device == ep.device) {
                    self.devices.remove(&ep.device);
                }
                change
            }
        }
    }

    fn seen(&mut self, e: Endpoint) -> Option<Change> {
        let key = match self.endpoints.get(&e.id) {
            Some(known) => known.device,
            None => e.container.map_or_else(|| hash(&e.id), hash),
        };
        let ep = self.endpoints.entry(e.id).or_insert_with(|| EndpointState { device: key, ..Default::default() });
        ep.name = e.name.or(ep.name.take());
        ep.connected = e.connected.unwrap_or(ep.connected);
        ep.cod = e.cod.or(ep.cod);
        ep.appearance = e.appearance.or(ep.appearance);
        self.devices.entry(key).or_insert_with(|| Device { key, container: e.container, ..Default::default() });
        self.refresh(key)
    }

    /// Recomputes a device from its endpoints; says whether it (dis)connected.
    fn refresh(&mut self, key: u64) -> Option<Change> {
        let device = self.devices.get_mut(&key)?;
        let endpoints: Vec<&EndpointState> = self.endpoints.values().filter(|e| e.device == key).collect();
        let connected = endpoints.iter().any(|e| e.connected);
        // The Classic endpoint (it has a class of device) usually carries the friendly name.
        let mut named: Vec<&&EndpointState> = endpoints.iter().filter(|e| e.name.is_some()).collect();
        named.sort_by_key(|e| e.cod.is_none());
        if let Some(name) = named.first().and_then(|e| e.name.clone()) {
            device.name = name;
        }
        let cod = endpoints.iter().find_map(|e| e.cod);
        let appearance = endpoints.iter().find_map(|e| e.appearance);
        device.kind = kinds::classify(cod, appearance, &device.name);
        let before = std::mem::replace(&mut device.connected, connected);
        match (before, connected) {
            (false, true) => Some(Change::Connected(key)),
            (true, false) => {
                // Stale values must not outlive the connection (D-2).
                device.battery = None;
                device.pods = None;
                Some(Change::Disconnected(key))
            }
            _ => None,
        }
    }

    pub fn get(&self, key: u64) -> Option<&Device> {
        self.devices.get(&key)
    }

    pub fn get_mut(&mut self, key: u64) -> Option<&mut Device> {
        self.devices.get_mut(&key)
    }

    pub fn connected(&self) -> impl Iterator<Item = &Device> {
        self.devices.values().filter(|d| d.connected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(id: &str, connected: Option<bool>, container: Option<u128>) -> BtEvent {
        BtEvent::Seen(Endpoint { id: id.into(), connected, container, ..Default::default() })
    }

    #[test]
    fn devices_dual_mode_is_one_device() {
        let mut r = Registry::default();
        let classic = Endpoint { id: "classic".into(), name: Some("Rohit's AirPods Pro".into()), connected: Some(false), container: Some(7), cod: Some((4, 1)), appearance: None };
        let le = Endpoint { id: "le".into(), name: Some("LE-AirPods".into()), connected: Some(false), container: Some(7), cod: None, appearance: None };
        assert_eq!(r.apply(BtEvent::Seen(le)), None);
        assert_eq!(r.apply(BtEvent::Seen(classic)), None);
        let key = hash(7u128);
        let d = r.get(key).unwrap();
        assert_eq!((d.name.as_str(), d.kind), ("Rohit's AirPods Pro", Kind::Earbuds));
        assert!(d.apple_audio());

        // Either endpoint connecting connects the device, once.
        assert_eq!(r.apply(seen("le", Some(true), None)), Some(Change::Connected(key)));
        assert_eq!(r.apply(seen("classic", Some(true), None)), None);
        r.get_mut(key).unwrap().battery = Some(80);
        assert_eq!(r.apply(seen("le", Some(false), None)), None);
        assert_eq!(r.connected().count(), 1);
        assert_eq!(r.apply(seen("classic", Some(false), None)), Some(Change::Disconnected(key)));
        assert_eq!(r.get(key).unwrap().battery, None);
    }

    #[test]
    fn devices_without_a_container_and_removal() {
        let mut r = Registry::default();
        assert_eq!(r.apply(seen("mouse", Some(true), None)), Some(Change::Connected(hash("mouse"))));
        // Updates without the property keep the last state.
        assert_eq!(r.apply(seen("mouse", None, None)), None);
        assert_eq!(r.apply(BtEvent::Removed("mouse".into())), Some(Change::Disconnected(hash("mouse"))));
        assert!(r.get(hash("mouse")).is_none());
        assert_eq!(r.apply(BtEvent::Removed("never-seen".into())), None);
    }
}
