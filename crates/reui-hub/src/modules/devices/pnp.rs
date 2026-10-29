//! Configuration Manager (cfgmgr32) reads: device-node properties, parents and status, and
//! the Bluetooth battery level Windows shows in Settings. All blocking; call from the
//! blocking pool. Missing nodes and properties are normal (a device without a battery
//! service, one that just went away, D-5) and come back as `None`.

use std::collections::HashMap;

use windows::core::HSTRING;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_DevNode_PropertyW, CM_Get_DevNode_Status, CM_Get_Device_ID_ListW, CM_Get_Device_ID_List_SizeW, CM_Get_Parent, CM_Locate_DevNodeW, CM_DEVNODE_STATUS_FLAGS, CM_GETIDLIST_FILTER_ENUMERATOR,
    CM_GETIDLIST_FILTER_PRESENT, CM_LOCATE_DEVNODE_NORMAL, CM_PROB, CR_SUCCESS,
};
use windows::Win32::Devices::Properties::{DEVPKEY_Device_ContainerId, DEVPKEY_Device_HardwareIds, DEVPROPTYPE, DEVPROP_TYPE_BYTE, DEVPROP_TYPE_GUID, DEVPROP_TYPE_STRING_LIST, DEVPROP_TYPE_UINT16};
use windows::Win32::Foundation::DEVPROPKEY;

use super::catalog::DeviceIds;

/// The battery level Windows reads from hands-free (HFP) and LE battery services.
const DEVPKEY_BLUETOOTH_BATTERY: DEVPROPKEY = DEVPROPKEY { fmtid: windows::core::GUID::from_u128(0x104ea319_6ee2_4701_bd47_8ddbf425bbe5), pid: 2 };
/// The Device ID profile's record (`bthguid.h`: `DEVPKEY_Bluetooth_DeviceVIDSource` 6 (BYTE),
/// `DeviceVID` 7 and `DevicePID` 8 (UINT16)); defined here rather than pulling in the WDK
/// bindings for three keys.
const BLUETOOTH_DEVICE: u128 = 0x2bd67d8b_8beb_48d5_87e0_6cda3428040a;
const DEVPKEY_BLUETOOTH_VID_SOURCE: DEVPROPKEY = DEVPROPKEY { fmtid: windows::core::GUID::from_u128(BLUETOOTH_DEVICE), pid: 6 };
const DEVPKEY_BLUETOOTH_VID: DEVPROPKEY = DEVPROPKEY { fmtid: windows::core::GUID::from_u128(BLUETOOTH_DEVICE), pid: 7 };
const DEVPKEY_BLUETOOTH_PID: DEVPROPKEY = DEVPROPKEY { fmtid: windows::core::GUID::from_u128(BLUETOOTH_DEVICE), pid: 8 };

/// Enumerators whose nodes are a paired device's services: Classic profiles (`BTHENUM`),
/// hands-free audio (`BTHHFENUM`, where headsets report their battery), the LE device and
/// its GATT services (`BTHLE`, `BTHLEDEVICE`).
const BLUETOOTH_ENUMERATORS: [&str; 4] = ["BTHENUM", "BTHHFENUM", "BTHLE", "BTHLEDEVICE"];

pub fn property(devinst: u32, key: &DEVPROPKEY) -> Option<(DEVPROPTYPE, Vec<u8>)> {
    let mut kind = DEVPROPTYPE::default();
    let mut size = 0u32;
    // The first call only sizes (CR_BUFFER_SMALL).
    _ = unsafe { CM_Get_DevNode_PropertyW(devinst, key, &mut kind, None, &mut size, 0) };
    if size == 0 || size > 64 * 1024 {
        return None;
    }
    let mut buf = vec![0u8; size as usize];
    (unsafe { CM_Get_DevNode_PropertyW(devinst, key, &mut kind, Some(buf.as_mut_ptr()), &mut size, 0) } == CR_SUCCESS).then(|| {
        buf.truncate(size as usize);
        (kind, buf)
    })
}

pub fn locate(instance_id: &str) -> Option<u32> {
    let mut devinst = 0u32;
    (unsafe { CM_Locate_DevNodeW(&mut devinst, &HSTRING::from(instance_id), CM_LOCATE_DEVNODE_NORMAL) } == CR_SUCCESS).then_some(devinst)
}

