//! The Bluetooth Device ID profile's vendor and product ids, as Windows stores them: the
//! devnode properties `DEVPKEY_Bluetooth_DeviceVIDSource` / `VID` / `PID` (`bthguid.h`), and
//! the same values inside hardware ids, `BTHENUM\{uuid}_VID&0001004C_PID&2014` (Classic:
//! four hex digits of source, four of vendor) or `BTHLEDevice\{uuid}_Dev_VID&02045e_PID&0b13…`
//! (LE: two of source). Pure.

/// Who assigned the vendor id (`VendorIdSource`: 1 = Bluetooth SIG company id, 2 = USB-IF
/// vendor id; Microsoft's `System.DeviceInterface.Bluetooth.VendorIdSource` docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VidSource {
    Bluetooth,
    Usb,
}

impl VidSource {
    pub fn from_raw(raw: u32) -> Option<VidSource> {
        match raw {
            1 => Some(VidSource::Bluetooth),
            2 => Some(VidSource::Usb),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeviceIds {
    pub source: VidSource,
    pub vid: u16,
    pub pid: u16,
}

impl DeviceIds {
    /// From the three devnode properties; a zero vendor id means "not set".
    pub fn from_properties(source: u8, vid: u16, pid: u16) -> Option<DeviceIds> {
        let source = VidSource::from_raw(u32::from(source))?;
        (vid != 0).then_some(DeviceIds { source, vid, pid })
    }

    /// From a hardware id, when it carries `VID&…_PID&…`.
    pub fn from_hardware_id(id: &str) -> Option<DeviceIds> {
        let upper = id.to_ascii_uppercase();
        let start = upper.find("VID&")? + 4;
        let vid_field: String = upper[start..].chars().take_while(char::is_ascii_hexdigit).collect();
        let rest = &upper[start + vid_field.len()..];
        let pid_field: String = rest.strip_prefix("_PID&")?.chars().take_while(char::is_ascii_hexdigit).collect();
        // The vendor field is the source then four digits of vendor id.
        if !(5..=8).contains(&vid_field.len()) || pid_field.len() != 4 {
            return None;
        }
        let (source, vid) = vid_field.split_at(vid_field.len() - 4);
        let source = VidSource::from_raw(u32::from_str_radix(source, 16).ok()?)?;
        let vid = u16::from_str_radix(vid, 16).ok()?;
        let pid = u16::from_str_radix(&pid_field, 16).ok()?;
        (vid != 0).then_some(DeviceIds { source, vid, pid })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_ids_from_hardware_ids() {
        // Observed on the "Buds 3r" (Bluetrum SIG id) and in public AirPods Pro 2 listings.
        assert_eq!(
            DeviceIds::from_hardware_id(r"BTHENUM\{0000110b-0000-1000-8000-00805f9b34fb}_VID&00010642_PID&0220"),
            Some(DeviceIds { source: VidSource::Bluetooth, vid: 0x0642, pid: 0x0220 })
        );
        assert_eq!(
            DeviceIds::from_hardware_id(r"BTHENUM\{0000110b-0000-1000-8000-00805f9b34fb}_VID&0001004C_PID&2014"),
            Some(DeviceIds { source: VidSource::Bluetooth, vid: 0x004C, pid: 0x2014 })
        );
        // An LE controller: two digits of source, a USB-IF vendor.
        assert_eq!(
            DeviceIds::from_hardware_id(r"BTHLEDevice\{00001812-0000-1000-8000-00805f9b34fb}_Dev_VID&02045e_PID&0b13_REV&0509"),
            Some(DeviceIds { source: VidSource::Usb, vid: 0x045E, pid: 0x0B13 })
        );
        // No Device ID record ("100 wireless"), a "local manufacturer" id, junk.
        assert_eq!(DeviceIds::from_hardware_id(r"BTHENUM\Dev_377313131CAA"), None);
        assert_eq!(DeviceIds::from_hardware_id(r"BTHENUM\{0000110b-0000-1000-8000-00805f9b34fb}_LOCALMFG&005d"), None);
        assert_eq!(DeviceIds::from_hardware_id("VID&0003004C_PID&2014"), None);
        assert_eq!(DeviceIds::from_hardware_id("VID&00010000_PID&2014"), None);
        assert_eq!(DeviceIds::from_hardware_id("VID&0001004C_PID&20"), None);
    }

    #[test]
    fn devices_ids_from_properties() {
        assert_eq!(DeviceIds::from_properties(1, 0x0642, 0x0220), Some(DeviceIds { source: VidSource::Bluetooth, vid: 0x0642, pid: 0x0220 }));
        assert_eq!(DeviceIds::from_properties(2, 0x054C, 0x0CE6).map(|i| i.source), Some(VidSource::Usb));
        assert_eq!(DeviceIds::from_properties(0, 0x004C, 0x2014), None);
        assert_eq!(DeviceIds::from_properties(1, 0, 0), None);
    }
}
