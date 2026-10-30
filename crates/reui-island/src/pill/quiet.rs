//! Quieting a kind of moment from the moment itself (`crate::quiet`): which moment a
//! right-click is on and its entries in the island's menu, what picking one does, and
//! keeping snoozed modules out of the pill until their hour is up.

use reui_protocol::{ActivityId, Quiet};
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

use crate::quiet;
use crate::settings::{self, Settings};

use super::timers::TIMER_SNOOZE;
use super::{MenuEntry, Pill};

impl Pill {
    /// The quiet entries for a right-click at client point (`x`, `y`): for the moment in the
    /// bubble when it's on the bubble, else the one in the pill. Remembered for the pick.
    pub fn moment_menu(&mut self, x: f32, y: f32) -> Vec<MenuEntry> {
        let (sx, sy) = (x + self.window.left as f32, y + self.window.top as f32);
        let on_bubble = self.bubble_rect().is_some_and(|(l, t, r, b)| sx >= l && sx < r && sy >= t && sy < b);
        let activity = self.snapshot.activities.get(usize::from(on_bubble));
        let entries = activity.map(quiet::entries).unwrap_or_default();
        self.gate.quieting = activity.filter(|_| !entries.is_empty()).and_then(|a| Some((a.id.clone(), a.quiet.clone()?)));
        entries.into_iter().map(|(label, action)| (label.to_string(), action.to_string(), false, false)).collect()
    }

    /// An entry from [`Self::moment_menu`] was picked.
    pub fn quiet_pick(&mut self, action: &str) {
        let Some((id, declared)) = self.gate.quieting.take() else { return };
        match action {
            quiet::ACTION_OFF => self.turn_off(id, declared),
            quiet::ACTION_SNOOZE => {
                eprintln!("island: {} snoozed for an hour", id.module);
                let mut settings = Settings::load();
                let now = settings::now_ms();
                settings.snoozed = quiet::snooze(&settings.snoozed, &id.module, now + quiet::SNOOZE_MS, now);
                settings.save();
                self.apply_settings(settings);
            }
            _ => {}
        }
    }

    /// "Don't show these": the module's own setting goes off, written as Settings writes it,
    /// and the module is told to `reload`. The moment on screen goes now rather than when
    /// it would have.
    fn turn_off(&mut self, id: ActivityId, declared: Quiet) {
        let (Some(setting), Some(dir)) = (declared.setting, Settings::dir()) else { return };
        match quiet::turn_off(&dir, &setting) {
            Ok(()) => {
                eprintln!("island: {} {} off", setting.file, setting.key);
                self.command(&id.module, "reload");
                self.gate.dismissed = Some(id);
                self.gate_changed();
            }
            Err(e) => eprintln!("island: can't turn off {} in {}: {e}", setting.key, setting.file),
        }
    }

    /// Debug builds: what picking entry `n` of the menu at client point (`x`, `y`) does, for
    /// a test script (`main.rs`), so no menu shows at the user's pointer.
    #[cfg(debug_assertions)]
    pub fn test_quiet(&mut self, x: f32, y: f32, n: usize) {
        let entries = self.moment_menu(x, y);
        eprintln!("island: test quiet menu {:?}", entries.iter().map(|e| e.0.as_str()).collect::<Vec<_>>());
        if let Some((_, action, ..)) = entries.get(n) {
            self.quiet_pick(&action.clone());
        }
    }

    /// settings.json's snoozes (at start, and whenever the file changes: a snooze from the
    /// menu, an Undo in Settings).
    pub(super) fn set_snoozed(&mut self, list: Vec<(String, u64)>) {
        if list != self.gate.snoozed {
            self.gate.snoozed = list;
            self.gate_changed();
        }
    }

    /// What gets through changed: show it, hide or show for it, and wake when the next
    /// snooze ends (no timer at all while nothing is snoozed).
    pub(super) fn gate_changed(&mut self) {
        let ordered = self.ordered();
        if let Err(e) = self.render(ordered) {
            eprintln!("island: render failed: {e}");
        }
        self.refresh_visibility();
        unsafe {
            match quiet::next_end(&self.gate.snoozed, settings::now_ms()) {
                Some(ms) => _ = SetTimer(Some(self.hwnd), TIMER_SNOOZE, (ms.min(u64::from(u32::MAX - 50)) as u32).saturating_add(50), None),
                None => _ = KillTimer(Some(self.hwnd), TIMER_SNOOZE),
            }
        }
    }
}
