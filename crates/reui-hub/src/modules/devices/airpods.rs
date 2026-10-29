//! AirPods (and Beats) left / right / case batteries, lid and colour from Apple's "proximity
//! pairing" advertisements (manufacturer data, company 0x004C, message type 0x07), which the
//! buds broadcast while out of the case or with the lid open.
//!
//! Layout after the company id, as the community reads it ([RE]; the research doc §2.1):
//! `07 len 01 MM MM SS BB CF LL CC ..`
//! - `MM MM` the product id, little-endian (`14 20` = 0x2014, AirPods Pro 2): the same id
//!   the Device ID record carries, so it keys the catalog directly;
//! - `SS` status: bit 0x20 set = the left bud is primary, otherwise the bud nibbles and
//!   charging flags are swapped;
//! - `BB` buds (low nibble left, high nibble right, when not swapped);
//! - `CF` high nibble charging flags (0x1 left, 0x2 right, swapped with the buds; 0x4 case),
//!   low nibble the case;
//! - `LL` lid: bit 0x08 clear = open, set = closed (the other bits count open/close cycles).
//!   Only a bud inside the case sees the lid, so without a case level it means nothing;
//! - `CC` colour (`catalog::Colour::from_apple`);
//! - the rest is encrypted (exact percentages; the key only comes over Apple's own protocol)
//!   and isn't read.
//!
//! Levels are 0–10 in tens of percent; 15 means "not available" (in the ear without the
//! case, say). AirPods 4 send these too (they're in the catalog; PRD D-6).
//!
//! The scan is passive (no requests of our own), filtered to Apple 0x07 messages, and runs
//! only in bursts the module starts and stops (≤ 20 s, D4). Nothing is stored.

use tokio::sync::mpsc::UnboundedSender;
use windows::Devices::Bluetooth::Advertisement::{BluetoothLEAdvertisementFilter, BluetoothLEAdvertisementReceivedEventArgs, BluetoothLEAdvertisementWatcher, BluetoothLEManufacturerData, BluetoothLEScanningMode};
use windows::Foundation::TypedEventHandler;
use windows::Storage::Streams::{DataReader, DataWriter};

use super::catalog::{self, Colour, DeviceIds, Family, VidSource};

const APPLE: u16 = catalog::APPLE;
const PROXIMITY_PAIRING: u8 = 0x07;
/// Ads weaker than this are someone else's buds (or ours in another room).
pub const NEAR_DBM: i16 = -60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pods {
    /// The product id (0x2014 for AirPods Pro 2).
    pub model: u16,
    pub left: Option<u8>,
    pub right: Option<u8>,
    pub case: Option<u8>,
    pub left_charging: bool,
    pub right_charging: bool,
    pub case_charging: bool,
    /// Whether the case's lid is open, when a bud in the case can tell.
    pub lid_open: Option<bool>,
    pub colour: Option<Colour>,
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
    let model = u16::from_le_bytes([data[3], data[4]]);
    let swapped = data[5] & 0x20 == 0;
    let (lo, hi) = (data[6] & 0x0F, data[6] >> 4);
    let (left, right) = if swapped { (hi, lo) } else { (lo, hi) };
    let flags = data[7] >> 4;
    let (left_flag, right_flag) = if swapped { (0x2, 0x1) } else { (0x1, 0x2) };
    let case = level(data[7] & 0x0F);
    Some(Pods {
        model,
        left: level(left),
        right: level(right),
        case,
        left_charging: flags & left_flag != 0,
        right_charging: flags & right_flag != 0,
        case_charging: flags & 0x4 != 0,
        lid_open: data.get(8).filter(|_| case.is_some()).map(|lid| lid & 0x08 == 0),
        colour: data.get(9).copied().and_then(Colour::from_apple),
    })
}

/// Which adverts may describe the connected buds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect {
    /// The Device ID record named the exact product: only its adverts (which also keeps a
    /// spoofed advert for another model out, the `spam.rs` concern).
    Model(u16),
    /// Only the name is known: adverts from models of these families.
    Families(&'static [Family]),
    /// A renamed pair without ids: any Apple model.
    Any,
}

