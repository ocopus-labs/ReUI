//! Updates (PRD product shell, P4): the island's side of them. The networking lives in
//! Settings (`reui-settings --check-updates` asks GitHub for the latest release and notes
//! it in `%LOCALAPPDATA%\ReUI\update.json`); this module only decides when to run that and
//! what to say:
//!
//! - **Daily check**, when "Check automatically" is on (`check_updates` in `settings.json`,
//!   default on): ten minutes after start and then hourly it looks at `checked_ms`, and
//!   when the last check is a day old it runs the check. No network here, none at all
//!   when it's off.
//! - **Update available**, once per version: a quiet Brief with a button to Settings'
//!   About, where Install is (`announced` remembers it).
//! - **Updated**, once, on the first start of a new version: a Brief with "What's new"
//!   (unless `whats_new` is off); `version` / `updated_from` tell Settings to show the
//!   CHANGELOG entry.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Duration;

use reui_protocol::{Button, ButtonContent, ButtonRole, CompactView, Element, ExpandedView, Headline, Priority, Row, Tone};
use serde_json::{Map, Value};
use tokio::time::{sleep, Instant};

use super::Module;
use crate::hub::{ModuleContext, ModuleInput, View};
use crate::paths;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const STATE_FILE: &str = "update.json";
const DAY_MS: u64 = 24 * 60 * 60 * 1000;
/// The first look waits until the PC has settled after sign-in.
const FIRST_LOOK: Duration = Duration::from_secs(10 * 60);
const LOOK_EVERY: Duration = Duration::from_secs(60 * 60);
const CHECK_TIMEOUT: Duration = Duration::from_secs(120);
const SHOW: Duration = Duration::from_secs(8);
const GLYPH_DOWNLOAD: char = '\u{E896}';
const GLYPH_NEW: char = '\u{E735}';

const AVAILABLE: &str = "available";
const UPDATED: &str = "updated";

pub struct Updates;

impl Module for Updates {
    fn name(&self) -> &'static str {
        "updates"
    }

    fn run(self: Box<Self>, ctx: ModuleContext) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(run(ctx))
    }
}

fn read(path: &Path) -> Map<String, Value> {
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|v| v.as_object().cloned()).unwrap_or_default()
}

/// Merges `pairs` into `update.json` (Settings writes other keys of it).
fn write(pairs: Vec<(&str, Value)>) {
    let path = paths::local().join(STATE_FILE);
    let mut map = read(&path);
    for (k, v) in pairs {
        map.insert(k.into(), v);
    }
    let temp = path.with_extension("json.tmp");
    let result = std::fs::create_dir_all(paths::local())
        .and_then(|_| std::fs::write(&temp, serde_json::to_string_pretty(&Value::Object(map)).unwrap_or_default() + "\n"))
        .and_then(|_| std::fs::rename(&temp, &path));
    if let Err(e) = result {
        eprintln!("updates: can't write {}: {e}", path.display());
    }
}

/// A key of `settings.json` that defaults to on.
fn setting_on(key: &str) -> bool {
    read(&paths::config().join("settings.json")).get(key).and_then(Value::as_bool).unwrap_or(true)
}

/// This start against the last: the keys to write, and the version it was updated from.
fn version_change(state: &Map<String, Value>, current: &str) -> (Vec<(&'static str, Value)>, Option<String>) {
    match state.get("version").and_then(Value::as_str) {
        Some(last) if last == current => (Vec::new(), None),
        Some(last) => (vec![("version", Value::from(current)), ("updated_from", Value::from(last))], Some(last.to_string())),
        // A first install has nothing new to tell.
        None => (vec![("version", Value::from(current))], None),
    }
}

/// Whether the daily check is due.
fn check_due(state: &Map<String, Value>, now_ms: u64, enabled: bool) -> bool {
    enabled && state.get("checked_ms").and_then(Value::as_u64).is_none_or(|t| now_ms.saturating_sub(t) >= DAY_MS)
}

/// The version found by a check that the island hasn't mentioned yet.
fn to_announce(state: &Map<String, Value>) -> Option<String> {
    let available = state.get("available")?.as_str()?;
    (available != VERSION && state.get("announced").and_then(Value::as_str) != Some(available)).then(|| available.to_string())
}

fn settings_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.with_file_name("reui-settings.exe");
    exe.exists().then_some(exe)
}

/// Opens Settings on About.
fn open_about() {
    if let Some(exe) = settings_exe() {
        _ = std::process::Command::new(exe).args(["--page", "about"]).spawn();
    }
}

/// Runs `reui-settings --check-updates` and waits for it.
async fn run_check() {
    let Some(exe) = settings_exe() else { return };
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let child = tokio::process::Command::new(exe).arg("--check-updates").creation_flags(CREATE_NO_WINDOW).kill_on_drop(true).spawn();
    match child {
        Ok(mut child) => {
            if tokio::time::timeout(CHECK_TIMEOUT, child.wait()).await.is_err() {
                eprintln!("updates: the check took too long");
            }
        }
        Err(e) => eprintln!("updates: can't run the check: {e}"),
    }
}

