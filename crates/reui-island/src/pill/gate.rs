//! What gets through to the pill: the hub's snapshot, less what the user has quieted. A
//! pause or the hide shortcut lets only decisions (and a card the user summons) through; an
//! app rule for the app in front hides the island or lets only decisions through
//! (`app_rules.rs`); a snoozed module's moments wait (`quiet.rs`). Then `order::arrange`
//! decides what's in front.

use reui_protocol::{ActivityId, Priority, Quiet, Snapshot};
use windows::Win32::Foundation::HWND;

use crate::app_rules::{self, AppRule};
use crate::desktop;
use crate::order;
use crate::quiet;
use crate::settings::Visibility;

use super::{Pill, State};

/// The user's quieting rules and the app they apply to now.
#[derive(Default)]
pub(super) struct Gate {
    /// `app_rules` from settings.json.
    pub rules: Vec<(String, AppRule)>,
    /// The app in front (shell surfaces keep the last one; see `app_rules::front_app`).
    pub app: Option<String>,
    /// Whether `app` has been read yet (the island may start with any app in front).
    pub seen: bool,
    /// `snoozed` from settings.json: module, until (Unix ms).
    pub snoozed: Vec<(String, u64)>,
    /// The moment a right-click menu is about, and what it declared, until an entry is picked.
    pub quieting: Option<(ActivityId, Quiet)>,
    /// A moment just turned off from itself: held back until the module retracts it.
    pub dismissed: Option<ActivityId>,
}

impl Gate {
    /// The rule for the app in front now.
    pub fn rule(&self) -> Option<AppRule> {
        app_rules::rule_for(&self.rules, self.app.as_deref())
    }
}

impl Pill {
    /// What to show where (see `order::arrange`).
    pub(super) fn ordered(&mut self) -> Snapshot {
        let open = if self.state == State::Expanded { self.open_id.clone() } else { None };
        // "When needed" hides ongoing things, so a confirmation there leads instead of
        // waiting in the bubble beside something the user can't see.
        let briefs_lead = self.visibility == Visibility::WhenNeeded && !self.desktop_shows() && !self.raw.activities.iter().any(|a| a.priority >= Priority::Alert);
        let mut raw = self.raw.activities.clone();
        if self.gate.dismissed.as_ref().is_some_and(|id| !raw.iter().any(|a| a.id == *id)) {
            self.gate.dismissed = None;
        }
        let now = reui_protocol::unix_ms();
        raw.retain(|a| !quiet::held(&self.gate.snoozed, a, now) && Some(&a.id) != self.gate.dismissed.as_ref());
        if self.paused || self.hidden_by_user || self.only_decisions() {
            // Paused, hidden or an app that wants quiet: only decisions show (see
            // `wants_shown`), alone in the pill; hidden, so does a card the user summons (a
            // hotkey asks for exactly that).
            let asked = self.pinned.clone().filter(|_| self.hidden_by_user && !self.paused && self.summoned);
            raw.retain(|a| a.priority == Priority::Decision || Some(&a.id) == asked.as_ref());
        }
        let (mut activities, pin_alive) = order::arrange(raw, self.pinned.as_ref(), self.summoned, open.as_ref(), briefs_lead);
        if !pin_alive {
            self.pinned = None;
            self.summoned = false;
        }
        // A drag over the island: the drop target leads, whatever else is going on, and
        // stands for the shelf (its own pill would only repeat it in the bubble).
        if let Some(face) = self.drop_face() {
            activities.retain(|a| a.id.module != face.id.module);
            activities.insert(0, face);
        }
        Snapshot { activities }
    }

    /// The app in front asks for decisions only.
    pub(super) fn only_decisions(&self) -> bool {
        self.gate.rule() == Some(AppRule::OnlyDecisions)
    }

    /// The app in front asks for no island at all.
    pub(super) fn app_hides(&self) -> bool {
        self.gate.rule() == Some(AppRule::Hide)
    }

    /// `app_rules` from settings.json. The first time, the app already in front is read, so
    /// a rule applies from the start rather than from the next app switch.
    pub(super) fn set_app_rules(&mut self, rules: Vec<(String, AppRule)>) {
        if rules == self.gate.rules && self.gate.seen {
            return;
        }
        let before = self.gate.rule();
        self.gate.rules = rules;
        if !self.gate.seen {
            self.gate.seen = true;
            let fg = unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
            self.gate.app = app_rules::front_app(desktop::front(fg), app_rules::exe_of(fg), None);
        }
        if self.gate.rule() != before {
            self.gate_changed();
        }
    }

    /// The foreground changed to `fg`: the rule of the app now in front applies.
    pub(super) fn note_front_app(&mut self, fg: HWND) {
        let front = desktop::front(fg);
        let exe = if front == desktop::Front::App { app_rules::exe_of(fg) } else { None };
        self.set_front_app(front, exe);
    }

    fn set_front_app(&mut self, front: desktop::Front, exe: Option<String>) {
        let before = self.gate.rule();
        self.gate.seen = true;
        self.gate.app = app_rules::front_app(front, exe, self.gate.app.take());
        if self.gate.rule() != before {
            eprintln!("island: app rule {:?} for {:?}", self.gate.rule(), self.gate.app);
            self.gate_changed();
        }
    }

    /// Debug builds: a test script says process `pid` came to the front (see `main.rs`), so
    /// app rules can be checked without switching the user's windows.
    #[cfg(debug_assertions)]
    pub fn test_front_app(&mut self, pid: u32) {
        self.set_front_app(desktop::Front::App, app_rules::exe_of_pid(pid));
    }
}