pub fn parent(devinst: u32) -> Option<u32> {
    let mut p = 0u32;
    (unsafe { CM_Get_Parent(&mut p, devinst, 0) } == CR_SUCCESS).then_some(p)
}

/// `DN_*` status flags.
pub fn status(devinst: u32) -> Option<u32> {
    let (mut status, mut problem) = (CM_DEVNODE_STATUS_FLAGS::default(), CM_PROB::default());
    (unsafe { CM_Get_DevNode_Status(&mut status, &mut problem, devinst, 0) } == CR_SUCCESS).then_some(status.0)
}

/// Splits a NUL-separated, double-NUL-terminated list.
pub fn multi_sz(buf: &[u16]) -> Vec<String> {
    buf.split(|&c| c == 0).filter(|s| !s.is_empty()).map(String::from_utf16_lossy).collect()
}

/// Instance ids of the present devices under an enumerator.
fn instance_ids(enumerator: &str) -> Vec<String> {
    let filter = HSTRING::from(enumerator);
    let flags = CM_GETIDLIST_FILTER_ENUMERATOR | CM_GETIDLIST_FILTER_PRESENT;
    for _ in 0..3 {
        let mut len = 0u32;
        if unsafe { CM_Get_Device_ID_List_SizeW(&mut len, &filter, flags) } != CR_SUCCESS || len == 0 {
            return Vec::new();
        }
        let mut buf = vec![0u16; len as usize];
        // A device arriving in between makes the list longer than sized: try again.
        if unsafe { CM_Get_Device_ID_ListW(&filter, &mut buf, flags) } == CR_SUCCESS {
            return multi_sz(&buf);
        }
    }
    Vec::new()
}

fn guid(value: &(DEVPROPTYPE, Vec<u8>)) -> Option<u128> {
    let (kind, bytes) = value;
    if *kind != DEVPROP_TYPE_GUID || bytes.len() != 16 {
        return None;
    }
    let mut raw = [0u8; 16];
    raw.copy_from_slice(bytes);
    // In memory a GUID is (u32, u16, u16, [u8; 8]), little-endian fields.
    let g = windows::core::GUID::from_values(
        u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]),
        u16::from_le_bytes([raw[4], raw[5]]),
        u16::from_le_bytes([raw[6], raw[7]]),
        [raw[8], raw[9], raw[10], raw[11], raw[12], raw[13], raw[14], raw[15]],
    );
    Some(g.to_u128())
}

/// Battery levels by container: the lowest level any of a container's service nodes
/// reports. Containers without one are absent.
pub fn batteries(containers: &[u128]) -> HashMap<u128, u8> {
    let mut levels: HashMap<u128, u8> = HashMap::new();
    if containers.is_empty() {
        return levels;
    }
    for enumerator in BLUETOOTH_ENUMERATORS {
        for id in instance_ids(enumerator) {
            let Some(devinst) = locate(&id) else { continue };
            let Some(container) = property(devinst, &DEVPKEY_Device_ContainerId).as_ref().and_then(guid) else { continue };
            if !containers.contains(&container) {
                continue;
            }
            if let Some((kind, bytes)) = property(devinst, &DEVPKEY_BLUETOOTH_BATTERY) {
                if kind == DEVPROP_TYPE_BYTE && bytes.len() == 1 && bytes[0] <= 100 {
                    let level = levels.entry(container).or_insert(bytes[0]);
                    *level = (*level).min(bytes[0]);
                }
            }
        }
    }
    levels
}

/// The Device ID record by container: the devnode properties where Windows stored them (on
/// the device's own `BTHENUM\Dev_…` / `BTHLE\Dev_…` node), else the `VID&…_PID&…` in a
/// service node's hardware ids. Containers without one ("100 wireless" on the PC this was
/// written on) are absent.
pub fn device_ids(containers: &[u128]) -> HashMap<u128, DeviceIds> {
    let mut found: HashMap<u128, DeviceIds> = HashMap::new();
    let mut from_properties: Vec<u128> = Vec::new();
    if containers.is_empty() {
        return found;
    }
    for enumerator in BLUETOOTH_ENUMERATORS {
        for id in instance_ids(enumerator) {
            let Some(devinst) = locate(&id) else { continue };
            let Some(container) = property(devinst, &DEVPKEY_Device_ContainerId).as_ref().and_then(guid) else { continue };
            if !containers.contains(&container) || from_properties.contains(&container) {
                continue;
            }
            // The properties are authoritative; a hardware id only fills in until one is seen.
            if let Some(ids) = ids_from_properties(devinst) {
                found.insert(container, ids);
                from_properties.push(container);
            } else if let Some(ids) = hardware_ids(devinst).iter().find_map(|h| DeviceIds::from_hardware_id(h)) {
                found.entry(container).or_insert(ids);
            }
        }
    }
    found
}

