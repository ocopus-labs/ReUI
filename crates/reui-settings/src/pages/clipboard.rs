//! Clipboard (`clipboard.json`): the history hotkey, how long history is kept, link
//! previews, apps never recorded, and clearing it.

use serde_json::Value;

use super::{button, enable, flag, group, head, shortcut, Effect, Page};
use crate::ui::icons;
use crate::keys;
use crate::model::files::{add_app, File};
use crate::state::State;
use crate::ui::{Act, Block, Part, Role, Row, Target};

/// Days of history; 0 keeps it forever.
const RETENTION: [u64; 6] = [1, 7, 30, 90, 365, 0];

pub fn build(state: &State) -> Vec<Block> {
    let mut blocks = head(state, File::Clipboard, Page::Clipboard, "A history of what you copy, kept on this PC, to search and paste again. Copies from password managers and anything that looks like a secret are never saved.");
    enable(&mut blocks, state, File::Clipboard, "Clipboard history");
    let ok = state.files.doc(File::Clipboard).error().is_none();

    let hotkey_row = shortcut::row(state, File::Clipboard, "Opens clipboard history");
    let retention = Row::new("retention", "Keep history").detail("Pinned items are kept forever").part(Part::Dropdown { label: days(state.files.retention_days()) }).enabled(ok);
    blocks.push(group("History", vec![hotkey_row, retention, flag(state, File::Clipboard, "link_previews", "Link previews", "Fetches the title and image of copied links from their sites")]));

    let mut apps: Vec<Row> = state.files.ignored_apps().iter().map(|app| Row::new(&format!("app:{app}"), app).mono().part(Part::Remove).enabled(ok)).collect();
    apps.push(
        Row::new("add-app", "Add an app")
            .part(Part::Field { text: state.field.clone(), placeholder: "app.exe".into() })
            .part(button("Add", Role::Secondary))
            .part(Part::Dropdown { label: "Running apps".into() })
            .enabled(ok),
    );
    blocks.push(group("Never recorded from", apps));

    let armed = state.is_armed("clear", 0);
    let clear = Row::new("clear", "Clear history").detail("Removes everything except pinned items").icon(icons::danger(icons::DELETE)).part(Part::Button { label: if armed { "Clear now".into() } else { "Clear".into() }, role: Role::Destructive, armed });
    blocks.push(group("", vec![clear]));
    blocks
}

fn days(n: u64) -> String {
    match n {
        0 => "Forever".into(),
        1 => "1 day".into(),
        365 => "1 year".into(),
        n => format!("{n} days"),
    }
}

fn choices(current: u64) -> Vec<u64> {
    let mut list = RETENTION.to_vec();
    if !list.contains(&current) {
        let at = list.iter().position(|&d| d == 0 || d > current).unwrap_or(list.len());
        list.insert(at, current);
    }
    list
}

pub fn menu(state: &mut State, target: &Target) -> Option<(Vec<String>, Option<usize>)> {
    match (target.row.as_str(), target.part) {
        ("retention", _) => {
            let current = state.files.retention_days();
            let list = choices(current);
            Some((list.iter().map(|&d| days(d)).collect(), list.iter().position(|&d| d == current)))
        }
        ("add-app", 2) => {
            let ignored = state.files.ignored_apps();
            state.menu_apps = keys::running_apps().into_iter().filter(|a| !ignored.contains(a)).collect();
            Some((state.menu_apps.clone(), None))
        }
        _ => None,
    }
}

pub fn act(state: &mut State, target: &Target, act: Act) -> Effect {
    let row = target.row.as_str();
    match (row, target.part, act) {
        ("hotkey", _, Act::Press) => return Effect::Record(File::Clipboard),
        ("retention", _, Act::Choose(i)) => {
            if let Some(&d) = choices(state.files.retention_days()).get(i) {
                state.set(File::Clipboard, "retention_days", Value::from(d));
            }
        }
        ("add-app", 0 | 1, Act::Press) => {
            let text = state.field.clone();
            if add(state, &text) {
                state.field.clear();
            }
        }
        ("add-app", 2, Act::Choose(i)) => {
            if let Some(app) = state.menu_apps.get(i).cloned() {
                add(state, &app);
            }
        }
        ("clear", _, Act::Press) => state.tell_hub("clipboard", "clear"),
        _ => {
            if let Some(app) = row.strip_prefix("app:") {
                let list: Vec<String> = state.files.ignored_apps().into_iter().filter(|a| a != app).collect();
                state.set(File::Clipboard, "ignored_apps", Value::from(list));
            }
        }
    }
    Effect::None
}

fn add(state: &mut State, name: &str) -> bool {
    match add_app(&state.files.ignored_apps(), name) {
        Some(list) => {
            state.set(File::Clipboard, "ignored_apps", Value::from(list));
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::tests::state;

    #[test]
    fn apps_are_added_and_removed() {
        let (mut state, dir) = state("clipboard-apps");
        state.field = "Code".into();
        act(&mut state, &Target::new("add-app", 1), Act::Press);
        assert!(state.files.ignored_apps().contains(&"code.exe".to_string()));
        assert!(state.field.is_empty());
        act(&mut state, &Target::new("app:code.exe", 0), Act::Press);
        assert!(!state.files.ignored_apps().contains(&"code.exe".to_string()));
        assert_eq!(state.files.ignored_apps().len(), 8);
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn retention_choices_keep_odd_values() {
        assert_eq!(choices(30), RETENTION);
        assert_eq!(choices(45), [1, 7, 30, 45, 90, 365, 0]);
        assert_eq!((days(0).as_str(), days(1).as_str(), days(365).as_str(), days(14).as_str()), ("Forever", "1 day", "1 year", "14 days"));
    }
}
