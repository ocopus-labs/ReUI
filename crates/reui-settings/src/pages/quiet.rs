//! General's quieting groups (docs/PRD-behaviour.md §4.3–4.4): **App rules**, the apps in
//! front of which the island hides or shows only decisions (settings.json `app_rules`, an
//! object of exe → "hide" | "decisions"; apps are added as the clipboard's ignored apps are:
//! typed, or picked from the running apps), and **Snoozed**, the modules snoozed from a
//! moment's menu (`snoozed`, module → Unix ms), each with Undo.

use serde_json::{Map, Value};

use super::{button, footed, group, Page};
use crate::keys;
use crate::system;
use crate::model::files::{add_app, File, APP_RULES};
use crate::state::State;
use crate::ui::{Act, Block, Part, Role, Row, Target};

/// How each of [`APP_RULES`] reads in its menu.
const RULE_LABELS: [&str; 2] = ["Hide the island", "Only decisions"];

/// The App rules group.
pub fn blocks(state: &State, ok: bool) -> Vec<Block> {
    let mut rows: Vec<Row> = state
        .files
        .app_rules()
        .into_iter()
        .map(|(exe, rule)| Row::new(&format!("rule:{exe}"), &exe).mono().part(Part::Dropdown { label: RULE_LABELS[rule].into() }).part(Part::Remove).enabled(ok))
        .collect();
    rows.push(
        Row::new("add-rule", "Add an app")
            .part(Part::Field { text: state.field.clone(), placeholder: "app.exe".into() })
            .part(button("Add", Role::Secondary))
            .part(Part::Dropdown { label: "Running apps".into() })
            .enabled(ok),
    );
    let mut blocks = vec![footed("App rules", rows, "While the app is in front. Only decisions still shows permission requests and questions.")];
    let now = reui_protocol::unix_ms();
    let snoozed: Vec<Row> = state
        .files
        .snoozed(now)
        .into_iter()
        .map(|(module, until)| Row::new(&format!("snooze:{module}"), &module_name(&module)).detail(format!("Shows again in {}", system::time_left(until - now))).part(button("Undo", Role::Secondary)).enabled(ok))
        .collect();
    if !snoozed.is_empty() {
        blocks.push(group("Snoozed", snoozed));
    }
    blocks
}

/// A module as its Settings page names it.
fn module_name(module: &str) -> String {
    let page = match module {
        "osd" => Some(Page::Osd),
        "devices" => Some(Page::Devices),
        "clipboard" => Some(Page::Clipboard),
        "timers" => Some(Page::Timers),
        "media" => Some(Page::NowPlaying),
        "privacy" => Some(Page::Indicators),
        "stats" => Some(Page::Stats),
        "claude" | "codex" | "opencode" => Some(Page::Agents),
        _ => None,
    };
    page.map_or_else(|| module.to_string(), |p| p.title().to_string())
}

pub fn menu(state: &mut State, target: &Target) -> Option<(Vec<String>, Option<usize>)> {
    if target.row == "add-rule" && target.part == 2 {
        let ruled: Vec<String> = state.files.app_rules().into_iter().map(|(exe, _)| exe).collect();
        state.menu_apps = keys::running_apps().into_iter().filter(|a| !ruled.contains(a)).collect();
        return Some((state.menu_apps.clone(), None));
    }
    let exe = target.row.strip_prefix("rule:")?;
    let current = state.files.app_rules().into_iter().find(|(e, _)| e == exe).map(|(_, rule)| rule);
    Some((RULE_LABELS.iter().map(|l| l.to_string()).collect(), current))
}

/// Handles the group's controls; `false` when the target isn't one of them.
pub fn act(state: &mut State, target: &Target, act: &Act) -> bool {
    match (target.row.as_str(), target.part, act) {
        ("add-rule", 0 | 1, Act::Press) => {
            let text = state.field.clone();
            if add(state, &text) {
                state.field.clear();
            }
        }
        ("add-rule", 2, Act::Choose(i)) => {
            if let Some(app) = state.menu_apps.get(*i).cloned() {
                add(state, &app);
            }
        }
        (row, 0, Act::Choose(i)) if row.starts_with("rule:") && *i < APP_RULES.len() => {
            let exe = row["rule:".len()..].to_string();
            edit(state, |rules| _ = rules.insert(exe, Value::from(APP_RULES[*i])));
        }
        (row, 1, Act::Press) if row.starts_with("rule:") => {
            let exe = row["rule:".len()..].to_string();
            edit(state, |rules| _ = rules.remove(&exe));
        }
        // The island sees the file change and lets the module's moments through again.
        (row, 0, Act::Press) if row.starts_with("snooze:") => {
            let module = &row["snooze:".len()..];
            let mut snoozed = state.files.doc(File::Island).get("snoozed").and_then(Value::as_object).cloned().unwrap_or_default();
            snoozed.remove(module);
            state.set(File::Island, "snoozed", Value::Object(snoozed));
        }
        _ => return false,
    }
    true
}

