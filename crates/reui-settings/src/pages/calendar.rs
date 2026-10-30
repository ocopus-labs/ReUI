//! Calendar (`calendar.json`): the calendars read (secret ICS links, shown by their host
//! with how their last refresh went; Windows' own store), adding a link, and how long
//! before a meeting the island starts counting down.

use serde_json::Value;

use super::{button, enable, flag, footed, group, head, Effect, Page};
use crate::model::calendar::{self, Refresh};
use crate::model::files::File;
use crate::state::State;
use crate::ui::{icons, Act, Block, Part, Role, Row, Target, Tone};

/// Minutes before a meeting the countdown starts.
const LEAD: [u64; 3] = [5, 10, 15];

pub fn build(state: &State) -> Vec<Block> {
    let mut blocks = head(state, File::Calendar, Page::Calendar, "Your next meeting in the island: a countdown in its last minutes, and Join when it starts. ReUI reads your calendars from their own secret links and asks nothing else.");
    enable(&mut blocks, state, File::Calendar, "Calendar");
    let doc = state.files.doc(File::Calendar);
    let ok = doc.error().is_none();
    let status = calendar::status(&state.paths);

    let mut rows: Vec<Row> = calendar::sources(doc)
        .into_iter()
        .map(|s| {
            let row = Row::new(&format!("source:{}", s.id), if s.host.is_empty() { "Calendar" } else { &s.host }).icon(icons::calendar()).part(Part::Toggle { on: s.on }).part(Part::Remove).enabled(ok);
            let failing = s.on && status.get(&s.id).is_some_and(|r| !r.ok);
            // Last, so the toggle and Remove keep their places whether it's there or not.
            let row = if failing { row.part(button("Try again", Role::Secondary)) } else { row };
            refreshed(row, status.get(&s.id), s.on)
        })
        .collect();
    let add = Row::new("add-link", "Add a calendar")
        .detail("Its secret iCal address, from Google or Outlook")
        .part(Part::Field { text: state.field.clone(), placeholder: "https://…/basic.ics".into() })
        .part(button("Add", Role::Secondary))
        .enabled(ok);
    let add = match &state.calendar_note {
        Some(note) => add.detail(note.clone()).tone(Tone::Warning),
        None => add,
    };
    rows.push(add);
    let windows = flag(state, File::Calendar, "windows", "Windows calendar", "Calendars Windows itself keeps, from its Mail and Calendar app");
    let windows = if doc.bool("windows") { refreshed(windows, status.get("windows"), true) } else { windows };
    rows.push(windows);
    blocks.push(footed("Your calendars", rows, "Links and the copies ReUI keeps are encrypted for your Windows account. ReUI asks only each calendar's own server, every 15 minutes."));

    let lead = Row::new("lead", "Count down from").detail("Before a meeting starts").part(Part::Dropdown { label: minutes(lead_min(state)) }).enabled(ok);
    blocks.push(group("", vec![lead]));
    blocks
}

/// A source's row with how its last refresh went.
fn refreshed(row: Row, refresh: Option<&Refresh>, on: bool) -> Row {
    match refresh {
        _ if !on => row,
        Some(r) if r.ok => row.detail(r.line(reui_protocol::unix_ms())),
        Some(r) => row.detail(r.line(reui_protocol::unix_ms())).tone(Tone::Warning),
        None => row,
    }
}

fn lead_min(state: &State) -> u64 {
    state.files.doc(File::Calendar).get("lead_min").and_then(Value::as_u64).filter(|m| (1..=60).contains(m)).unwrap_or(10)
}

fn minutes(n: u64) -> String {
    if n == 1 { "1 minute".into() } else { format!("{n} minutes") }
}

fn choices(current: u64) -> Vec<u64> {
    let mut list = LEAD.to_vec();
    if !list.contains(&current) {
        list.push(current);
        list.sort_unstable();
    }
    list
}

