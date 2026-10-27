//! About: version, the idle cost measured live, logs, reporting a bug, licenses.

use super::{button, group, intro, Effect};
use crate::state::State;
use crate::stats::{self, PROCESSES};
use crate::system;
use crate::ui::{Act, Block, Part, Role, Row, Target};

/// Where "Report a bug" goes. TODO: the ReUI repository once it's public
/// (`https://github.com/<owner>/<repo>/`); GitHub asks for the repository until then.
pub const REPOSITORY: &str = "https://github.com/";

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn build(state: &State) -> Vec<Block> {
    let mut blocks = vec![intro("About", "")];
    blocks.push(group("", vec![Row::new("version", &format!("ReUI {VERSION}")).detail(format!("Windows build {}", state.windows_build))]));

    let total = state.stats.total();
    let cpu = total.cpu.map_or_else(|| "–".to_string(), stats::percent);
    blocks.push(Block::Figures { label: Some("Idle cost".into()), items: vec![(system::bytes(total.memory), "Memory".into()), (cpu, "CPU".into())] });
    let rows = PROCESSES
        .iter()
        .zip(&state.stats.usage)
        .map(|(name, usage)| {
            let value = match (usage.running, usage.cpu) {
                (false, _) => "Not running".to_string(),
                (true, Some(cpu)) => format!("{} · {}", system::bytes(usage.memory), stats::percent(cpu)),
                (true, None) => system::bytes(usage.memory),
            };
            Row::new(&format!("process:{name}"), name).mono().part(Part::Value(value))
        })
        .collect();
    blocks.push(group("", rows));

    blocks.push(group(
        "Help",
        vec![
            Row::new("logs", "Logs").detail(state.paths.local.display().to_string()).part(button("Open", Role::Secondary)),
            Row::new("bug", "Report a bug").detail("A GitHub issue with the version and Windows build, nothing else").part(button("Report", Role::Secondary)),
        ],
    ));
    blocks.push(group("", vec![Row::new("licenses", "Licenses").detail("Built with open-source Rust crates under the MIT and Apache-2.0 licenses")]));
    blocks
}

/// The new-issue link, prefilled with diagnostics that identify no one.
pub fn bug_url(build: &str) -> String {
    let body = format!("**What happened**\n\n\n**What you expected**\n\n\n---\nReUI {VERSION} · Windows build {build}\n");
    format!("{REPOSITORY}issues/new?body={}", system::url_encode(&body))
}

pub fn act(state: &mut State, target: &Target, act: Act) -> Effect {
    if act != Act::Press {
        return Effect::None;
    }
    match target.row.as_str() {
        "logs" => system::open_folder(&state.paths.local),
        "bug" => system::open(&bug_url(&state.windows_build)),
        _ => {}
    }
    Effect::None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bug_reports_carry_only_version_and_build() {
        let url = bug_url("26200.1");
        assert!(url.starts_with(REPOSITORY));
        assert!(url.contains(&system::url_encode(&format!("ReUI {VERSION}"))));
        assert!(url.contains("26200.1"));
        assert!(!url.contains("Users"));
    }
}