/// Adds an app with the first rule (hide), named as the island matches it.
fn add(state: &mut State, name: &str) -> bool {
    let ruled: Vec<String> = state.files.app_rules().into_iter().map(|(exe, _)| exe).collect();
    let Some(exe) = add_app(&ruled, name).and_then(|list| list.last().cloned()) else { return false };
    edit(state, |rules| _ = rules.insert(exe, Value::from(APP_RULES[0])));
    true
}

/// Changes the `app_rules` object as it is in the file, so entries this version doesn't
/// understand are kept.
fn edit(state: &mut State, change: impl FnOnce(&mut Map<String, Value>)) {
    let mut rules = state.files.doc(File::Island).get("app_rules").and_then(Value::as_object).cloned().unwrap_or_default();
    change(&mut rules);
    state.set(File::Island, "app_rules", Value::Object(rules));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::tests::state;

    #[test]
    fn rules_are_added_changed_and_removed() {
        let (mut state, dir) = state("quiet-rules");
        state.field = "Game".into();
        assert!(act(&mut state, &Target::new("add-rule", 1), &Act::Press));
        assert_eq!(state.files.app_rules(), vec![("game.exe".to_string(), 0)]);
        assert!(state.field.is_empty());
        act(&mut state, &Target::new("rule:game.exe", 0), &Act::Choose(1));
        assert_eq!(state.files.app_rules(), vec![("game.exe".to_string(), 1)]);
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("roaming").join("settings.json")).unwrap()).unwrap();
        assert_eq!(saved["app_rules"]["game.exe"], "decisions");
        // Twice is once.
        state.field = "game.exe".into();
        act(&mut state, &Target::new("add-rule", 0), &Act::Press);
        assert_eq!(state.files.app_rules().len(), 1);
        act(&mut state, &Target::new("rule:game.exe", 1), &Act::Press);
        assert!(state.files.app_rules().is_empty());
        assert!(!act(&mut state, &Target::new("visibility", 0), &Act::Choose(0)));
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn snoozes_show_with_undo() {
        let (mut state, dir) = state("quiet-snoozed");
        std::fs::create_dir_all(dir.join("roaming")).unwrap();
        let later = reui_protocol::unix_ms() + 30 * 60_000;
        std::fs::write(dir.join("roaming").join("settings.json"), format!(r#"{{ "snoozed": {{ "osd": {later}, "devices": 5 }} }}"#)).unwrap();
        state.refresh();
        let built = blocks(&state, true);
        let Some(Block::Group { label, rows, .. }) = built.last() else { panic!("no snoozed group") };
        assert_eq!((label.as_deref(), rows.len(), rows[0].title.as_str()), (Some("Snoozed"), 1, "Volume & brightness"), "ended snoozes don't show");
        assert!(act(&mut state, &Target::new("snooze:osd", 0), &Act::Press));
        assert!(state.files.snoozed(0).iter().all(|(m, _)| m != "osd"));
        assert_eq!(blocks(&state, true).len(), 1, "nothing snoozed: no group");
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_entries_are_kept() {
        let (mut state, dir) = state("quiet-unknown");
        std::fs::create_dir_all(dir.join("roaming")).unwrap();
        std::fs::write(dir.join("roaming").join("settings.json"), r#"{ "app_rules": { "x.exe": "someday" } }"#).unwrap();
        state.refresh();
        assert!(state.files.app_rules().is_empty());
        state.field = "y".into();
        act(&mut state, &Target::new("add-rule", 1), &Act::Press);
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("roaming").join("settings.json")).unwrap()).unwrap();
        assert_eq!((saved["app_rules"]["x.exe"].as_str(), saved["app_rules"]["y.exe"].as_str()), (Some("someday"), Some("hide")));
        _ = std::fs::remove_dir_all(&dir);
    }
}