fn brief(glyph: char, compact: &str, headline: String, button: &str) -> View {
    let art = Element::Glyph { glyph, tone: Tone::Secondary };
    View {
        accent: None,
        audio: None,
        compact: CompactView { leading: art.clone(), trailing: Element::Text { text: compact.into(), tone: Tone::Secondary } },
        minimal: art.clone(),
        expanded: Some(ExpandedView {
            leading: art,
            eyebrow: Some("ReUI".into()),
            headline: Headline::Text(headline),
            detail: None,
            trailing: Element::Empty,
            wide: false,
            tint: false,
            pager: None,
            open: None,
            rows: vec![Row::Buttons(vec![Button { action: "open".into(), content: ButtonContent::Text(button.into()), role: ButtonRole::Primary }])],
        }),
    }
}

async fn run(mut ctx: ModuleContext) {
    let state = read(&paths::local().join(STATE_FILE));
    let (pairs, updated_from) = version_change(&state, VERSION);
    if !pairs.is_empty() {
        write(pairs);
    }
    // Shown until this time, then retracted.
    let mut showing: Vec<(&str, Instant)> = Vec::new();
    if updated_from.is_some() && setting_on("whats_new") {
        ctx.publish(UPDATED, Priority::Brief, brief(GLYPH_NEW, "Updated", format!("Updated to ReUI {VERSION}"), "What's new"));
        showing.push((UPDATED, Instant::now() + SHOW));
    }

    let mut next_look = Instant::now() + FIRST_LOOK;
    loop {
        let wake = showing.iter().map(|(_, t)| *t).chain([next_look]).min().unwrap_or(next_look);
        tokio::select! {
            input = ctx.next() => match input {
                Some(ModuleInput::Action(action)) if action.action == "open" => {
                    open_about();
                    ctx.retract(&action.key);
                    showing.retain(|(k, _)| *k != action.key);
                }
                Some(_) => {}
                None => break,
            },
            _ = sleep(wake.saturating_duration_since(Instant::now())) => {
                let now = Instant::now();
                showing.retain(|(key, until)| {
                    let done = *until <= now;
                    if done {
                        ctx.retract(key);
                    }
                    !done
                });
                if next_look > now {
                    continue;
                }
                next_look = now + LOOK_EVERY;
                if !check_due(&read(&paths::local().join(STATE_FILE)), reui_protocol::unix_ms(), setting_on("check_updates")) {
                    continue;
                }
                run_check().await;
                if let Some(version) = to_announce(&read(&paths::local().join(STATE_FILE))) {
                    write(vec![("announced", Value::from(version.clone()))]);
                    ctx.publish(AVAILABLE, Priority::Brief, brief(GLYPH_DOWNLOAD, "Update available", format!("ReUI {version} is available"), "View"));
                    showing.retain(|(k, _)| *k != AVAILABLE);
                    showing.push((AVAILABLE, Instant::now() + SHOW));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn a_new_version_is_noticed_once_and_a_first_install_isnt_an_update() {
        assert_eq!(version_change(&map(json!({})), "0.2.0"), (vec![("version", json!("0.2.0"))], None));
        let (pairs, from) = version_change(&map(json!({ "version": "0.1.0" })), "0.2.0");
        assert_eq!(from.as_deref(), Some("0.1.0"));
        assert_eq!(pairs, vec![("version", json!("0.2.0")), ("updated_from", json!("0.1.0"))]);
        assert_eq!(version_change(&map(json!({ "version": "0.2.0", "updated_from": "0.1.0" })), "0.2.0"), (Vec::new(), None));
    }

    #[test]
    fn the_daily_check_runs_once_a_day_and_only_when_on() {
        let now = 10 * DAY_MS;
        assert!(check_due(&map(json!({})), now, true));
        assert!(!check_due(&map(json!({})), now, false));
        assert!(!check_due(&map(json!({ "checked_ms": now - DAY_MS / 2 })), now, true));
        assert!(check_due(&map(json!({ "checked_ms": now - DAY_MS })), now, true));
    }

    #[test]
    fn each_found_version_is_announced_once() {
        assert_eq!(to_announce(&map(json!({ "available": "9.0.0" }))).as_deref(), Some("9.0.0"));
        assert_eq!(to_announce(&map(json!({ "available": "9.0.0", "announced": "9.0.0" }))), None);
        assert_eq!(to_announce(&map(json!({ "available": "9.1.0", "announced": "9.0.0" }))).as_deref(), Some("9.1.0"));
        assert_eq!(to_announce(&map(json!({ "available": null }))), None);
        assert_eq!(to_announce(&map(json!({ "available": VERSION }))), None);
    }
}
