//! The island's hide shortcut (Win+Alt+I by default): hides the island until it's pressed
//! again, without pausing anything else (modules and clipboard recording carry on, and
//! decisions still come through, as during a pause).
//!
//! Hotkeys live hub-side on the shared system-events window, like the timers' and the
//! stats card's, so the island needs no keyboard hook for it; the island keeps the hidden
//! state (`hidden` in settings.json) and this only relays the press as
//! `HubMsg::ToggleHidden`. Win+Alt+I, not Win+Alt+H: Windows uses Win+Alt+H for voice
//! typing.
//!
//! Module `island`, no activities. Commands: `toggle-hidden` (`reui-bridge hide`, and how
//! scripts test it without pressing keys), `reload` (the Settings window changed
//! `hide_shortcut` in settings.json).

use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use serde_json::Value;
use tokio::sync::mpsc;

use super::hotkey::Hotkey;
use super::system_events::{self, SystemEvent};
use super::{supervise, Module};
use crate::hub::{ModuleAction, ModuleContext, ModuleInput};

pub const DEFAULT_SHORTCUT: &str = "Win+Alt+I";
/// The shortcut's id on the shared system-events window.
const HOTKEY_ID: i32 = 0x7301;

pub struct HideIsland;

impl Module for HideIsland {
    fn name(&self) -> &'static str {
        "island"
    }

    fn run(self: Box<Self>, mut ctx: ModuleContext) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async move { supervise("island", &mut ctx, |ctx| Box::pin(run(ctx))).await })
    }
}

async fn run(ctx: &mut ModuleContext) -> Result<(), String> {
    let (events_tx, mut events) = mpsc::unbounded_channel();
    // The first subscriber creates the window, which blocks for a moment.
    let subscription = tokio::task::spawn_blocking(move || system_events::subscribe(events_tx)).await.ok().flatten();
    let mut registered: Option<Hotkey> = None;
    let register = |registered: &mut Option<Hotkey>, wanted: Option<Hotkey>| {
        if wanted != *registered {
            if let Some(events) = &subscription {
                events.hotkey(HOTKEY_ID, wanted.as_ref());
            }
            *registered = wanted;
        }
    };
    register(&mut registered, load().await);
    loop {
        tokio::select! {
            input = ctx.next() => match input {
                Some(ModuleInput::Action(ModuleAction { key, action })) if key.is_empty() => match action.as_str() {
                    "toggle-hidden" => ctx.toggle_hidden(),
                    "reload" => register(&mut registered, load().await),
                    _ => {}
                },
                Some(_) => {}
                None => return Ok(()),
            },
            Some(event) = events.recv() => match event {
                SystemEvent::Hotkey { id: HOTKEY_ID } => ctx.toggle_hidden(),
                SystemEvent::HotkeyTaken { id: HOTKEY_ID } => eprintln!("island: the hide shortcut is taken by another app"),
                _ => {}
            },
        }
    }
}

/// The shortcut from settings.json, read off the hub thread.
async fn load() -> Option<Hotkey> {
    let path = crate::paths::config().join("settings.json");
    tokio::task::spawn_blocking(move || shortcut(&path)).await.ok().flatten()
}

/// `hide_shortcut` from the island's settings file: the default when the file or the key
/// is missing or unreadable, none when it's set to "" (turned off).
fn shortcut(path: &Path) -> Option<Hotkey> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    parse(&text)
}

fn parse(text: &str) -> Option<Hotkey> {
    let value: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).unwrap_or_default();
    match value.get("hide_shortcut").and_then(Value::as_str).map(str::trim) {
        Some("") => None,
        Some(label) => Hotkey::parse(label).or_else(|| Hotkey::parse(DEFAULT_SHORTCUT)),
        None => Hotkey::parse(DEFAULT_SHORTCUT),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shortcut_is_read_from_the_islands_settings() {
        let default = Hotkey::parse(DEFAULT_SHORTCUT).unwrap();
        assert_eq!(default.label, "Win+Alt+I");
        assert_eq!(parse(""), Some(default.clone()), "no file");
        assert_eq!(parse(r#"{ "visibility": "always" }"#), Some(default.clone()));
        assert_eq!(parse(r#"{ "hide_shortcut": "ctrl+alt+h" }"#).map(|h| h.label), Some("Ctrl+Alt+H".into()));
        assert_eq!(parse(r#"{ "hide_shortcut": "nonsense+" }"#), Some(default), "unreadable: the default");
        assert_eq!(parse(r#"{ "hide_shortcut": "" }"#), None, "off");
    }
}
