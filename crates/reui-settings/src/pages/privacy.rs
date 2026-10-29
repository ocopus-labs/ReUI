//! Privacy & data: the plain statement, where data lives and how big it is, what goes
//! online, and clearing clipboard history.

use super::{button, group, intro, Effect, Page};
use crate::model::files::File;
use crate::state::State;
use crate::system;
use crate::ui::{icons, Act, Block, Part, Role, Row, Target};

pub const STATEMENT: &str = "Everything stays on this PC. ReUI connects to the internet only to show link previews, to fetch lyrics (lrclib.net) when you turn them on, and to check for updates.";

pub fn build(state: &State) -> Vec<Block> {
    let mut blocks = vec![intro(state, Page::Privacy, STATEMENT)];
    let folder = |id: &str, title: &str, path: &std::path::Path, size: u64| {
        Row::new(id, title).icon(icons::folder()).detail(path.display().to_string()).part(Part::Value(system::bytes(size))).part(button("Open", Role::Secondary))
    };
    blocks.push(group(
        "On this PC",
        vec![
            folder("data-roaming", "Settings and clipboard history", &state.paths.config, state.sizes.0),
            folder("data-local", "Caches and logs", &state.paths.local, state.sizes.1),
        ],
    ));
    let on = |file: File, key: &str| if state.files.doc(file).bool(key) { "On" } else { "Off" };
    blocks.push(group(
        "Online",
        vec![
            Row::new("net-lyrics", "Synced lyrics").icon(icons::tile(icons::LYRICS, reui_design::tokens::prefs::section::NOW_PLAYING)).detail("Song titles go to lrclib.net").part(Part::Value(on(File::Media, "lyrics").into())).part(button("Change", Role::Secondary)),
            Row::new("net-links", "Link previews").icon(icons::tile(icons::LINK, reui_design::tokens::prefs::section::CLIPBOARD)).detail("Copied links' pages are fetched for their title and image").part(Part::Value(on(File::Clipboard, "link_previews").into())).part(button("Change", Role::Secondary)),
            Row::new("net-updates", "Update checks").icon(icons::tile(icons::UPDATE, reui_design::tokens::color::system::GREEN)).detail("api.github.com, once a day, no personal data").part(Part::Value(on(File::Island, "check_updates").into())).part(button("Change", Role::Secondary)),
        ],
    ));
    let armed = state.is_armed("clear", 0);
    blocks.push(group(
        "",
        vec![Row::new("clear", "Clear clipboard history").icon(icons::danger(icons::DELETE)).detail("Removes everything except pinned items").part(Part::Button { label: if armed { "Clear now".into() } else { "Clear".into() }, role: Role::Destructive, armed })],
    ));
    blocks
}

pub fn act(state: &mut State, target: &Target, act: Act) -> Effect {
    if act != Act::Press {
        return Effect::None;
    }
    match target.row.as_str() {
        "data-roaming" => system::open_folder(&state.paths.config),
        "data-local" => system::open_folder(&state.paths.local),
        "net-lyrics" => return Effect::Navigate(Page::NowPlaying),
        "net-links" => return Effect::Navigate(Page::Clipboard),
        "net-updates" => return Effect::Navigate(Page::About),
        "clear" => {
            state.tell_hub("clipboard", "clear");
            state.refresh_sizes();
        }
        _ => {}
    }
    Effect::None
}
