//! AirPods (and Beats) left / right / case batteries from Apple's "proximity pairing"
//! advertisements (manufacturer data, company 0x004C, message type 0x07), which the buds
//! broadcast while out of the case or with the lid open.
//!
//! Layout after the company id (hex nibbles as OpenPods and LibrePods read them):
//! `07 len 01 MM MM SS BB CF ..`: model (big-endian `MM MM`), status `SS` (bit 0x20 set =
//! the left bud is primary, otherwise the bud nibbles are swapped), buds `BB` (low nibble
//! left, high nibble right, when not swapped), `CF` (high nibble charging flags: 0x1 left,
//! 0x2 right (swapped with the buds), 0x4 case; low nibble the case). Levels are 0–10 in
//! tens of percent; 15 means "not available" (in the ear without the case, say).
//!
//! The scan is passive (no requests of our own), filtered to Apple 0x07 messages, and runs
//! only in bursts the module starts and stops (≤ 20 s, D4). Nothing is stored.

use tokio::sync::mpsc::UnboundedSender;
use windows::Devices::Bluetooth::Advertisement::{BluetoothLEAdvertisementFilter, BluetoothLEAdvertisementReceivedEventArgs, BluetoothLEAdvertisementWatcher, BluetoothLEManufacturerData, BluetoothLEScanningMode};
use windows::Foundation::TypedEventHandler;
use windows::Storage::Streams::{DataReader, DataWriter};

const APPLE: u16 = 0x004C;
const PROXIMITY_PAIRING: u8 = 0x07;
/// Ads weaker than this are someone else's buds (or ours in another room).
pub const NEAR_DBM: i16 = -60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pods {
    pub model: u16,
    pub left: Option<u8>,
    pub right: Option<u8>,
    pub case: Option<u8>,
    pub left_charging: bool,
    pub right_charging: bool,
    pub case_charging: bool,
}

impl Pods {
    /// The buds' lowest level (the one that runs out first).
    pub fn buds(&self) -> Option<u8> {
        match (self.left, self.right) {
            (Some(l), Some(r)) => Some(l.min(r)),
            (l, r) => l.or(r),
        }
    }

    /// Charging, as far as low-battery alerts go: every bud we can see is.
    pub fn buds_charging(&self) -> bool {
        let seen = [(self.left, self.left_charging), (self.right, self.right_charging)];
        seen.iter().any(|(l, _)| l.is_some()) && seen.iter().all(|(l, c)| l.is_none() || *c)
    }
}

fn level(nibble: u8) -> Option<u8> {
    (nibble <= 10).then_some(nibble * 10)
}

/// Parses a proximity-pairing message (manufacturer data without the company id).
pub fn parse(data: &[u8]) -> Option<Pods> {
    if data.len() < 8 || data[0] != PROXIMITY_PAIRING {
        return None;
    }
    let model = u16::from_be_bytes([data[3], data[4]]);
    let swapped = data[5] & 0x20 == 0;
    let (lo, hi) = (data[6] & 0x0F, data[6] >> 4);
    let (left, right) = if swapped { (hi, lo) } else { (lo, hi) };
    let flags = data[7] >> 4;
    let (left_flag, right_flag) = if swapped { (0x2, 0x1) } else { (0x1, 0x2) };
    Some(Pods {
        model,
        left: level(left),
        right: level(right),
        case: level(data[7] & 0x0F),
        left_charging: flags & left_flag != 0,
        right_charging: flags & right_flag != 0,
        case_charging: flags & 0x4 != 0,
    })
}

/// Model families, to accept only ads from the kind of buds that just connected (its name is
/// all we know about it; a renamed pair accepts any Apple model).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    AirPods,
    Pro,
    Max,
    Beats,
    Any,
}

pub fn family_of_name(name: &str) -> Family {
    let n = name.to_lowercase();
    if n.contains("beats") {
        Family::Beats
    } else if n.contains("airpods max") {
        Family::Max
    } else if n.contains("airpods pro") {
        Family::Pro
    } else if n.contains("airpods") {
        Family::AirPods
    } else {
        Family::Any
    }
}

pub fn family_of_model(model: u16) -> Family {
    match model {
        0x0220 | 0x0F20 | 0x1320 | 0x1920 | 0x1B20 => Family::AirPods,
        0x0E20 | 0x1420 | 0x2420 | 0x2720 => Family::Pro,
        0x0A20 | 0x1F20 => Family::Max,
        0x0320 | 0x0520 | 0x0620 | 0x0920 | 0x0B20 | 0x0C20 | 0x1020 | 0x1120 | 0x1220 | 0x1620 | 0x1720 | 0x1D20 => Family::Beats,
        _ => Family::Any,
    }
}

/// Whether an ad may describe the connected buds: near enough, and the same family.
pub fn accept(expected: Family, pods: &Pods, rssi: i16) -> bool {
    rssi >= NEAR_DBM && (expected == Family::Any || family_of_model(pods.model) == expected)
}

/// A reading from one ad.
#[derive(Debug, Clone, Copy)]
pub struct Sighting {
    pub pods: Pods,
    pub rssi: i16,
}

/// A running scan; dropping it stops.
pub struct Scan {
    watcher: BluetoothLEAdvertisementWatcher,
    token: i64,
}

