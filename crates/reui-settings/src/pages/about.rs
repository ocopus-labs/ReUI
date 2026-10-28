//! About: version and updates, What's new after an update, the idle cost measured live,
//! help (guide, the tour, logs, reporting a bug), licenses.

use super::{button, flag, group, Effect, Page};
use crate::model::agents::ago;
use crate::model::files::File;
use crate::state::State;
use crate::stats::{self, PROCESSES};
use crate::system;
use crate::ui::{icons, Act, Block, Icon, Part, Role, Row, Target, Tone};
use crate::updates::{self, Phase};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn build(state: &State) -> Vec<Block> {
    let mut blocks = vec![Block::Intro { eyebrow: None, title: "ReUI".into(), body: Some(format!("Version {VERSION} · Windows build {}", state.windows_build)), icon: Some(Icon::Mark), preview: None, row: None }];
    if let Some(text) = state.whats_new.as_ref().filter(|_| state.files.doc(File::Island).bool("whats_new")) {
        blocks.push(Block::Text { label: Some("What's new".into()), title: format!("ReUI {VERSION}"), body: text.clone(), accent: true });
    }
    blocks.extend(update_blocks(state));

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

    let repo = updates::repository_set();
    let guide = Row::new("guide", "Guide").detail(if repo { "Every feature and shortcut, on GitHub" } else { "Opens once the GitHub repository is set" }).part(button("Open", Role::Secondary)).enabled(repo).icon(icons::help(icons::HELP));
    blocks.push(group(
        "Help",
        vec![
            guide,
            Row::new("tour", "Replay the tour").icon(icons::welcome()).detail("The three welcome steps again").part(button("Replay", Role::Secondary)),
            Row::new("logs", "Logs").icon(icons::gray(icons::PAGE)).detail(state.paths.local.display().to_string()).part(button("Open", Role::Secondary)),
            Row::new("bug", "Report a bug").icon(icons::danger(icons::BUG)).detail("A GitHub issue with the version and Windows build, nothing else").part(button("Report", Role::Secondary)),
        ],
    ));
    blocks.push(group("", vec![Row::new("licenses", "Licenses").icon(icons::gray(icons::DOCUMENT)).detail("Built with open-source Rust crates under the MIT and Apache-2.0 licenses")]));
    blocks
}

/// The Updates group, and the release notes while an update is on offer.
fn update_blocks(state: &State) -> Vec<Block> {
    let checked = updates::checked_ms(&updates::read_state(&state.paths)).map(|t| ago(reui_protocol::unix_ms(), t));
    let check = |title: &str, detail: String, tone: Tone| Row::new("update", title).detail(detail).tone(tone).part(button("Check", Role::Secondary));
    let mut notes = None;
    let row = match state.updates.phase() {
        _ if !updates::repository_set() => Row::new("update", "Check for updates").detail("Update checks start once the GitHub repository is set"),
        Phase::Idle => check("Check for updates", checked.map_or_else(|| "Not checked yet".into(), |t| format!("Last checked {t}")), Tone::Secondary),
        Phase::Checking => check("Check for updates", "Checking…".into(), Tone::Secondary).enabled(false),
        Phase::UpToDate => check("ReUI is up to date", format!("Checked {}", checked.unwrap_or_else(|| "just now".into())), Tone::Secondary),
        Phase::Failed(e) => check("Check for updates", e, Tone::Negative),
        Phase::Available(release) => {
            let size = release.setup.as_ref().map(|s| s.size).filter(|&s| s > 0).map_or_else(String::new, |s| format!(" · {}", system::bytes(s)));
            notes = Some(release.clone());
            match release.setup {
                Some(_) => Row::new("update", &format!("ReUI {} is available", release.version)).detail(format!("Downloads the installer{size}")).tone(Tone::Positive).part(button("Install", Role::Primary)),
                None => Row::new("update", &format!("ReUI {} is available", release.version)).detail("This release has no installer yet").tone(Tone::Warning),
            }
        }
        Phase::Downloading { release, done } => {
            let total = release.setup.as_ref().map_or(0, |s| s.size);
            let progress = if total > 0 { format!("Downloading · {} %", (done * 100 / total).min(100)) } else { format!("Downloading · {}", system::bytes(done)) };
            notes = Some(release.clone());
            Row::new("update", &format!("ReUI {} is available", release.version)).detail(progress).part(button("Install", Role::Primary)).enabled(false)
        }
        Phase::Ready(_) => Row::new("update", "Installing").detail("ReUI restarts in a moment").part(button("Install", Role::Primary)).enabled(false),
    };
    let rows = vec![
        row,
        flag(state, File::Island, "check_updates", "Check automatically", "Once a day, from api.github.com"),
        flag(state, File::Island, "whats_new", "What's new after updates", "The new version's changes, once, here"),
    ];
    let mut blocks = vec![group("Updates", rows)];
    if let Some(release) = notes.filter(|r| !r.notes.is_empty()) {
        blocks.push(Block::Text { label: None, title: format!("What's new in {}", release.version), body: release.notes, accent: false });
    }
    blocks
}

