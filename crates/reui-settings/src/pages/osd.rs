//! Volume & brightness (`osd.json`).

use serde_json::Value;

use super::{flag, footed, group, head, Effect, Page};
use crate::ui::icons;
use reui_design::tokens::color::system;
use crate::model::files::File;
use crate::state::State;
use crate::ui::{Act, Block, Part, Row, Target};

/// The wheel step slider spans 1 %..=10 % in whole percents.
const STEP_MIN: f64 = 0.01;
const STEP_MAX: f64 = 0.10;

pub fn build(state: &State) -> Vec<Block> {
    let mut blocks = head(state, File::Osd, Page::Osd, "The island answers the volume and brightness keys with a level you can drag, and shows Caps Lock, battery saver and the microphone when they change.");
    let ok = state.files.doc(File::Osd).error().is_none();
    let step = state.files.scroll_step();
    let wheel = Row::new("step", "Wheel step")
        .detail("How much one wheel notch over the island changes the level")
        .part(Part::Slider { value: step_value(step), reading: format!("{} %", (step * 100.0).round()) })
        .enabled(ok);
    blocks.push(group(
        "Pop-ups",
        vec![
            flag(state, File::Osd, "volume", "Volume", "When the volume changes"),
            flag(state, File::Osd, "brightness", "Brightness", "When the screen's brightness changes"),
            flag(state, File::Osd, "show_automatic_brightness", "Automatic brightness", "Also when Windows changes the brightness by itself"),
            flag(state, File::Osd, "show_percentage", "Show percentage", "The level as a number in the open pop-up"),
            wheel,
        ],
    ));
    blocks.push(footed(
        "Windows' pop-up",
        vec![flag(state, File::Osd, "replace_windows_flyout", "Replace Windows' pop-up", "Only ReUI's level shows").icon(icons::gray(icons::MONITOR))],
        "Windows' own volume and brightness pop-up is hidden while ReUI runs, and back when ReUI quits.",
    ));
    blocks.push(group(
        "System states",
        vec![
            flag(state, File::Osd, "lock_keys", "Caps Lock and Num Lock", "").icon(icons::gray(icons::KEYBOARD)),
            flag(state, File::Osd, "battery_saver", "Battery saver", "").icon(icons::tile(icons::BATTERY, system::GREEN)),
            flag(state, File::Osd, "mic_mute", "Microphone mute", "").icon(icons::tile(icons::MIC, system::RED)),
        ],
    ));
    blocks
}

/// The step (0.01..=0.10) as a slider position.
fn step_value(step: f64) -> f32 {
    (((step - STEP_MIN) / (STEP_MAX - STEP_MIN)).clamp(0.0, 1.0)) as f32
}

/// A slider position as a step, in whole percents.
fn step_at(value: f32) -> f64 {
    let percent = (STEP_MIN + f64::from(value.clamp(0.0, 1.0)) * (STEP_MAX - STEP_MIN)) * 100.0;
    percent.round() / 100.0
}

pub fn act(state: &mut State, target: &Target, act: Act) -> Effect {
    if let ("step", Act::Value(v)) = (target.row.as_str(), act) {
        let step = step_at(v);
        if (step - state.files.scroll_step()).abs() > 1e-9 {
            state.set(File::Osd, "scroll_step", Value::from(step));
        }
    }
    Effect::None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_step_slider_moves_in_whole_percents() {
        assert_eq!(step_at(0.0), 0.01);
        assert_eq!(step_at(1.0), 0.1);
        assert_eq!(step_at(0.12), 0.02);
        assert_eq!(step_value(0.02), (0.01 / 0.09) as f32);
        assert_eq!(step_value(0.5), 1.0);
    }
}
