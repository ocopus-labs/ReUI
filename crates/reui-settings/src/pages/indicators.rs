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
    // A call: a call app holding the microphone (docs/PRD-calls.md); the apps below only
    // matter while calls are on.
    let calls = state.files.doc(File::Privacy).bool("calls");
    let app = |key: &str, title: &str, detail: &str| {
        let row = flag(state, File::Privacy, key, title, detail);
        if calls { row } else { row.enabled(false) }
    };
    blocks.push(footed(
        "Calls",
        vec![
            flag(state, File::Privacy, "calls", "Ongoing calls", "The call's time and a mute in place of the microphone dot").icon(icons::tile(icons::PHONE, system::GREEN)),
            app("call_teams", "Teams", ""),
            app("call_zoom", "Zoom", ""),
            app("call_discord", "Discord", ""),
            app("call_slack", "Slack", ""),
            app("call_webex", "Webex", ""),
            app("call_messengers", "WhatsApp, Telegram and Signal", ""),
            app("call_browsers", "Browsers", "Any browser using the microphone, such as Meet"),
        ],
        "The mute is the microphone's own, so the app may still show itself unmuted.",
    ));
    blocks
}