/// The new-issue link, prefilled with diagnostics that identify no one.
pub fn bug_url(build: &str) -> String {
    let body = format!("**What happened**\n\n\n**What you expected**\n\n\n---\nReUI {VERSION} · Windows build {build}\n");
    format!("{}issues/new?body={}", updates::repository_url(), system::url_encode(&body))
}

/// The guide: `docs/guide.md` in the repository.
pub fn guide_url() -> String {
    format!("{}blob/main/docs/guide.md", updates::repository_url())
}

pub fn act(state: &mut State, target: &Target, act: Act) -> Effect {
    if act != Act::Press {
        return Effect::None;
    }
    match target.row.as_str() {
        "update" => match state.updates.phase() {
            Phase::Available(_) => return Effect::Download,
            Phase::Idle | Phase::UpToDate | Phase::Failed(_) => return Effect::CheckUpdates,
            _ => {}
        },
        "guide" => system::open(&guide_url()),
        "tour" => return Effect::Navigate(Page::WelcomeFeatures),
        "logs" => system::open_folder(&state.paths.local),
        "bug" => system::open(&bug_url(&state.windows_build)),
        _ => {}
    }
    Effect::None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::tests::state;

    #[test]
    fn bug_reports_carry_only_version_and_build() {
        let url = bug_url("26200.1");
        assert!(url.starts_with(&updates::repository_url()));
        assert!(url.contains(&system::url_encode(&format!("ReUI {VERSION}"))));
        assert!(url.contains("26200.1"));
        assert!(!url.contains("Users"));
    }

    #[test]
    fn without_a_repository_updates_say_so_and_never_check() {
        let (mut state, dir) = state("about-updates");
        let blocks = build(&state);
        let rows: Vec<&Row> = blocks.iter().flat_map(|b| if let Block::Group { rows, .. } = b { rows.iter().collect() } else { Vec::new() }).collect();
        let update = rows.iter().find(|r| r.id == "update").unwrap();
        assert_eq!(update.detail.as_deref(), Some("Update checks start once the GitHub repository is set"));
        assert!(update.parts.is_empty());
        assert!(!rows.iter().find(|r| r.id == "guide").unwrap().enabled);
        assert_eq!(act(&mut state, &Target::new("tour", 0), Act::Press), Effect::Navigate(Page::WelcomeFeatures));
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn whats_new_shows_as_a_card_until_turned_off() {
        let (mut state, dir) = state("about-whats-new");
        state.whats_new = Some("• Updates".into());
        assert!(build(&state).iter().any(|b| matches!(b, Block::Text { accent: true, .. })));
        crate::pages::act(Page::About, &mut state, &Target::new("island:whats_new", 0), Act::Press);
        assert!(!build(&state).iter().any(|b| matches!(b, Block::Text { accent: true, .. })));
        _ = std::fs::remove_dir_all(&dir);
    }
}
