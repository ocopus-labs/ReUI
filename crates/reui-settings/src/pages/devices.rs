//! Devices (`devices.json`).

use super::{button, flag, group, head, Effect};
use crate::model::files::File;
use crate::state::State;
use crate::ui::{Act, Block, Role, Row, Target};

pub fn build(state: &State) -> Vec<Block> {
    let mut blocks = head(state, File::Devices, "Devices", "Bluetooth devices as they connect, batteries before they run out, charging, and drives you plug in, with Eject.");
    blocks.push(group(
        "Moments",
        vec![
            flag(state, File::Devices, "connect_cards", "Connected devices", "A card with the battery when a Bluetooth device connects"),
            flag(state, File::Devices, "low_battery", "Low battery", "At 20 % and 10 %, for this PC and its devices"),
            flag(state, File::Devices, "charging", "Charging", "When you plug in, with the time to full"),
            flag(state, File::Devices, "slow_charger", "Slow charger", "When the charger can't keep up"),
            flag(state, File::Devices, "drives", "Drives", "USB drives and memory cards, with Eject"),
            flag(state, File::Devices, "phones", "Phones", "Phones you connect"),
        ],
    ));
    blocks.push(group("AirPods", vec![flag(state, File::Devices, "airpods", "Case and buds", "Battery levels from a short Bluetooth scan nearby")]));
    blocks.push(group("", vec![Row::new("overview", "Your devices").detail("Batteries of everything connected, in the island").part(button("Show", Role::Secondary))]));
    blocks
}

pub fn act(state: &mut State, target: &Target, act: Act) -> Effect {
    if target.row == "overview" && act == Act::Press {
        state.tell_hub("devices", "overview");
    }
    Effect::None
}
