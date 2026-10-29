//! System stats (`stats.json`): the shortcut that opens the card, and which rows it shows
//! (each row left out is a counter never read).

use super::{button, enable, flag, footed, group, head, shortcut, Effect, Page};
use crate::model::files::File;
use crate::state::State;
use crate::ui::icons;
use crate::ui::{Act, Block, Role, Row, Target};

pub fn build(state: &State) -> Vec<Block> {
    let mut blocks = head(state, File::Stats, Page::Stats, "CPU, GPU, memory, disk and network in the island, read only while the card is open.");
    enable(&mut blocks, state, File::Stats, "System stats");
    let ok = state.files.doc(File::Stats).error().is_none();
    let show = Row::new("show", "System stats").detail("How busy this PC is, right now").icon(icons::stats()).part(button("Show", Role::Secondary)).enabled(ok);
    blocks.push(group("", vec![shortcut::row(state, File::Stats, "Opens the stats card"), show]));
    blocks.push(footed(
        "In the card",
        vec![
            flag(state, File::Stats, "gpu", "GPU", "The busiest engine, and its memory"),
            flag(state, File::Stats, "disk", "Disk", "How much of the time the disks are busy"),
            flag(state, File::Stats, "network", "Network", "Download and upload"),
            flag(state, File::Stats, "power", "Power", "What the battery gives or takes, on a laptop"),
        ],
        "Read once a second while the card is open, and never otherwise.",
    ));
    blocks
}

pub fn act(state: &mut State, target: &Target, act: Act) -> Effect {
    match (target.row.as_str(), act) {
        ("hotkey", Act::Press) => return Effect::Record(File::Stats),
        ("show", Act::Press) => state.tell_hub("stats", "open"),
        _ => {}
    }
    Effect::None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::tests::state;

    #[test]
    fn stats_rows_write_the_keys_the_module_reads() {
        let (mut state, dir) = state("stats-page");
        let blocks = build(&state);
        assert!(matches!(blocks[0], Block::Intro { .. }));
        assert_eq!(state.files.shortcut(File::Stats), "Win+Alt+S");
        crate::pages::act(Page::Stats, &mut state, &Target::new("stats:gpu", 0), Act::Press);
        let saved: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("roaming").join("stats.json")).unwrap()).unwrap();
        assert_eq!((saved["gpu"].as_bool(), saved["disk"].as_bool()), (Some(false), Some(true)));
        _ = std::fs::remove_dir_all(&dir);
    }
}
