//! Coding agents: per agent, its status (connected, when ReUI last heard from it, or that
//! it isn't on this PC), Connect / Disconnect with the file that changes, and a test event;
//! then what the island shows of sessions (`coding.json`).

use serde_json::Value;

use super::{button, file_error, flag, group, intro, Effect, Page};
use crate::model::agents::{self, Agent, Status};
use crate::model::files::{File, SHOW_SESSIONS};
use crate::state::State;
use crate::ui::{icons, Act, Block, Icon, Part, Role, Row, Target, Tone};

pub fn build(state: &State) -> Vec<Block> {
    let mut blocks = vec![intro(state, Page::Agents, "Claude Code, Codex and OpenCode in the island when they need you: permission requests you can answer there, questions, and turns that failed.")];
    blocks.extend(file_error(state, File::Coding));
    let bridge = state.paths.installed_bridge().exists() || state.paths.exe_dir.join("reui-bridge.exe").exists();
    let now = reui_protocol::unix_ms();
    let rows = Agent::ALL
        .iter()
        .enumerate()
        .map(|(i, &agent)| {
            let file = state.paths.short(&agent.file(&state.paths));
            let row = Row::new(&format!("agent:{}", agent.arg()), agent.name()).icon(if agent == Agent::Claude { Icon::Claude } else { icons::gray(icons::CODE) });
            let installed = state.agent_installed[i];
            let line = agents::status_line(state.agents[i], installed, state.agent_last[i], now);
            let row = match state.agents[i] {
                Status::Connected => row.detail(format!("{line} · {file}")).tone(Tone::Positive).part(button("Send test", Role::Secondary)).part(button("Disconnect", Role::Secondary)),
                // Questions and plans need the newer hooks; installing again replaces ours.
                Status::Outdated => row.detail(format!("{line} · questions need newer hooks")).tone(Tone::Warning).part(button("Update hooks", Role::Accent)).part(button("Disconnect", Role::Secondary)),
                Status::NotConnected if installed => row.detail(format!("{line} · adds ReUI to {file}")).part(button("Connect", Role::Accent)),
                Status::NotConnected => row.detail(line).part(button("Connect", Role::Secondary)),
                Status::Unreadable => row.detail(format!("{file} isn't valid JSON")).tone(Tone::Negative).part(button("Connect", Role::Accent)).enabled(false),
            };
            let row = match &state.agent_notes[i] {
                Some((note, tone)) => Row { detail: Some(note.clone()), tone: *tone, ..row },
                None => row,
            };
            if bridge {
                row
            } else {
                Row { detail: Some("reui-bridge.exe isn't next to Settings".into()), tone: Tone::Warning, ..row.enabled(false) }
            }
        })
        .collect();
    blocks.push(group("", rows));

    let ok = state.files.doc(File::Coding).error().is_none();
    let selected = state.files.show_sessions();
    let detail = ["Only for permissions, questions and failed turns", "Also while they think and run tools"][selected];
    let sessions = Row::new("sessions", "Show sessions").detail(detail).part(Part::Segmented { options: vec!["When needed".into(), "While working".into()], selected }).enabled(ok);
    let finished = flag(state, File::Coding, "show_finished", "Finished turns", "A moment when a turn ends, with how long it took");
    blocks.push(group("In the island", vec![sessions, finished]));
    blocks
}

