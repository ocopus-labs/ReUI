//! Shelf (`shelf.json`): how long held things stay, opening the shelf, and clearing it
//! (docs/PRD-file-shelf.md §5.3, §10).

use serde_json::Value;

use super::{button, enable, group, head, Effect, Page};
use crate::model::files::{File, KEEP};
use crate::state::State;
use crate::ui::icons;
use crate::ui::{Act, Block, Part, Role, Row, Target};

/// What each `keep` reads as, in [`KEEP`]'s order.
const KEEP_LABELS: [&str; 4] = ["Until removed", "Until ReUI restarts", "A day", "A week"];

pub fn build(state: &State) -> Vec<Block> {
    let mut blocks = head(state, File::Shelf, Page::Shelf, "Drag files, images, links or text onto the island to hold them, then drag them out into any app. Files stay where they are, and nothing leaves this PC.");
    enable(&mut blocks, state, File::Shelf, "Shelf");
    let ok = state.files.doc(File::Shelf).error().is_none();
    let keep = Row::new("keep", "Keep held things").detail("Images and text are saved in ReUI's folder; files are only referred to").part(Part::Dropdown { label: KEEP_LABELS[state.files.keep()].into() }).enabled(ok);
    let open = Row::new("open", "Shelf").detail("What's held now").icon(icons::shelf()).part(button("Show", Role::Secondary)).enabled(ok);
    blocks.push(group("", vec![keep, open]));
    let armed = state.is_armed("clear", 0);
    let clear = Row::new("clear", "Clear the shelf").detail("Lets go of everything; your files stay where they are").icon(icons::danger(icons::DELETE)).part(Part::Button { label: if armed { "Clear now".into() } else { "Clear".into() }, role: Role::Destructive, armed });
    blocks.push(group("", vec![clear]));
    blocks
}

pub fn menu(state: &mut State, target: &Target) -> Option<(Vec<String>, Option<usize>)> {
    (target.row == "keep").then(|| (KEEP_LABELS.iter().map(|l| l.to_string()).collect(), Some(state.files.keep())))
}

pub fn act(state: &mut State, target: &Target, act: Act) -> Effect {
    match (target.row.as_str(), act) {
        ("keep", Act::Choose(i)) => {
            if let Some(keep) = KEEP.get(i) {
                state.set(File::Shelf, "keep", Value::from(*keep));
            }
        }
        ("open", Act::Press) => state.tell_hub("shelf", "open"),
        ("clear", Act::Press) => state.tell_hub("shelf", "clear"),
        _ => {}
    }
    Effect::None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::tests::state;

    #[test]
    fn keep_writes_what_the_module_reads() {
        let (mut state, dir) = state("shelf-page");
        assert_eq!(state.files.keep(), 0);
        assert_eq!(menu(&mut state, &Target::new("keep", 0)).map(|(l, at)| (l.len(), at)), Some((4, Some(0))));
        act(&mut state, &Target::new("keep", 0), Act::Choose(3));
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("roaming").join("shelf.json")).unwrap()).unwrap();
        assert_eq!((saved["keep"].as_str(), saved["saved_cap_mb"].as_u64()), (Some("week"), Some(512)));
        assert_eq!(state.files.keep(), 3);
        _ = std::fs::remove_dir_all(&dir);
    }
}
