//! Everything the pages show and change: the settings files plus what the window learned
//! this session (startup entry, agents, sizes, idle cost, notes after an action).

use serde_json::Value;

use crate::model::agents::{Agent, Status};
use crate::model::files::{File, Files};
use crate::paths::Paths;
use crate::stats::Stats;
use crate::system::{self, Startup};
use crate::ui::{Target, Tone};
use crate::hub;

pub struct State {
    pub paths: Paths,
    pub files: Files,
    pub startup: Startup,
    pub agents: [Status; 3],
    /// What the last Connect / Disconnect said, per agent.
    pub agent_notes: [Option<(String, Tone)>; 3],
    /// A write that failed for a reason other than an invalid file.
    pub write_error: Option<String>,
    /// The hotkey recorder is listening.
    pub recording: bool,
    /// What the recorder last said ("taken by another app"), or that a new hotkey waits for
    /// a restart.
    pub hotkey_note: Option<(String, Tone)>,
    /// The shortcut in the file when the window opened: the running hub holds that one, so
    /// it never counts as taken.
    pub held_hotkey: String,
    /// The ignored-apps field.
    pub field: String,
    /// The running apps the open menu lists.
    pub menu_apps: Vec<String>,
    /// A destructive button waiting for its confirming press.
    pub armed: Option<Target>,
    pub stats: Stats,
    /// (roaming, local) folder sizes.
    pub sizes: (u64, u64),
    pub windows_build: String,
}

impl State {
    pub fn new(paths: Paths) -> Self {
        let files = Files::open(&paths);
        let mut state = Self {
            held_hotkey: files.hotkey(),
            files,
            startup: Startup::Off,
            agents: [Status::NotConnected; 3],
            agent_notes: Default::default(),
            write_error: None,
            recording: false,
            hotkey_note: None,
            field: String::new(),
            menu_apps: Vec::new(),
            armed: None,
            stats: Stats::default(),
            sizes: (0, 0),
            windows_build: system::windows_build(),
            paths,
        };
        state.refresh();
        state
    }

    /// Re-reads everything that may have changed outside the window (on activation).
    pub fn refresh(&mut self) {
        self.files.reload();
        self.startup = system::startup(&self.paths);
        self.agents = Agent::ALL.map(|a| a.status(&self.paths));
        self.stats.sample();
    }

    pub fn refresh_sizes(&mut self) {
        self.sizes = (system::folder_size(&self.paths.config), system::folder_size(&self.paths.local));
    }

    /// Writes one key, then tells whoever reads the file. Failures show on the page: an
    /// invalid file as its error card, anything else at the top.
    pub fn set(&mut self, file: File, key: &str, value: Value) {
        match self.files.doc_mut(file).set(key, value) {
            Ok(()) => {
                self.write_error = None;
                if let Some(module) = file.module() {
                    self.tell_hub(module, "reload");
                }
            }
            Err(e) => {
                if self.files.doc(file).error().is_none() {
                    self.write_error = Some(e);
                }
            }
        }
    }

    /// A module command for the running hub (never in sandbox mode: tests mustn't reach a
    /// real ReUI).
    pub fn tell_hub(&self, module: &str, action: &str) {
        if !self.paths.sandbox {
            hub::send(vec![(module.to_string(), action.to_string())]);
        }
    }

    pub fn is_armed(&self, row: &str, part: usize) -> bool {
        self.armed.as_ref().is_some_and(|t| t.row == row && t.part == part)
    }

    pub fn hub_running(&self) -> bool {
        self.stats.usage[0].running
    }
}
