//! The island's own shortcuts. **Hide** (Win+Alt+I by default) hides the island until it's
//! pressed again, without pausing anything else (modules and clipboard recording carry on,
//! and decisions still come through, as during a pause). **Open** (Win+Alt+N by default)
//! opens whatever is waiting with the keyboard on it, a decision first, or the row of
//! places when nothing is (docs/PRD-keyboard-island.md).
//!
//! Hotkeys live hub-side on the shared system-events window, like the timers' and the
//! stats card's, so the island needs no keyboard hook for them; the island keeps the state
//! (`hidden` in settings.json, what's open) and this only relays a press as
//! `HubMsg::ToggleHidden` or `HubMsg::OpenIsland`. Win+Alt+I, not Win+Alt+H: Windows uses
//! Win+Alt+H for voice typing.
//!
//! Module `island`, no activities. Commands: `toggle-hidden` (`reui-bridge hide`, and how
//! scripts test it without pressing keys), `open` (`reui-bridge open`), `reload` (the
//! Settings window changed `hide_shortcut` or `open_shortcut` in settings.json).

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
pub const DEFAULT_OPEN_SHORTCUT: &str = "Win+Alt+N";
/// The shortcuts' ids on the shared system-events window.
const HOTKEY_ID: i32 = 0x7301;
const HOTKEY_OPEN: i32 = 0x7302;

pub struct HideIsland;

impl Module for HideIsland {
    fn name(&self) -> &'static str {
        "island"
    }

    fn run(self: Box<Self>, mut ctx: ModuleContext) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async move { supervise("island", &mut ctx, |ctx| Box::pin(run(ctx))).await })
    }
}

/// Both shortcuts as settings.json has them.
#[derive(Clone, Debug, PartialEq, Default)]
struct Shortcuts {
    hide: Option<Hotkey>,
    open: Option<Hotkey>,
}

async fn run(ctx: &mut ModuleContext) -> Result<(), String> {
    let (events_tx, mut events) = mpsc::unbounded_channel();
    // The first subscriber creates the window, which blocks for a moment.
    let subscription = tokio::task::spawn_blocking(move || system_events::subscribe(events_tx)).await.ok().flatten();
    let mut registered = Shortcuts::default();
    let register = |registered: &mut Shortcuts, wanted: Shortcuts| {
        if let Some(events) = &subscription {
            if wanted.hide != registered.hide {
                events.hotkey(HOTKEY_ID, wanted.hide.as_ref());
            }
            if wanted.open != registered.open {
                events.hotkey(HOTKEY_OPEN, wanted.open.as_ref());
            }
        }
        *registered = wanted;
    };
    register(&mut registered, load().await);
    loop {
        tokio::select! {
            input = ctx.next() => match input {
                Some(ModuleInput::Action(ModuleAction { key, action })) if key.is_empty() => match action.as_str() {
                    "toggle-hidden" => ctx.toggle_hidden(),
                    "open" => ctx.open_island(),
                    "reload" => register(&mut registered, load().await),
                    _ => {}
                },
                Some(_) => {}
                None => return Ok(()),
            },
            Some(event) = events.recv() => match event {
                SystemEvent::Hotkey { id: HOTKEY_ID } => ctx.toggle_hidden(),
                SystemEvent::Hotkey { id: HOTKEY_OPEN } => ctx.open_island(),
                SystemEvent::HotkeyTaken { id: HOTKEY_ID } => eprintln!("island: the hide shortcut is taken by another app"),
                SystemEvent::HotkeyTaken { id: HOTKEY_OPEN } => eprintln!("island: the open shortcut is taken by another app"),
                _ => {}
            },
        }
    }
}

/// The shortcuts from settings.json, read off the hub thread.
async fn load() -> Shortcuts {
    let path = crate::paths::config().join("settings.json");
    tokio::task::spawn_blocking(move || shortcuts(&path)).await.unwrap_or_default()
}

fn shortcuts(path: &Path) -> Shortcuts {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    parse(&text)
}

/// `hide_shortcut` and `open_shortcut` from the island's settings file: the default when
/// the file or the key is missing or unreadable, none when it's set to "" (turned off).
fn parse(text: &str) -> Shortcuts {
    let value: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).unwrap_or_default();
    let read = |key: &str, default: &str| match value.get(key).and_then(Value::as_str).map(str::trim) {
        Some("") => None,
        Some(label) => Hotkey::parse(label).or_else(|| Hotkey::parse(default)),
        None => Hotkey::parse(default),
    };
    Shortcuts { hide: read("hide_shortcut", DEFAULT_SHORTCUT), open: read("open_shortcut", DEFAULT_OPEN_SHORTCUT) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shortcuts_are_read_from_the_islands_settings() {
        let hide = Hotkey::parse(DEFAULT_SHORTCUT).unwrap();
        let open = Hotkey::parse(DEFAULT_OPEN_SHORTCUT).unwrap();
        assert_eq!((hide.label.as_str(), open.label.as_str()), ("Win+Alt+I", "Win+Alt+N"));
        let defaults = Shortcuts { hide: Some(hide.clone()), open: Some(open.clone()) };
        assert_eq!(parse(""), defaults, "no file");
        assert_eq!(parse(r#"{ "visibility": "always" }"#), defaults);
        assert_eq!(parse(r#"{ "hide_shortcut": "ctrl+alt+h" }"#).hide.map(|h| h.label), Some("Ctrl+Alt+H".into()));
        assert_eq!(parse(r#"{ "hide_shortcut": "nonsense+" }"#).hide, Some(hide), "unreadable: the default");
        assert_eq!(parse(r#"{ "hide_shortcut": "" }"#).hide, None, "off");
        assert_eq!(parse(r#"{ "open_shortcut": "" }"#), Shortcuts { hide: defaults.hide.clone(), open: None });
        assert_eq!(parse(r#"{ "open_shortcut": "win+shift+space" }"#).open.map(|h| h.label), Some("Win+Shift+Space".into()));
    }
}
