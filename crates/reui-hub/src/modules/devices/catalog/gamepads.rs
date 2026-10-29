//! Game controllers by USB-IF vendor and product id: what `RawGameController`'s
//! `HardwareVendorId` / `HardwareProductId` report, and what Bluetooth controllers put in
//! their Device ID record (vendor id source 2).
//!
//! Source: SDL's `src/joystick/controller_list.h` (github.com/libsdl-org/SDL, zlib licence,
//! Copyright (C) Valve Corporation; ids and the product named in each row's comment).
//! Thanks to the SDL project. Names are the product names as Microsoft, Sony and Nintendo
//! sell them, shortened where SDL's comment adds the connection ("(Bluetooth)", "(BLE)").

use super::{Family, Model};

const MICROSOFT: u16 = 0x045E;
const SONY: u16 = 0x054C;
const NINTENDO: u16 = 0x057E;
const SDL: &str = "SDL controller_list.h (zlib)";

const fn pad(vid: u16, pid: u16, name: &'static str, family: Family) -> Model {
    Model { vid, pid, name, family, colour: None, source: SDL }
}

pub const MODELS: &[Model] = &[
    // Xbox.
    pad(MICROSOFT, 0x028E, "Xbox 360 Controller", Family::XboxController),
    pad(MICROSOFT, 0x028F, "Xbox 360 Controller", Family::XboxController),
    pad(MICROSOFT, 0x02A1, "Xbox 360 Wireless Controller", Family::XboxController),
    pad(MICROSOFT, 0x0719, "Xbox 360 Wireless Controller", Family::XboxController),
    pad(MICROSOFT, 0x02D1, "Xbox One Controller", Family::XboxController),
    pad(MICROSOFT, 0x02DD, "Xbox One Controller", Family::XboxController),
    pad(MICROSOFT, 0x02FF, "Xbox One Controller", Family::XboxController),
    pad(MICROSOFT, 0x02E0, "Xbox One S Controller", Family::XboxController),
    pad(MICROSOFT, 0x02EA, "Xbox One S Controller", Family::XboxController),
    pad(MICROSOFT, 0x02FD, "Xbox One S Controller", Family::XboxController),
    pad(MICROSOFT, 0x0B20, "Xbox One S Controller", Family::XboxController),
    pad(MICROSOFT, 0x0B12, "Xbox Series X|S Controller", Family::XboxController),
    pad(MICROSOFT, 0x0B13, "Xbox Series X|S Controller", Family::XboxController),
    pad(MICROSOFT, 0x02E3, "Xbox Elite Controller", Family::XboxController),
    pad(MICROSOFT, 0x0B00, "Xbox Elite Series 2 Controller", Family::XboxController),
    pad(MICROSOFT, 0x0B05, "Xbox Elite Series 2 Controller", Family::XboxController),
    pad(MICROSOFT, 0x0B22, "Xbox Elite Series 2 Controller", Family::XboxController),
    // The Adaptive Controller doesn't look like a gamepad: the generic outline.
    pad(MICROSOFT, 0x0B0A, "Xbox Adaptive Controller", Family::Gamepad),
    pad(MICROSOFT, 0x0B0C, "Xbox Adaptive Controller", Family::Gamepad),
    pad(MICROSOFT, 0x0B21, "Xbox Adaptive Controller", Family::Gamepad),
    // PlayStation.
    pad(SONY, 0x0268, "DualShock 3", Family::Dualshock),
    pad(SONY, 0x05C4, "DualShock 4", Family::Dualshock),
    pad(SONY, 0x09CC, "DualShock 4", Family::Dualshock),
    pad(SONY, 0x0BA0, "DualShock 4", Family::Dualshock),
    pad(SONY, 0x0CE6, "DualSense", Family::Dualsense),
    pad(SONY, 0x0DF2, "DualSense Edge", Family::Dualsense),
    pad(SONY, 0x0E5F, "Access Controller", Family::Gamepad),
    // Nintendo.
    pad(NINTENDO, 0x2009, "Switch Pro Controller", Family::SwitchPro),
    pad(NINTENDO, 0x2069, "Switch 2 Pro Controller", Family::SwitchPro),
    pad(NINTENDO, 0x2006, "Joy-Con (L)", Family::Gamepad),
    pad(NINTENDO, 0x2007, "Joy-Con (R)", Family::Gamepad),
    pad(NINTENDO, 0x2008, "Joy-Con (L/R)", Family::Gamepad),
    pad(NINTENDO, 0x2067, "Joy-Con 2 (L)", Family::Gamepad),
    pad(NINTENDO, 0x2066, "Joy-Con 2 (R)", Family::Gamepad),
    pad(NINTENDO, 0x2068, "Joy-Con 2 (L/R)", Family::Gamepad),
];
