//! Apple and Beats models by product id. Apple's Bluetooth SIG company id is 0x004C; the
//! product id is what the Device ID profile reports (`BTHENUM\…_VID&0001004C_PID&2014`) and,
//! byte for byte, the model field of the proximity-pairing advert (`14 20`, little-endian).
//!
//! Sources (facts only; nothing here is copied from GPL/AGPL projects):
//! - ids: AppleDB (github.com/littlebyteorg/appledb, MIT), `deviceFiles/…/*.json`, where the
//!   `identifier` list carries `Device1,<product id in decimal>` (the file is named in each
//!   row's comment); cross-checked, where it lists them, against The Apple Wiki's "Bluetooth
//!   PIDs" page (revision 330332, Mar 2026), which takes them from Apple's own
//!   `MobileCoreTypes.bundle` / `CoreTypes.bundle` Info.plist files;
//! - names: AppleDB's product names without the bud ("(Left)") or the case and connector
//!   ("with Wireless Charging Case", "with USB-C"), superscripts written as digits; "AirPods
//!   4 (ANC)" as Apple Support writes it ("Identify your AirPods",
//!   support.apple.com/en-us/109525).
//!
//! Colour is set only for products sold in one colour (every AirPods and AirPods Pro is
//! white, per AppleDB's `colors`).

use super::{Colour, Family, Model, APPLE};

const BOTH: &str = "AppleDB deviceFiles (MIT), Device1 id; The Apple Wiki Bluetooth PIDs";
const APPLEDB: &str = "AppleDB deviceFiles (MIT), Device1 id";

const fn white(pid: u16, name: &'static str, family: Family, source: &'static str) -> Model {
    Model { vid: APPLE, pid, name, family, colour: Some(Colour::White), source }
}

const fn model(pid: u16, name: &'static str, family: Family, source: &'static str) -> Model {
    Model { vid: APPLE, pid, name, family, colour: None, source }
}

