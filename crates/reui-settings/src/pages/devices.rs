//! Devices (`devices.json`).

use super::{button, enable, flag, footed, group, head, Effect, Page};
use crate::ui::icons;
use reui_design::tokens::color::system;
use crate::model::files::File;
use crate::state::State;
use crate::ui::{Act, Block, Role, Row, Target};

pub fn build(state: &State) -> Vec<Block> {
    let mut blocks = head(state, File::Devices, Page::Devices, "Bluetooth devices as they connect, batteries before they run out, charging, and drives you plug in, with Eject.");
    enable(&mut blocks, state, File::Devices, "Devices");
    blocks.push(group(
        "Moments",
        vec![
            flag(state, File::Devices, "connect_cards", "Connected devices", "A card with the battery when a Bluetooth device connects").icon(icons::tile(icons::BLUETOOTH, system::BLUE)),
            flag(state, File::Devices, "low_battery", "Low battery", "At 20 % and 10 %, for this PC and its devices").icon(icons::tile(icons::BATTERY_LOW, system::RED)),
            flag(state, File::Devices, "charging", "Charging", "When you plug in, with the time to full").icon(icons::tile(icons::BOLT, system::GREEN)),
            flag(state, File::Devices, "slow_charger", "Slow charger", "When the charger can't keep up").icon(icons::tile(icons::WARNING, system::ORANGE)),
            flag(state, File::Devices, "drives", "Drives", "USB drives and memory cards, with Eject").icon(icons::gray(icons::USB)),
            flag(state, File::Devices, "phones", "Phones", "Phones you connect").icon(icons::gray(icons::PHONE)),
        ],
    ));
    blocks.push(footed("AirPods", vec![flag(state, File::Devices, "airpods", "Case and buds", "Battery of each bud and the case").icon(icons::gray(icons::HEADPHONES))], "Read from a short Bluetooth scan nearby."));
    blocks.push(group("", vec![Row::new("overview", "Your devices").detail("Batteries of everything connected, in the island").icon(icons::devices()).part(button("Show", Role::Secondary))]));
    blocks
}

pub fn act(state: &mut State, target: &Target, act: Act) -> Effect {
    if target.row == "overview" && act == Act::Press {
        state.tell_hub("devices", "overview");
    }
    Effect::None
}
