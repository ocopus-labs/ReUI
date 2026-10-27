//! Paired Bluetooth devices: two `DeviceWatcher`s over association endpoints (Classic and
//! LE selectors, paired only), reading `System.Devices.Aep.IsConnected`, the `ContainerId`
//! that ties a dual-mode device's endpoints together, and the class (Classic Class of
//! Device, LE Appearance). Connects and disconnects arrive as `Updated`. The watchers are
//! cache-backed: no scanning and no connections of our own. Callbacks run on the WinRT
//! thread pool and only forward what they read.

use tokio::sync::mpsc::UnboundedSender;
use windows::core::{Interface, Ref, HSTRING, IInspectable};
use windows::Devices::Bluetooth::{BluetoothDevice, BluetoothLEDevice};
use windows::Devices::Enumeration::{DeviceInformation, DeviceInformationKind, DeviceInformationUpdate, DeviceWatcher};
use windows::Foundation::{IPropertyValue, PropertyType, TypedEventHandler};
use windows_collections::{IIterable, IMapView};

const IS_CONNECTED: &str = "System.Devices.Aep.IsConnected";
const CONTAINER: &str = "System.Devices.Aep.ContainerId";
const COD_MAJOR: &str = "System.Devices.Aep.Bluetooth.Cod.Major";
const COD_MINOR: &str = "System.Devices.Aep.Bluetooth.Cod.Minor";
const APPEARANCE: &str = "System.Devices.Aep.Bluetooth.Le.Appearance";
const NAME: &str = "System.ItemNameDisplay";

/// What one callback saw of an endpoint; `None` fields weren't in it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Endpoint {
    pub id: String,
    pub name: Option<String>,
    pub connected: Option<bool>,
    pub container: Option<u128>,
    pub cod: Option<(u32, u32)>,
    pub appearance: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BtEvent {
    /// Added or updated.
    Seen(Endpoint),
    /// Unpaired (or the radio went away).
    Removed(String),
}

/// The running watchers; dropping them stops both.
pub struct Watchers {
    watchers: Vec<(DeviceWatcher, [i64; 3])>,
}

impl Watchers {
    pub fn start(tx: UnboundedSender<BtEvent>) -> windows::core::Result<Self> {
        let mut watchers = Vec::new();
        for selector in [BluetoothDevice::GetDeviceSelectorFromPairingState(true)?, BluetoothLEDevice::GetDeviceSelectorFromPairingState(true)?] {
            match watch(&selector, &tx) {
                Ok(w) => watchers.push(w),
                Err(e) => eprintln!("devices: a Bluetooth watcher didn't start ({e})"),
            }
        }
        if watchers.is_empty() {
            return Err(windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL));
        }
        Ok(Self { watchers })
    }
}

impl Drop for Watchers {
    fn drop(&mut self) {
        for (w, [added, updated, removed]) in &self.watchers {
            _ = w.Stop();
            _ = w.RemoveAdded(*added);
            _ = w.RemoveUpdated(*updated);
            _ = w.RemoveRemoved(*removed);
        }
    }
}

fn watch(selector: &HSTRING, tx: &UnboundedSender<BtEvent>) -> windows::core::Result<(DeviceWatcher, [i64; 3])> {
    let props: IIterable<HSTRING> = vec![IS_CONNECTED, CONTAINER, COD_MAJOR, COD_MINOR, APPEARANCE, NAME].into_iter().map(HSTRING::from).collect::<Vec<_>>().into();
    let watcher = DeviceInformation::CreateWatcherWithKindAqsFilterAndAdditionalProperties(selector, &props, DeviceInformationKind::AssociationEndpoint)?;
    let (a, u, r) = (tx.clone(), tx.clone(), tx.clone());
    let added = watcher.Added(&TypedEventHandler::new(move |_, info: Ref<DeviceInformation>| {
        if let Some(info) = info.as_ref() {
            let mut e = read(info.Id()?, info.Properties().ok());
            e.name = info.Name().ok().map(|n| n.to_string()).filter(|n| !n.is_empty()).or(e.name);
            _ = a.send(BtEvent::Seen(e));
        }
        Ok(())
    }))?;
    let updated = watcher.Updated(&TypedEventHandler::new(move |_, update: Ref<DeviceInformationUpdate>| {
        if let Some(update) = update.as_ref() {
            _ = u.send(BtEvent::Seen(read(update.Id()?, update.Properties().ok())));
        }
        Ok(())
    }))?;
    let removed = watcher.Removed(&TypedEventHandler::new(move |_, update: Ref<DeviceInformationUpdate>| {
        if let Some(update) = update.as_ref() {
            _ = r.send(BtEvent::Removed(update.Id()?.to_string()));
        }
        Ok(())
    }))?;
    watcher.Start()?;
    Ok((watcher, [added, updated, removed]))
}

fn read(id: HSTRING, props: Option<IMapView<HSTRING, IInspectable>>) -> Endpoint {
    let mut e = Endpoint { id: id.to_string(), ..Default::default() };
    let Some(props) = props else { return e };
    let get = |key: &str| props.Lookup(&HSTRING::from(key)).ok().and_then(|v| v.cast::<IPropertyValue>().ok());
    e.connected = get(IS_CONNECTED).and_then(|v| v.GetBoolean().ok());
    e.container = get(CONTAINER).and_then(|v| v.GetGuid().ok()).map(|g| g.to_u128()).filter(|g| *g != 0);
    e.cod = get(COD_MAJOR).and_then(|v| number(&v)).map(|major| (major, get(COD_MINOR).and_then(|v| number(&v)).unwrap_or(0)));
    e.appearance = get(APPEARANCE).and_then(|v| number(&v)).filter(|a| *a != 0);
    e.name = get(NAME).and_then(|v| v.GetString().ok()).map(|s| s.to_string()).filter(|s| !s.is_empty());
    e
}

/// Windows types these properties differently across builds; any unsigned number will do.
fn number(v: &IPropertyValue) -> Option<u32> {
    match v.Type().ok()? {
        PropertyType::UInt8 => v.GetUInt8().ok().map(u32::from),
        PropertyType::UInt16 => v.GetUInt16().ok().map(u32::from),
        PropertyType::UInt32 => v.GetUInt32().ok(),
        PropertyType::Int16 => v.GetInt16().ok().and_then(|n| u32::try_from(n).ok()),
        PropertyType::Int32 => v.GetInt32().ok().and_then(|n| u32::try_from(n).ok()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lists the paired devices' kinds and connection states for a few seconds (read only;
    /// names and ids aren't printed). Run with `--ignored`.
    #[test]
    #[ignore = "reads the machine's paired Bluetooth devices"]
    fn devices_bluetooth_watch_reads() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let w = Watchers::start(tx).expect("watchers");
        std::thread::sleep(std::time::Duration::from_secs(3));
        drop(w);
        while let Ok(BtEvent::Seen(e)) = rx.try_recv() {
            let kind = super::super::kinds::classify(e.cod, e.appearance, e.name.as_deref().unwrap_or(""));
            println!("{kind:?} connected={:?} container={} cod={:?} appearance={:?}", e.connected, e.container.is_some(), e.cod, e.appearance);
        }
    }
}