pub const MODELS: &[Model] = &[
    // AirPods and AirPods Pro (all white earbuds).
    white(0x2002, "AirPods (1st generation)", Family::Airpods, BOTH),         // AirPods1,1 / Device1,8194
    white(0x200F, "AirPods (2nd generation)", Family::Airpods, BOTH),         // AirPods1,2 / Device1,8207
    white(0x2013, "AirPods (3rd generation)", Family::Airpods, BOTH),         // AirPods1,3 / Device1,8211
    white(0x2019, "AirPods 4", Family::Airpods, APPLEDB),                     // AirPods1,4 / Device1,8217
    white(0x201B, "AirPods 4 (ANC)", Family::Airpods, BOTH),                  // AirPods1,5 / Device1,8219
    white(0x2036, "AirPods 5", Family::Airpods, APPLEDB),                     // AirPods3,7 / Device1,8246
    white(0x2030, "AirPods 5", Family::Airpods, APPLEDB),                     // AirPods3,8 (wireless charging case) / Device1,8240
    white(0x200E, "AirPods Pro (1st generation)", Family::AirpodsPro, BOTH),     // AirPodsPro1,1 / Device1,8206
    white(0x2014, "AirPods Pro (2nd generation)", Family::AirpodsPro, BOTH),     // AirPodsPro1,2 (Lightning) / Device1,8212
    white(0x2024, "AirPods Pro (2nd generation)", Family::AirpodsPro, BOTH),     // AirPodsPro1,2 (USB-C) / Device1,8228
    white(0x2027, "AirPods Pro 3", Family::AirpodsPro, APPLEDB),                 // AirPodsPro1,3 / Device1,8231
    white(0x2028, "AirPods Pro 3", Family::AirpodsPro, APPLEDB),                 // AirPodsPro1,3 / Device1,8232
    // AirPods Max come in several colours: the advert says which.
    model(0x200A, "AirPods Max", Family::AirpodsMax, BOTH),    // AirPodsMax1,1 (Lightning) / Device1,8202
    model(0x201F, "AirPods Max", Family::AirpodsMax, BOTH),    // AirPodsMax1,1 (USB-C) / Device1,8223
    model(0x202D, "AirPods Max 2", Family::AirpodsMax, BOTH),  // AirPodsMax1,2 / Device1,8237
    // Beats earbuds.
    model(0x2005, "BeatsX", Family::BeatsEarbuds, BOTH),                       // BeatsX1,1 / Device1,8197
    model(0x2003, "Powerbeats3 Wireless", Family::BeatsEarbuds, BOTH),         // PowerBeats3,1 / Device1,8195
    model(0x200B, "Powerbeats Pro", Family::BeatsEarbuds, BOTH),               // PowerbeatsPro1,1 / Device1,8203
    model(0x200D, "Powerbeats (4th generation)", Family::BeatsEarbuds, BOTH),  // PowerBeats4,1 / Device1,8205
    model(0x2010, "Beats Flex", Family::BeatsEarbuds, BOTH),                   // BeatsX2,1 / Device1,8208
    model(0x2011, "Beats Studio Buds", Family::BeatsEarbuds, BOTH),            // BeatsStudioBuds1,1 / Device1,8209
    model(0x2012, "Beats Fit Pro", Family::BeatsEarbuds, BOTH),                // BeatsFitPro1,1 / Device1,8210
    model(0x2016, "Beats Studio Buds +", Family::BeatsEarbuds, BOTH),          // BeatsStudioBuds1,2 / Device1,8214
    model(0x201D, "Powerbeats Pro 2", Family::BeatsEarbuds, APPLEDB),          // Powerb3,1 / Device1,8221
    model(0x2026, "Beats Solo Buds", Family::BeatsEarbuds, BOTH),              // BeatsSoloBuds1,1 / Device1,8230
    model(0x202F, "Powerbeats Fit", Family::BeatsEarbuds, APPLEDB),            // Powerbeats Fit / Device1,8239
    // Beats headphones.
    model(0x2006, "Beats Solo3 Wireless", Family::BeatsHeadphones, BOTH),      // BeatsSolo3,1 / Device1,8198
    model(0x2007, "Beats Studio3 Wireless", Family::BeatsHeadphones, APPLEDB), // BeatsStudio3,1 / Device1,8199
    model(0x2009, "Beats Studio3 Wireless", Family::BeatsHeadphones, BOTH),    // BeatsStudio3,2 / Device1,8201
    model(0x200C, "Beats Solo Pro", Family::BeatsHeadphones, BOTH),            // BeatsSoloPro1,1 / Device1,8204
    model(0x2017, "Beats Studio Pro", Family::BeatsHeadphones, BOTH),          // BeatsStudioPro1,1 / Device1,8215
    model(0x2025, "Beats Solo 4", Family::BeatsHeadphones, BOTH),              // BeatsSolo4,1 / Device1,8229
    model(0x2038, "Beats 360", Family::BeatsHeadphones, APPLEDB),              // Beats3601,1 / Device1,8248 (filed under headphones)
    // Beats speakers.
    model(0x201A, "Beats Pill", Family::Speaker, BOTH),                   // BeatsPill1,2 (3rd generation) / Device1,8218
    model(0x2600, "Beats Pill+", Family::Speaker, BOTH),                  // Device1,9728
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_catalog_apple_ids_are_apple_product_ids() {
        // Every Apple audio product id so far is 0x20xx (or 0x26xx for the older Pill+).
        assert!(MODELS.iter().all(|m| m.vid == APPLE && (m.pid >> 8 == 0x20 || m.pid >> 8 == 0x26)));
        assert_eq!(MODELS.iter().filter(|m| m.name.starts_with("AirPods Max")).count(), 3);
        assert!(MODELS.iter().filter(|m| m.name.starts_with("AirPods") && !m.name.starts_with("AirPods Max")).all(|m| m.colour == Some(Colour::White)));
    }
}
