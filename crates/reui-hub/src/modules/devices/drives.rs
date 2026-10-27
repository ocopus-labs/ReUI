//! External drives: which mounted volumes count (removable media, and fixed disks on an
//! external bus such as USB or SD; never the system drive, mounted ISO images (CD-ROM) or
//! VHDs (a virtual bus), D6), with their label and space. Blocking; called from the blocking
//! pool after a volume arrives or leaves, and when the overview opens.

use windows::core::HSTRING;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW, BusType1394, BusTypeMmc, BusTypeSd, BusTypeUsb, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING, STORAGE_BUS_TYPE,
};
use windows::Win32::System::Ioctl::{PropertyStandardQuery, StorageDeviceProperty, IOCTL_STORAGE_QUERY_PROPERTY, STORAGE_DEVICE_DESCRIPTOR, STORAGE_PROPERTY_QUERY};
use windows::Win32::System::IO::DeviceIoControl;

const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drive {
    pub letter: char,
    pub label: String,
    pub free: u64,
    pub total: u64,
    /// An SD / MMC card (its own glyph).
    pub card: bool,
}

/// Whether a volume is one the user plugged in.
pub fn is_external(drive_type: u32, bus: STORAGE_BUS_TYPE, system: bool) -> bool {
    if system {
        return false;
    }
    let external_bus = [BusTypeUsb, BusTypeSd, BusTypeMmc, BusType1394].contains(&bus);
    match drive_type {
        DRIVE_REMOVABLE => true,
        DRIVE_FIXED => external_bus,
        _ => false,
    }
}

/// "12.4 GB", "980 MB", "1.8 TB" (decimal units, as Explorer's drive bars round).
pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 || value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        let text = format!("{value:.1}");
        format!("{} {}", text.strip_suffix(".0").unwrap_or(&text), UNITS[unit])
    }
}

/// "12.4 GB free of 64 GB".
pub fn space(drive: &Drive) -> String {
    format!("{} free of {}", size(drive.free), size(drive.total))
}

/// What a drive is called: its label, else what kind it is.
pub fn name(drive: &Drive) -> String {
    match drive.label.trim() {
        "" if drive.card => "SD Card".into(),
        "" => "USB Drive".into(),
        label => label.to_owned(),
    }
}

/// Opens `\\.\E:` without read or write access (enough for storage queries).
pub fn open_volume(letter: char) -> Option<HANDLE> {
    let path = HSTRING::from(format!(r"\\.\{letter}:"));
    unsafe { CreateFileW(&path, 0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None) }.ok()
}

fn bus_type(letter: char) -> Option<STORAGE_BUS_TYPE> {
    let handle = open_volume(letter)?;
    let query = STORAGE_PROPERTY_QUERY { PropertyId: StorageDeviceProperty, QueryType: PropertyStandardQuery, ..Default::default() };
    // u64 storage keeps the descriptor aligned; it's followed by variable-length strings.
    let mut buf = [0u64; 128];
    let mut returned = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            Some((&query as *const STORAGE_PROPERTY_QUERY).cast()),
            size_of::<STORAGE_PROPERTY_QUERY>() as u32,
            Some(buf.as_mut_ptr().cast()),
            size_of_val(&buf) as u32,
            Some(&mut returned),
            None,
        )
    };
    _ = unsafe { CloseHandle(handle) };
    if ok.is_err() || (returned as usize) < std::mem::offset_of!(STORAGE_DEVICE_DESCRIPTOR, RawPropertiesLength) {
        return None;
    }
    // SAFETY: the buffer holds at least the fixed part of the descriptor.
    Some(unsafe { (*(buf.as_ptr() as *const STORAGE_DEVICE_DESCRIPTOR)).BusType })
}

fn system_letter() -> Option<char> {
    std::env::var("SystemDrive").ok()?.chars().next().map(|c| c.to_ascii_uppercase())
}

/// The external drives with media in them, by letter.
pub fn scan() -> Vec<Drive> {
    let mask = unsafe { GetLogicalDrives() };
    let system = system_letter();
    let mut drives = Vec::new();
    for (i, letter) in ('A'..='Z').enumerate() {
        if mask & (1 << i) == 0 {
            continue;
        }
        let root = HSTRING::from(format!(r"{letter}:\"));
        let drive_type = unsafe { GetDriveTypeW(&root) };
        if drive_type != DRIVE_REMOVABLE && drive_type != DRIVE_FIXED {
            continue;
        }
        let bus = bus_type(letter).unwrap_or_default();
        if !is_external(drive_type, bus, system == Some(letter)) {
            continue;
        }
        // An empty card-reader slot has no volume information (D-10).
        let mut label = [0u16; 261];
        if unsafe { GetVolumeInformationW(&root, Some(&mut label), None, None, None, None) }.is_err() {
            continue;
        }
        let (mut free, mut total) = (0u64, 0u64);
        _ = unsafe { GetDiskFreeSpaceExW(&root, Some(&mut free), Some(&mut total), None) };
        let len = label.iter().position(|&c| c == 0).unwrap_or(label.len());
        drives.push(Drive { letter, label: String::from_utf16_lossy(&label[..len]), free, total, card: bus == BusTypeSd || bus == BusTypeMmc });
    }
    drives
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Storage::FileSystem::{BusTypeFileBackedVirtual, BusTypeNvme, BusTypeSata};

    #[test]
    fn devices_which_drives_count() {
        assert!(is_external(DRIVE_REMOVABLE, BusTypeUsb, false));
        assert!(is_external(DRIVE_REMOVABLE, BusTypeSd, false));
        // USB SSDs report themselves as fixed disks.
        assert!(is_external(DRIVE_FIXED, BusTypeUsb, false));
        assert!(!is_external(DRIVE_FIXED, BusTypeNvme, false));
        assert!(!is_external(DRIVE_FIXED, BusTypeSata, false));
        // A mounted VHD, and the system drive whatever its bus (Windows To Go).
        assert!(!is_external(DRIVE_FIXED, BusTypeFileBackedVirtual, false));
        assert!(!is_external(DRIVE_REMOVABLE, BusTypeUsb, true));
        // ISO images mount as CD-ROMs (5).
        assert!(!is_external(5, BusTypeUsb, false));
    }

    #[test]
    fn devices_drive_sizes_and_names() {
        assert_eq!(size(12_400_000_000), "12.4 GB");
        assert_eq!(size(64_000_000_000), "64 GB");
        assert_eq!(size(980_000_000), "980 MB");
        assert_eq!(size(1_830_000_000_000), "1.8 TB");
        assert_eq!(size(512), "512 B");
        let d = Drive { letter: 'E', label: String::new(), free: 12_400_000_000, total: 64_000_000_000, card: false };
        assert_eq!(space(&d), "12.4 GB free of 64 GB");
        assert_eq!(name(&d), "USB Drive");
        assert_eq!(name(&Drive { card: true, ..d.clone() }), "SD Card");
        assert_eq!(name(&Drive { label: "BACKUP".into(), ..d }), "BACKUP");
    }

    /// Lists the external drives (read only). Run with `--ignored`.
    #[test]
    #[ignore = "reads the machine's drives"]
    fn devices_drive_scan_reads() {
        for d in scan() {
            println!("{}: card={} {}", d.letter, d.card, space(&d));
        }
    }
}
