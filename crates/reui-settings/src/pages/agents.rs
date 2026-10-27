//! Coding agents: connect or disconnect each, and the file that changes.

use super::{button, group, intro, Effect};
use crate::model::agents::{self, Agent, Status};
use crate::state::State;
use crate::ui::{Act, Block, Role, Row, Target, Tone};

pub fn build(state: &State) -> Vec<Block> {
    let mut blocks = vec![intro("Coding agents", "Claude Code, Codex and OpenCode sessions in the island: what they're working on, when they finish, and permission requests you can answer there.")];
    let bridge = state.paths.installed_bridge().exists() || state.paths.exe_dir.join("reui-bridge.exe").exists();
    let rows = Agent::ALL
        .iter()
        .enumerate()
        .map(|(i, &agent)| {
            let file = state.paths.short(&agent.file(&state.paths));
            let row = Row::new(&format!("agent:{}", agent.arg()), agent.name());
            let row = match state.agents[i] {
                Status::Connected => row.detail(format!("Connected · {file}")).tone(Tone::Positive).part(button("Disconnect", Role::Secondary)),
                Status::NotConnected => row.detail(format!("Adds ReUI to {file}")).part(button("Connect", Role::Accent)),
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
    let connect = state.agents[i] != Status::Connected;
    let note = match agents::bridge(&state.paths) {
        None => Some(("reui-bridge.exe isn't next to Settings".to_string(), Tone::Warning)),
        Some(bridge) => match agents::run(&bridge, agent, connect, &state.paths) {
            // Codex ignores new hooks until they're trusted in Codex itself.
            Ok(_) if connect && agent == Agent::Codex => Some(("Connected · open Codex and run /hooks to trust ReUI".to_string(), Tone::Warning)),
            Ok(_) if connect && agent == Agent::OpenCode => Some(("Connected · OpenCode picks it up on its next start".to_string(), Tone::Positive)),
            Ok(_) => None,
            Err(e) => Some((e, Tone::Negative)),
        },
    };
    state.agent_notes[i] = note;
    state.agents[i] = agent.status(&state.paths);
    Effect::None
}