const AIRPODS: &[Family] = &[Family::Airpods];
const PRO: &[Family] = &[Family::AirpodsPro];
const MAX: &[Family] = &[Family::AirpodsMax];
const BEATS: &[Family] = &[Family::BeatsEarbuds, Family::BeatsHeadphones, Family::Speaker];

/// What to accept for a device with these ids (if any) and this name.
pub fn expect(ids: Option<DeviceIds>, name: &str) -> Expect {
    if let Some(i) = ids.filter(|i| i.source == VidSource::Bluetooth && i.vid == APPLE && catalog::apple(i.pid).is_some()) {
        return Expect::Model(i.pid);
    }
    let n = name.to_lowercase();
    if n.contains("beats") {
        Expect::Families(BEATS)
    } else if n.contains("airpods max") {
        Expect::Families(MAX)
    } else if n.contains("airpods pro") {
        Expect::Families(PRO)
    } else if n.contains("airpods") {
        Expect::Families(AIRPODS)
    } else {
        Expect::Any
    }
}

/// Whether an ad may describe the connected buds: near enough, and the expected model.
pub fn accept(expected: Expect, pods: &Pods, rssi: i16) -> bool {
    rssi >= NEAR_DBM
        && match expected {
            Expect::Model(pid) => pods.model == pid,
            Expect::Families(families) => catalog::apple(pods.model).is_some_and(|m| families.contains(&m.family)),
            Expect::Any => true,
        }
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

    /// Proximity-pairing messages laid out as documented (company id stripped; the
    /// encrypted tail is zeroed, it isn't read). No AirPods were at hand to record live ones.
    fn payload(head: &[u8]) -> Vec<u8> {
        let mut v = head.to_vec();
        v.resize(27, 0);
        v
    }

    #[test]
    fn devices_airpods_pro_both_buds_in_the_case() {
        // AirPods Pro, left primary (0x75 has 0x20), buds 100 % / 100 %, both charging,
        // case 80 % not charging, lid 0x31 (bit 3 clear: open), colour 00 (white).
        let p = parse(&payload(&[0x07, 0x19, 0x01, 0x0E, 0x20, 0x75, 0xAA, 0x38, 0x31, 0x00])).unwrap();
        assert_eq!(p.model, 0x200E);
        assert_eq!(catalog::apple(p.model).map(|m| m.family), Some(Family::AirpodsPro));
        assert_eq!((p.left, p.right, p.case), (Some(100), Some(100), Some(80)));
        assert!(p.left_charging && p.right_charging && !p.case_charging);
        assert!(p.buds_charging());
        assert_eq!((p.lid_open, p.colour), (Some(true), Some(Colour::White)));
    }

    #[test]
    fn devices_airpods_lid_and_colour() {
        // AirPods Max (Lightning), space grey (09); no case, so the lid byte means nothing.
        let max = parse(&payload(&[0x07, 0x19, 0x01, 0x0A, 0x20, 0x21, 0xF8, 0x0F, 0x08, 0x09])).unwrap();
        assert_eq!((max.model, max.left, max.case), (0x200A, Some(80), None));
        assert_eq!((max.lid_open, max.colour), (None, Some(Colour::SpaceGrey)));
        // Buds in a closed case (bit 3 set), black Powerbeats Pro (01).
        let closed = parse(&payload(&[0x07, 0x19, 0x01, 0x0B, 0x20, 0x75, 0x99, 0x75, 0x09, 0x01])).unwrap();
        assert_eq!((closed.lid_open, closed.colour), (Some(false), Some(Colour::Black)));
        // A colour code nobody has decoded, and a short message without lid and colour.
        assert_eq!(parse(&payload(&[0x07, 0x19, 0x01, 0x0A, 0x20, 0x21, 0xF8, 0x0F, 0x00, 0x10])).unwrap().colour, None);
        let short = parse(&[0x07, 0x19, 0x01, 0x14, 0x20, 0x75, 0x99, 0x09]).unwrap();
        assert_eq!((short.model, short.lid_open, short.colour), (0x2014, None, None));
    }

    #[test]
    fn devices_airpods_4_adverts_are_read() {
        // AirPods 4 (ANC), product id 0x201B: in the catalog, so matched like any other pair.
        let p = parse(&payload(&[0x07, 0x19, 0x01, 0x1B, 0x20, 0x75, 0x66, 0x15, 0x31, 0x00])).unwrap();
        assert_eq!(catalog::apple(p.model).map(|m| m.name), Some("AirPods 4 (ANC)"));
        assert!(accept(expect(None, "Rohit's AirPods"), &p, -50));
    }

    #[test]
    fn devices_airpods_primary_flip_and_missing_parts() {
        // Right bud primary (0x55 lacks 0x20): the bud nibbles and charging flags swap.
        // Buds byte 0x7F: one nibble 15 (not available); flags 0x5 = case + low-bit bud.
        let p = parse(&payload(&[0x07, 0x19, 0x01, 0x0F, 0x20, 0x55, 0x7F, 0x56, 0x00])).unwrap();
        assert_eq!(catalog::apple(p.model).map(|m| m.family), Some(Family::Airpods));
        // Swapped: left = high nibble (7 → 70 %), right = low nibble (15 → none).
        assert_eq!((p.left, p.right), (Some(70), None));
        // Swapped flags: 0x1 is the right bud, 0x4 the case.
        assert!(!p.left_charging && p.right_charging && p.case_charging);
        assert_eq!(p.case, Some(60));
        assert_eq!(p.buds(), Some(70));
        assert!(!p.buds_charging());

        // Not swapped, same bytes otherwise: left = low nibble.
        let q = parse(&payload(&[0x07, 0x19, 0x01, 0x0F, 0x20, 0x75, 0x7F, 0x0F, 0x00])).unwrap();
        assert_eq!((q.left, q.right, q.case), (None, Some(70), None));
    }

    #[test]
    fn devices_airpods_rejects_other_messages_and_far_ads() {
        assert_eq!(parse(&[0x10, 0x05, 0x01, 0x18, 0x00, 0x00, 0x00, 0x00]), None); // "nearby" message
        assert_eq!(parse(&[0x07, 0x19, 0x01]), None);
        // AirPods Pro 2.
        let p = parse(&payload(&[0x07, 0x19, 0x01, 0x14, 0x20, 0x75, 0x99, 0x09, 0x00])).unwrap();
        assert_eq!((p.left, p.right, p.case), (Some(90), Some(90), Some(90)));
        let pro = expect(None, "Rohit's AirPods Pro");
        assert_eq!(pro, Expect::Families(PRO));
        assert!(accept(pro, &p, -50));
        assert!(!accept(pro, &p, -75));
        assert!(!accept(expect(None, "AirPods Max"), &p, -40));
        assert!(accept(expect(None, "Headphones"), &p, -40));
        assert_eq!(expect(None, "Studio Buds (Beats)"), Expect::Families(BEATS));
    }

    #[test]
    fn devices_airpods_known_ids_accept_only_their_model() {
        let ids = |pid| Some(DeviceIds { source: VidSource::Bluetooth, vid: APPLE, pid });
        let pro2 = parse(&payload(&[0x07, 0x19, 0x01, 0x14, 0x20, 0x75, 0x99, 0x09, 0x00])).unwrap();
        let pro2_usb_c = parse(&payload(&[0x07, 0x19, 0x01, 0x24, 0x20, 0x75, 0x99, 0x09, 0x00])).unwrap();
        // Renamed, but the Device ID record says AirPods Pro 2 (Lightning).
        let e = expect(ids(0x2014), "Kitchen");
        assert_eq!(e, Expect::Model(0x2014));
        assert!(accept(e, &pro2, -50));
        assert!(!accept(e, &pro2_usb_c, -50));
        // An Apple id the catalog doesn't know, or a chip vendor's: back to the name.
        assert_eq!(expect(ids(0x20FE), "AirPods Pro"), Expect::Families(PRO));
        assert_eq!(expect(Some(DeviceIds { source: VidSource::Bluetooth, vid: 0x0642, pid: 0x2014 }), "AirPods"), Expect::Families(AIRPODS));
    }
}