pub fn menu(state: &mut State, target: &Target) -> Option<(Vec<String>, Option<usize>)> {
    (target.row == "lead").then(|| {
        let current = lead_min(state);
        let list = choices(current);
        (list.iter().map(|&m| minutes(m)).collect(), list.iter().position(|&m| m == current))
    })
}

pub fn act(state: &mut State, target: &Target, act: Act) -> Effect {
    let row = target.row.as_str();
    match (row, target.part, act) {
        ("add-link", 0 | 1, Act::Press) => {
            let text = state.field.clone();
            match calendar::add(state.files.doc(File::Calendar), &text) {
                Ok(list) => {
                    state.set(File::Calendar, "sources", list);
                    state.field.clear();
                    state.calendar_note = None;
                }
                Err(why) => state.calendar_note = Some(why.into()),
            }
        }
        ("lead", _, Act::Choose(i)) => {
            if let Some(&m) = choices(lead_min(state)).get(i) {
                state.set(File::Calendar, "lead_min", Value::from(m));
            }
        }
        // Try again: every source, now, however recently the hub last asked.
        (_, 2, Act::Press) if row.starts_with("source:") => state.tell_hub("calendar", "refresh"),
        (_, part, Act::Press) => {
            if let Some(id) = row.strip_prefix("source:") {
                let doc = state.files.doc(File::Calendar);
                let on = calendar::sources(doc).iter().find(|s| s.id == id).is_some_and(|s| s.on);
                let list = calendar::change(doc, id, if part == 0 { Some(!on) } else { None });
                state.set(File::Calendar, "sources", list);
            }
        }
        _ => {}
    }
    Effect::None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::tests::state;

    #[test]
    fn a_link_is_added_sealed_toggled_and_removed() {
        let (mut state, dir) = state("calendar-page");
        state.field = "notalink".into();
        act(&mut state, &Target::new("add-link", 1), Act::Press);
        assert!(state.calendar_note.is_some(), "a bad link says why");
        state.field = "https://calendar.example.com/ical/private-xyz/basic.ics".into();
        act(&mut state, &Target::new("add-link", 1), Act::Press);
        assert!(state.field.is_empty() && state.calendar_note.is_none());
        let written = std::fs::read_to_string(dir.join("roaming").join("calendar.json")).unwrap();
        assert!(written.contains("dpapi:") && !written.contains("private-xyz"));
        let id = calendar::sources(state.files.doc(File::Calendar))[0].id.clone();
        // A source that keeps failing says why on its row, with Try again after its toggle
        // and Remove; pressing it removes nothing.
        let status = dir.join("local").join("calendar");
        std::fs::create_dir_all(&status).unwrap();
        std::fs::write(status.join("status.json"), format!(r#"{{ "{id}": {{ "ok": false, "at": 0, "events": 3, "error": "timed out" }} }}"#)).unwrap();
        let blocks = build(&state);
        let row = blocks.iter().flat_map(|b| if let Block::Group { rows, .. } = b { rows.iter().collect() } else { Vec::new() }).find(|r| r.id == format!("source:{id}")).unwrap();
        assert_eq!((row.detail.as_deref(), row.tone), (Some("Couldn't refresh: timed out"), Tone::Warning));
        assert!(matches!(&row.parts[2], Part::Button { label, .. } if label == "Try again"));
        act(&mut state, &Target::new(&format!("source:{id}"), 2), Act::Press);
        assert_eq!(calendar::sources(state.files.doc(File::Calendar)).len(), 1);
        act(&mut state, &Target::new(&format!("source:{id}"), 0), Act::Press);
        assert!(!calendar::sources(state.files.doc(File::Calendar))[0].on);
        act(&mut state, &Target::new(&format!("source:{id}"), 1), Act::Press);
        assert!(calendar::sources(state.files.doc(File::Calendar)).is_empty());
        act(&mut state, &Target::new("lead", 0), Act::Choose(0));
        assert_eq!(state.files.doc(File::Calendar).get("lead_min").and_then(Value::as_u64), Some(5));
        assert!(matches!(build(&state)[0], Block::Intro { .. }));
        _ = std::fs::remove_dir_all(&dir);
    }
}
