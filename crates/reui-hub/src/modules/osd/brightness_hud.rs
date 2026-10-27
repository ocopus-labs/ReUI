//! The brightness HUD's state, and the OS window's events: brightness keys and panel
//! changes, worker reports per display, the sliders, lock keys and battery saver.

use reui_protocol::Priority;
use tokio::time::Instant;

use super::brightness::{Display, Report, INTERNAL};
use crate::modules::system_events::{LockKey, SystemEvent as OsEvent};
use super::view::{self, DisplayLevel, LINGER};
use super::{Osd, BATTERY_SAVER, BRIGHTNESS, LOCK};
use crate::hub::ModuleContext;

impl Osd {
    pub(super) fn on_os(&mut self, ctx: &ModuleContext, event: OsEvent) {
        let now = Instant::now();
        match event {
            OsEvent::Brightness { percent, initial } => {
                let level = percent as f32 / 100.0;
                let index = match self.displays.iter().position(|d| d.id == INTERNAL) {
                    Some(i) => i,
                    // A real change means there is a panel even if WMI didn't say so (yet).
                    None if !initial => {
                        self.displays.insert(0, Display { id: INTERNAL.into(), name: view::display_label(None, true, 1), level });
                        0
                    }
                    None => return,
                };
                let ours = self.internal_write.is_some_and(|at| now < at + view::BRIGHTNESS_ECHO);
                if !ours {
                    self.displays[index].level = level;
                }
                if initial || !self.settings.brightness {
                    return;
                }
                if view::brightness_shows(now, self.internal_write, self.last_key, self.keys_seen, self.settings.show_automatic_brightness) {
                    self.front = index;
                    self.show_brightness(ctx, now);
                } else if !ours {
                    self.refresh_brightness(ctx);
                }
            }
            OsEvent::BrightnessKey => {
                self.last_key = Some(now);
                self.keys_seen = true;
                // Without a built-in panel the keys do nothing by themselves; show the
                // external monitors ReUI can set.
                let internal = self.displays.iter().any(|d| d.id == INTERNAL);
                if !internal && !self.displays.is_empty() && self.settings.brightness {
                    self.front = 0;
                    self.show_brightness(ctx, now);
                }
            }
            OsEvent::BatterySaver { on, initial } => {
                if !initial && self.settings.battery_saver {
                    self.show_state(ctx, BATTERY_SAVER, view::GLYPH_BATTERY_SAVER, if on { "Battery saver on" } else { "Battery saver off" });
                }
            }
            OsEvent::Lock { key, on } => {
                if self.settings.lock_keys {
                    let (glyph, name) = match key {
                        LockKey::Caps => (view::GLYPH_CAPS_LOCK, "Caps Lock"),
                        LockKey::Num => (view::GLYPH_NUM_LOCK, "Num Lock"),
                    };
                    self.show_state(ctx, LOCK, glyph, &format!("{name} {}", if on { "on" } else { "off" }));
                }
            }
            OsEvent::TaskbarCreated => self.ensure_flyout(),
            OsEvent::DisplaysChanged => {
                if let Some(w) = &self.worker {
                    w.probe();
                }
            }
            // The Devices module's events.
            OsEvent::PowerSource { .. } | OsEvent::BatteryPercent { .. } | OsEvent::Resumed | OsEvent::Unlocked | OsEvent::Volume | OsEvent::PortableDevice { .. } => {}
        }
    }

    pub(super) fn on_report(&mut self, ctx: &ModuleContext, report: Report) {
        match report {
            Report::Displays(displays) => {
                let front_id = self.displays.get(self.front).map(|d| d.id.clone());
                self.displays = displays;
                self.front = front_id.and_then(|id| self.displays.iter().position(|d| d.id == id)).unwrap_or(0);
                if self.displays.is_empty() && self.brightness.visible {
                    self.brightness.hide();
                    ctx.retract(BRIGHTNESS);
                } else {
                    self.refresh_brightness(ctx);
                }
            }
            Report::Failed { id } if id == INTERNAL => {
                // Re-read, so the bar shows what the panel really has.
                self.internal_write = None;
                if let Some(w) = &self.worker {
                    w.probe();
                }
            }
            Report::Failed { id } => {
                self.displays.retain(|d| d.id != id);
                self.front = self.front.min(self.displays.len().saturating_sub(1));
                if self.displays.is_empty() && self.brightness.visible {
                    self.brightness.hide();
                    ctx.retract(BRIGHTNESS);
                } else {
                    self.refresh_brightness(ctx);
                }
            }
        }
    }

    fn show_brightness(&mut self, ctx: &ModuleContext, now: Instant) {
        if !self.brightness.visible {
            self.ensure_flyout();
        }
        if self.brightness.changed(now, LINGER) {
            self.publish_brightness(ctx);
        }
    }

    pub(super) fn refresh_brightness(&mut self, ctx: &ModuleContext) {
        if self.brightness.refresh(Instant::now()) {
            self.publish_brightness(ctx);
        }
    }

    pub(super) fn publish_brightness(&self, ctx: &ModuleContext) {
        if !self.brightness.visible {
            return;
        }
        let levels: Vec<DisplayLevel> = self.displays.iter().map(|d| DisplayLevel { id: d.id.clone(), name: d.name.clone(), level: d.level }).collect();
        if let Some(view) = view::brightness_view(&levels, self.front, self.settings.show_percentage) {
            ctx.publish(BRIGHTNESS, Priority::Brief, view);
        }
    }

    pub(super) fn set_brightness(&mut self, ctx: &ModuleContext, id: &str, level: f32) {
        let Some(index) = self.displays.iter().position(|d| d.id == id) else { return };
        let now = Instant::now();
        self.displays[index].level = level;
        self.front = index;
        if id == INTERNAL {
            self.internal_write = Some(now);
        }
        self.brightness.interacted(now);
        self.publish_brightness(ctx);
        if let Some(w) = &self.worker {
            w.set(id, level);
        }
    }

}
