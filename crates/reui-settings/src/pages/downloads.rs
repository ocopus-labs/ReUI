//! Downloads & screenshots (`downloads.json`, `screenshots.json`): each moment's switch,
//! and the extra folders the downloads module watches.

use serde_json::Value;

use super::{button, flag, footed, group, head, Effect, Page};
use crate::model::files::{add_folder, File};
use crate::state::State;
use crate::ui::{icons, Act, Block, Part, Role, Row, Target, Tone};

pub fn build(state: &State) -> Vec<Block> {
    // Two features, two switches: each in its own group rather than one in the header card
    // that would read as both.
    let mut blocks = head(state, File::Downloads, Page::Downloads, "A browser's download in the island while it arrives, and the shot you just took, each with Open and Show in folder. Only names and sizes are read, never what's in a file.");
    let ok = state.files.doc(File::Downloads).error().is_none();

    let downloads = switch(state, File::Downloads, "Downloads", "While a browser downloads, and when it's done").icon(icons::downloads());
    let mut folders = vec![downloads, Row::new("downloads-folder", "Downloads folder").detail("Watched wherever Windows keeps it").icon(icons::folder())];
    folders.extend(state.files.download_folders().iter().map(|f| Row::new(&format!("folder:{f}"), f).mono().part(Part::Remove).enabled(ok)));
    folders.push(Row::new("add-folder", "Add a folder").part(Part::Field { text: state.field.clone(), placeholder: r"D:\Downloads".into() }).part(button("Add", Role::Secondary)).enabled(ok));
    blocks.push(footed("Downloads", folders, "Add a folder for a browser set to save somewhere else."));

    blocks.extend(super::file_error(state, File::Screenshots));
    let shots = switch(state, File::Screenshots, "Screenshots", "The shot after Win+Shift+S or PrtScn").icon(icons::screenshots());
    blocks.push(group("Screenshots", vec![shots]));
    blocks
}

/// A feature's `enabled` toggle as a row, saying when a change applies.
fn switch(state: &State, file: File, title: &str, detail: &str) -> Row {
    let row = flag(state, file, "enabled", title, detail);
    if state.restart_note {
        row.detail("Takes effect when ReUI restarts").tone(Tone::Warning)
    } else {
        row
    }
}

pub fn act(state: &mut State, target: &Target, act: Act) -> Effect {
    let row = target.row.as_str();
    match (row, target.part, act) {
        ("add-folder", 0 | 1, Act::Press) => {
            if let Some(list) = add_folder(&state.files.download_folders(), &state.field) {
                state.set(File::Downloads, "folders", Value::from(list));
                state.field.clear();
            }
        }
        _ => {
            if let Some(folder) = row.strip_prefix("folder:") {
                let list: Vec<String> = state.files.download_folders().into_iter().filter(|f| f != folder).collect();
                state.set(File::Downloads, "folders", Value::from(list));
            }
        }
    }
    Effect::None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::tests::state;

    #[test]
    fn folders_are_added_and_removed_and_the_screenshot_switch_is_its_own_file() {
        let (mut state, dir) = state("downloads-page");
        assert!(matches!(build(&state)[0], Block::Intro { .. }));
        state.field = r"D:\Saves\".into();
        act(&mut state, &Target::new("add-folder", 1), Act::Press);
        assert_eq!(state.files.download_folders(), vec![r"D:\Saves".to_string()]);
        assert!(state.field.is_empty());
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("roaming").join("downloads.json")).unwrap()).unwrap();
        assert_eq!(saved["folders"][0], r"D:\Saves");
        act(&mut state, &Target::new(r"folder:D:\Saves", 0), Act::Press);
        assert!(state.files.download_folders().is_empty());

        crate::pages::act(Page::Downloads, &mut state, &Target::new("screenshots:enabled", 0), Act::Press);
        let shots: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("roaming").join("screenshots.json")).unwrap()).unwrap();
        assert_eq!(shots["enabled"], false);
        _ = std::fs::remove_dir_all(&dir);
    }
}