pub fn act(state: &mut State, target: &Target, act: Act) -> Effect {
    if let ("sessions", &Act::Choose(i)) = (target.row.as_str(), &act) {
        if let Some(value) = SHOW_SESSIONS.get(i) {
            state.set(File::Coding, "show_sessions", Value::String((*value).into()));
        }
        return Effect::None;
    }
    let Some(arg) = target.row.strip_prefix("agent:") else { return Effect::None };
    let Some(i) = Agent::ALL.iter().position(|a| a.arg() == arg) else { return Effect::None };
    if act != Act::Press {
        return Effect::None;
    }
    let agent = Agent::ALL[i];
    let status = state.agents[i];
    // Connected rows: Send test, Disconnect. Outdated: Update hooks (connect again),
    // Disconnect. Others: Connect.
    let connected = match status {
        Status::Outdated => target.part == 1,
        s => s == Status::Connected,
    };
    let test = status == Status::Connected && target.part == 0;
    let note = match agents::bridge(&state.paths) {
        None => Some(("reui-bridge.exe isn't next to Settings".to_string(), Tone::Warning)),
        Some(bridge) if test => match agents::send_test(&bridge, agent, &state.paths) {
            Ok(_) => Some(("Test event sent · the island shows it now".to_string(), Tone::Positive)),
            Err(e) => Some((e, Tone::Negative)),
        },
        Some(bridge) => match agents::run(&bridge, agent, !connected, &state.paths) {
            // Codex ignores new hooks until they're trusted in Codex itself.
            Ok(_) if !connected && agent == Agent::Codex => Some(("Connected · open Codex and run /hooks to trust ReUI".to_string(), Tone::Warning)),
            Ok(_) if !connected && agent == Agent::OpenCode => Some(("Connected · OpenCode picks it up on its next start".to_string(), Tone::Positive)),
            Ok(_) => None,
            Err(e) => Some((e, Tone::Negative)),
        },
    };
    state.agent_notes[i] = note;
    state.agents[i] = agent.status(&state.paths);
    Effect::None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::tests::state;
    use crate::ui::Part;

    fn row<'a>(blocks: &'a [Block], id: &str) -> &'a Row {
        blocks.iter().find_map(|b| if let Block::Group { rows, .. } = b { rows.iter().find(|r| r.id == id) } else { None }).unwrap()
    }

    #[test]
    fn rows_say_whether_the_agent_is_here_connected_and_heard_from() {
        let (mut state, dir) = state("agents-page");
        let blocks = build(&state);
        assert!(row(&blocks, "agent:claude").detail.as_deref().unwrap().starts_with("reui-bridge.exe"), "no bridge in the sandbox");
        std::fs::write(dir.join("reui-bridge.exe"), b"").unwrap();
        std::fs::create_dir_all(dir.join("home").join(".claude")).unwrap();
        std::fs::write(dir.join("home").join(".claude").join("settings.json"), r#"{ "hooks": { "Stop": [{ "hooks": [{ "command": "reui-bridge.exe" }] }] } }"#).unwrap();
        state.refresh();
        let blocks = build(&state);
        let claude = row(&blocks, "agent:claude");
        assert!(claude.detail.as_deref().unwrap().ends_with("questions need newer hooks"), "an older build's hooks");
        assert!(matches!(&claude.parts[0], Part::Button { label, .. } if label == "Update hooks"));
        std::fs::write(dir.join("home").join(".claude").join("settings.json"), r#"{ "hooks": { "PreToolUse": [{ "matcher": "AskUserQuestion|ExitPlanMode", "hooks": [{ "command": "reui-bridge.exe", "args": ["claude", "ask"] }] }] } }"#).unwrap();
        state.refresh();
        let blocks = build(&state);
        let claude = row(&blocks, "agent:claude");
        assert!(claude.detail.as_deref().unwrap().starts_with("Connected · no events yet"));
        assert_eq!(claude.parts.len(), 2, "Send test and Disconnect");
        assert!(matches!(&claude.parts[0], Part::Button { label, .. } if label == "Send test"));
        assert_eq!(row(&blocks, "agent:codex").detail.as_deref(), Some("Agent not found on this PC"));

        std::fs::create_dir_all(dir.join("local")).unwrap();
        std::fs::write(dir.join("local").join(agents::STATUS_FILE), format!(r#"{{ "claude": {{ "last_event_ms": {} }} }}"#, reui_protocol::unix_ms() - 125_000)).unwrap();
        state.refresh();
        let blocks = build(&state);
        assert!(row(&blocks, "agent:claude").detail.as_deref().unwrap().starts_with("Connected · last event 2 min ago"));
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn session_rows_write_coding_json() {
        let (mut state, dir) = state("agents-coding");
        let blocks = build(&state);
        assert!(matches!(row(&blocks, "sessions").parts[0], Part::Segmented { selected: 0, .. }), "only when needed by default");
        act(&mut state, &Target::new("sessions", 0), Act::Choose(1));
        crate::pages::act(Page::Agents, &mut state, &Target::new("coding:show_finished", 0), Act::Press);
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("roaming").join("coding.json")).unwrap()).unwrap();
        assert_eq!((saved["show_sessions"].as_str(), saved["show_finished"].as_bool()), (Some("working"), Some(true)));
        _ = std::fs::remove_dir_all(&dir);
    }
}