fn ids_from_properties(devinst: u32) -> Option<DeviceIds> {
    let byte = |key| property(devinst, key).filter(|(kind, b)| *kind == DEVPROP_TYPE_BYTE && b.len() == 1).map(|(_, b)| b[0]);
    let word = |key| property(devinst, key).filter(|(kind, b)| *kind == DEVPROP_TYPE_UINT16 && b.len() == 2).map(|(_, b)| u16::from_le_bytes([b[0], b[1]]));
    DeviceIds::from_properties(byte(&DEVPKEY_BLUETOOTH_VID_SOURCE)?, word(&DEVPKEY_BLUETOOTH_VID)?, word(&DEVPKEY_BLUETOOTH_PID)?)
}

fn hardware_ids(devinst: u32) -> Vec<String> {
    match property(devinst, &DEVPKEY_Device_HardwareIds) {
        Some((kind, bytes)) if kind == DEVPROP_TYPE_STRING_LIST => {
            let wide: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            multi_sz(&wide)
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_multi_sz_lists() {
        let mut buf: Vec<u16> = "A\\1".encode_utf16().collect();
        buf.push(0);
        buf.extend("B\\2".encode_utf16());
        buf.extend([0, 0]);
        assert_eq!(multi_sz(&buf), ["A\\1", "B\\2"]);
        assert!(multi_sz(&[0, 0]).is_empty());
        let g = windows::core::GUID::from_u128(0x8c7ed206_3f8a_4827_b3ab_ae9e1faefc6c);
        let bytes = unsafe { std::slice::from_raw_parts((&g as *const windows::core::GUID).cast::<u8>(), 16) }.to_vec();
        assert_eq!(guid(&(DEVPROP_TYPE_GUID, bytes)), Some(g.to_u128()));
        assert_eq!(guid(&(DEVPROP_TYPE_BYTE, vec![1])), None);
    }

    /// Reads the battery levels of every paired container (read only). Run with `--ignored`.
    #[test]
    #[ignore = "reads the machine's Bluetooth device nodes"]
    fn devices_pnp_battery_reads() {
        let mut containers = Vec::new();
        for e in BLUETOOTH_ENUMERATORS {
            for id in instance_ids(e) {
                if let Some(c) = locate(&id).and_then(|d| property(d, &DEVPKEY_Device_ContainerId)).as_ref().and_then(guid) {
                    containers.push(c);
                }
            }
        }
        let levels = batteries(&containers);
        println!("{} containers, levels: {:?}", containers.len(), levels.values().collect::<Vec<_>>());
    }

    /// What each paired device resolves to: its Device ID record, class and the catalog's
    /// answer (read only; prints the devices' names, to this terminal only). Run with
    /// `--ignored --nocapture`.
    #[test]
    #[ignore = "reads the machine's paired Bluetooth devices"]
    fn devices_pnp_ids_resolve() {
        use super::super::bluetooth::{BtEvent, Watchers};
        use super::super::catalog;
        use super::super::registry::Registry;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let w = Watchers::start(tx).expect("watchers");
        std::thread::sleep(std::time::Duration::from_secs(3));
        drop(w);
        let mut registry = Registry::default();
        while let Ok(e) = rx.try_recv() {
            if matches!(e, BtEvent::Seen(_)) {
                registry.apply(e);
            }
        }
        let containers: Vec<u128> = registry.all().filter_map(|d| d.container).collect();
        let ids = device_ids(&containers);
        for d in registry.all() {
            let ids = d.container.and_then(|c| ids.get(&c).copied());
            let lookup = ids.map(catalog::lookup);
            let identity = catalog::resolve(&catalog::Known { ids, name: &d.name, kind: d.kind, ..Default::default() });
            println!(
                "{:?}: ids {ids:?} → {lookup:?}; resolves to model {:?}, family {:?}, kind {}, by {:?}",
                d.name,
                identity.name(),
                identity.family.map(|f| f.id()),
                identity.kind.id(),
                identity.by
            );
        }
    }
}
