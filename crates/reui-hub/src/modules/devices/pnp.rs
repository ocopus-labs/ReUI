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
use windows::Win32::Devices::Properties::{DEVPKEY_Device_ContainerId, DEVPROPTYPE, DEVPROP_TYPE_BYTE, DEVPROP_TYPE_GUID};
use windows::Win32::Foundation::DEVPROPKEY;

/// The battery level Windows reads from hands-free (HFP) and LE battery services.
const DEVPKEY_BLUETOOTH_BATTERY: DEVPROPKEY = DEVPROPKEY { fmtid: windows::core::GUID::from_u128(0x104ea319_6ee2_4701_bd47_8ddbf425bbe5), pid: 2 };

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
fn device_ids(enumerator: &str) -> Vec<String> {
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
        for id in device_ids(enumerator) {
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
            for id in device_ids(e) {
                if let Some(c) = locate(&id).and_then(|d| property(d, &DEVPKEY_Device_ContainerId)).as_ref().and_then(guid) {
                    containers.push(c);
                }
            }
        }
        let levels = batteries(&containers);
        println!("{} containers, levels: {:?}", containers.len(), levels.values().collect::<Vec<_>>());
    }
}
