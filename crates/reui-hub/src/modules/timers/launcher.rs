//! The timer field ("launcher", Passive, summoned by its hotkey, the tray or the view
//! switcher): what's typed is read live (a timer's length, focus, the stopwatch, a
//! reminder) and the hero previews what Enter will start; the presets start a timer at once.
//! Part of the timers module's state, split out so `mod.rs` stays the module loop.

use reui_protocol::{Priority, ACTION_COLLAPSED, ACTION_SEARCH, ACTION_SUBMIT, ACTION_UNDO};

use super::focus::Focus;
use super::parse::{self, Parsed};
use super::stopwatch::Stopwatch;
use super::{plan, remind, view, State, LAUNCHER};
use crate::hub::ModuleContext;

/// The open launcher: what's typed, and why the last try didn't start anything.
#[derive(Default)]
pub(super) struct Launcher {
    pub(super) query: String,
    pub(super) refused: Option<&'static str>,
}

impl State {
    /// The launcher's field and presets. Enter on an empty field starts the last length
    /// typed (the placeholder); anything refused says why in place of the detail line.
    pub(super) fn act_launcher(&mut self, ctx: &ModuleContext, action: &str) {
        let Some(launcher) = self.launcher.as_mut() else { return };
        let text = if let Some(query) = action.strip_prefix(ACTION_SEARCH) {
            launcher.query = query.to_string();
            launcher.refused = None;
            return self.publish_launcher(ctx);
        } else if action == ACTION_SUBMIT {
            let typed = launcher.query.trim().to_string();
            if typed.is_empty() { self.placeholder() } else { typed }
        } else if let Some(minutes) = action.strip_prefix("preset:").and_then(|m| m.parse::<u32>().ok()) {
            format!("{minutes}m")
        } else if action == ACTION_UNDO {
            if self.undo_reminder() {
                self.save();
                self.publish_launcher(ctx);
            }
            return;
        } else if action == ACTION_COLLAPSED {
            // Closed without starting anything.
            self.launcher = None;
            return ctx.retract(LAUNCHER);
        } else {
            return;
        };
        match self.start(&text) {
            Ok(()) => {
                if action == ACTION_SUBMIT && matches!(remind::typed(&text), remind::Typed::Other) && matches!(parse::parse(&text), Parsed::Timer { .. }) {
                    self.last_typed = Some(text);
                }
                self.launcher = None;
                ctx.retract(LAUNCHER);
                self.publish(ctx);
            }
            Err(why) => {
                if let Some(l) = self.launcher.as_mut() {
                    l.refused = Some(why);
                }
                self.publish_launcher(ctx);
            }
        }
    }

    /// Starts what `text` asks for, or says why it can't.
    pub(super) fn start(&mut self, text: &str) -> Result<(), &'static str> {
        let now = self.now();
        match remind::typed(text) {
            remind::Typed::Reminder(due) => {
                self.add_reminder(due)?;
                self.save();
                return Ok(());
            }
            remind::Typed::Refused(why) => return Err(why),
            remind::Typed::Other => {}
        }
        match parse::parse(text) {
            Parsed::Timer { ms, label } => {
                let id = self.timers.start(label, ms, now).ok_or("10 timers is the limit")?;
                println!("timers: started a {} timer", view::length_text(ms));
                self.page = Some(id);
            }
            Parsed::Focus => {
                if self.focus.is_some() {
                    return Err("A focus session is already on");
                }
                println!("timers: focus session started");
                self.focus = Some(Focus::start(plan(&self.settings), now));
            }
            // There's one stopwatch; asking again just shows it.
            Parsed::Stopwatch => _ = self.stopwatch.get_or_insert_with(|| Stopwatch::start(now)),
            Parsed::TooLong => return Err("Up to 24 hours"),
            Parsed::Empty | Parsed::Unknown => return Err("Type a length, like 10m"),
        }
        self.save();
        Ok(())
    }

    /// Opens the launcher (the hotkey, the tray's "New timer…"). Pressing the hotkey again
    /// while it's open closes it (the island does that).
    pub(super) fn open_launcher(&mut self, ctx: &ModuleContext) {
        self.launcher.get_or_insert_with(Launcher::default);
        self.publish_launcher(ctx);
        ctx.focus(LAUNCHER);
    }

    /// What an empty field offers: the last length typed, else the longest preset.
    fn placeholder(&self) -> String {
        self.last_typed.clone().unwrap_or_else(|| format!("{}m", self.settings.presets.iter().max().copied().unwrap_or(25)))
    }

    pub(super) fn publish_launcher(&self, ctx: &ModuleContext) {
        let Some(l) = &self.launcher else { return };
        // An empty field reads as what Enter would start.
        let parsed = match parse::parse(&l.query) {
            Parsed::Empty => parse::parse(&self.placeholder()),
            parsed => parsed,
        };
        let mut launcher = view::launcher(&l.query, &parsed, &self.placeholder(), &self.settings.presets, l.refused);
        self.dress_launcher(&mut launcher, &l.query, l.refused.is_some());
        ctx.publish(LAUNCHER, Priority::Passive, launcher);
    }
}
