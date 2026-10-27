//! Coding agents: per agent, its status (connected, when ReUI last heard from it, or that
//! it isn't on this PC), Connect / Disconnect with the file that changes, and a test event.

use super::{button, group, intro, Effect};
use crate::model::agents::{self, Agent, Status};
use crate::state::State;
use crate::ui::{Act, Block, Role, Row, Target, Tone};

pub fn build(state: &State) -> Vec<Block> {
    let mut blocks = vec![intro("Coding agents", "Claude Code, Codex and OpenCode sessions in the island: what they're working on, when they finish, and permission requests you can answer there.")];
    let bridge = state.paths.installed_bridge().exists() || state.paths.exe_dir.join("reui-bridge.exe").exists();
    let now = reui_protocol::unix_ms();
    let rows = Agent::ALL
        .iter()
        .enumerate()
        .map(|(i, &agent)| {
            let file = state.paths.short(&agent.file(&state.paths));
            let row = Row::new(&format!("agent:{}", agent.arg()), agent.name());
            let installed = state.agent_installed[i];
            let line = agents::status_line(state.agents[i], installed, state.agent_last[i], now);
            let row = match state.agents[i] {
                Status::Connected => row.detail(format!("{line} · {file}")).tone(Tone::Positive).part(button("Send test", Role::Secondary)).part(button("Disconnect", Role::Secondary)),
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
    blocks
}

pub fn act(state: &mut State, target: &Target, act: Act) -> Effect {
    let Some(arg) = target.row.strip_prefix("agent:") else { return Effect::None };
    let Some(i) = Agent::ALL.iter().position(|a| a.arg() == arg) else { return Effect::None };
    if act != Act::Press {
        return Effect::None;
    }
    let agent = Agent::ALL[i];
    let connected = state.agents[i] == Status::Connected;
    // Connected rows: Send test, Disconnect. Others: Connect.
    let test = connected && target.part == 0;
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
}