impl Scan {
    pub fn start(tx: UnboundedSender<Sighting>) -> windows::core::Result<Self> {
        let writer = DataWriter::new()?;
        writer.WriteByte(PROXIMITY_PAIRING)?;
        let filter = BluetoothLEAdvertisementFilter::new()?;
        filter.Advertisement()?.ManufacturerData()?.Append(&BluetoothLEManufacturerData::Create(APPLE, &writer.DetachBuffer()?)?)?;
        let watcher = BluetoothLEAdvertisementWatcher::Create(&filter)?;
        watcher.SetScanningMode(BluetoothLEScanningMode::Passive)?;
        let token = watcher.Received(&TypedEventHandler::new(move |_, args: windows::core::Ref<BluetoothLEAdvertisementReceivedEventArgs>| {
            if let Some(args) = args.as_ref() {
                if let Some(sighting) = read(args) {
                    _ = tx.send(sighting);
                }
            }
            Ok(())
        }))?;
        if let Err(e) = watcher.Start() {
            _ = watcher.RemoveReceived(token);
            return Err(e);
        }
        Ok(Self { watcher, token })
    }
}

impl Drop for Scan {
    fn drop(&mut self) {
        _ = self.watcher.Stop();
        _ = self.watcher.RemoveReceived(self.token);
    }
}

fn read(args: &BluetoothLEAdvertisementReceivedEventArgs) -> Option<Sighting> {
    let rssi = args.RawSignalStrengthInDBm().ok()?;
    let sections = args.Advertisement().ok()?.GetManufacturerDataByCompanyId(APPLE).ok()?;
    for section in sections {
        let Ok(buffer) = section.Data() else { continue };
        let len = buffer.Length().unwrap_or(0) as usize;
        if !(8..=64).contains(&len) {
            continue;
        }
        let mut bytes = vec![0u8; len];
        if DataReader::FromBuffer(&buffer).and_then(|r| r.ReadBytes(&mut bytes)).is_err() {
            continue;
        }
        if let Some(pods) = parse(&bytes) {
            return Some(Sighting { pods, rssi });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Proximity-pairing messages as captured (company id stripped; the encrypted tail is
    /// zeroed here, it isn't read).
    fn payload(head: [u8; 9]) -> Vec<u8> {
        let mut v = head.to_vec();
        v.resize(27, 0);
        v
    }

    #[test]
    fn devices_airpods_pro_both_buds_in_the_case() {
        // AirPods Pro, left primary (0x75 has 0x20), buds 100 % / 100 %, both charging,
        // case 80 % not charging.
        let p = parse(&payload([0x07, 0x19, 0x01, 0x0E, 0x20, 0x75, 0xAA, 0x38, 0x31])).unwrap();
        assert_eq!(p.model, 0x0E20);
        assert_eq!(family_of_model(p.model), Family::Pro);
        assert_eq!((p.left, p.right, p.case), (Some(100), Some(100), Some(80)));
        assert!(p.left_charging && p.right_charging && !p.case_charging);
        assert!(p.buds_charging());
    }

    #[test]
    fn devices_airpods_primary_flip_and_missing_parts() {
        // Right bud primary (0x55 lacks 0x20): the bud nibbles and charging flags swap.
        // Buds byte 0x7F: one nibble 15 (not available); flags 0x5 = case + low-bit bud.
        let p = parse(&payload([0x07, 0x19, 0x01, 0x0F, 0x20, 0x55, 0x7F, 0x56, 0x00])).unwrap();
        assert_eq!(family_of_model(p.model), Family::AirPods);
        // Swapped: left = high nibble (7 → 70 %), right = low nibble (15 → none).
        assert_eq!((p.left, p.right), (Some(70), None));
        // Swapped flags: 0x1 is the right bud, 0x4 the case.
        assert!(!p.left_charging && p.right_charging && p.case_charging);
        assert_eq!(p.case, Some(60));
        assert_eq!(p.buds(), Some(70));
        assert!(!p.buds_charging());

        // Not swapped, same bytes otherwise: left = low nibble.
        let q = parse(&payload([0x07, 0x19, 0x01, 0x0F, 0x20, 0x75, 0x7F, 0x0F, 0x00])).unwrap();
        assert_eq!((q.left, q.right, q.case), (None, Some(70), None));
    }

    #[test]
    fn devices_airpods_rejects_other_messages_and_far_ads() {
        assert_eq!(parse(&[0x10, 0x05, 0x01, 0x18, 0x00, 0x00, 0x00, 0x00]), None); // "nearby" message
        assert_eq!(parse(&[0x07, 0x19, 0x01]), None);
        let p = parse(&payload([0x07, 0x19, 0x01, 0x14, 0x20, 0x75, 0x99, 0x09, 0x00])).unwrap();
        assert_eq!((p.left, p.right, p.case), (Some(90), Some(90), Some(90)));
        assert!(accept(Family::Pro, &p, -50));
        assert!(!accept(Family::Pro, &p, -75));
        assert!(!accept(Family::Max, &p, -40));
        assert!(accept(Family::Any, &p, -40));
        assert_eq!(family_of_name("Rohit's AirPods Pro"), Family::Pro);
        assert_eq!(family_of_name("AirPods Max"), Family::Max);
        assert_eq!(family_of_name("Studio Buds (Beats)"), Family::Beats);
        assert_eq!(family_of_name("Headphones"), Family::Any);
    }
}
