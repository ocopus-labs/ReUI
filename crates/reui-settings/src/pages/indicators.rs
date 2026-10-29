//! Mic & camera (`privacy.json`): which kinds of use the privacy indicator shows.

use reui_design::tokens::color::system;

use super::{enable, flag, footed, head, Page};
use crate::model::files::File;
use crate::state::State;
use crate::ui::{icons, Block};

pub fn build(state: &State) -> Vec<Block> {
    let mut blocks = head(state, File::Privacy, Page::Indicators, "The app using your microphone or camera, or sharing your screen, in the island while it does, with a mute for the microphone.");
    enable(&mut blocks, state, File::Privacy, "Privacy indicators");
    blocks.push(footed(
        "Show",
        vec![
            flag(state, File::Privacy, "microphone", "Microphone", "").icon(icons::tile(icons::MIC, system::ORANGE)),
            flag(state, File::Privacy, "camera", "Camera", "").icon(icons::tile(icons::CAMERA, system::GREEN)),
            flag(state, File::Privacy, "screen", "Screen sharing and recording", "").icon(icons::tile(icons::SCREEN, system::PURPLE)),
            flag(state, File::Privacy, "location", "Location", "Weather and widgets use it often").icon(icons::tile(icons::LOCATION, system::BLUE)),
        ],
        "Read from Windows' own record of which apps use them.",
    ));
    blocks
}
