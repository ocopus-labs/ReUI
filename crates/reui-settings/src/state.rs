//! Everything the pages show and change: the settings files plus what the window learned
//! this session (startup entry, agents, sizes, idle cost, notes after an action).

use serde_json::Value;

use crate::model::agents::{Agent, Status};
use crate::model::files::{File, Files};
use crate::paths::Paths;
use crate::stats::Stats;
use crate::system::{self, Startup};
use crate::ui::{Target, Tone};
use crate::updates::{self, Updates};
use crate::hub;

pub struct State {
    pub paths: Paths,
    pub files: Files,
    pub startup: Startup,
    pub agents: [Status; 3],
    /// Whether each agent is on this PC.
    pub agent_installed: [bool; 3],
    /// When ReUI last heard from each (unix ms).
    pub agent_last: [Option<u64>; 3],
    /// What the last Connect / Disconnect said, per agent.
    pub agent_notes: [Option<(String, Tone)>; 3],
    /// A write that failed for a reason other than an invalid file.
    pub write_error: Option<String>,
    /// The hotkey recorder is listening, for this file's shortcut.
    pub recording: Option<File>,
    /// What the recorder last said about a file's shortcut ("taken by another app"), or
    /// that a new hotkey waits for a restart.
    pub hotkey_note: Option<(File, String, Tone)>,
    /// The shortcuts in the files when the window opened: the running hub holds those, so
    /// they never count as taken.
    pub held_hotkeys: Vec<(File, String)>,
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
    /// A feature was switched on or off while the hub runs: it applies on its next start.
    pub restart_note: bool,
    pub updates: Updates,
    /// This version's CHANGELOG entry, when this run follows an update (kept for the
    /// session once About has shown it).
    pub whats_new: Option<String>,
    /// Displays for the island's monitor choice: (device name, label).
    pub monitors: Vec<(String, String)>,
}

impl State {
    pub fn new(paths: Paths) -> Self {
        let files = Files::open(&paths);
        let mut state = Self {
            held_hotkeys: [File::Clipboard, File::Timers, File::Stats].map(|f| (f, files.shortcut(f))).to_vec(),
            files,
            startup: Startup::Off,
            agents: [Status::NotConnected; 3],
            agent_installed: [false; 3],
            agent_last: [None; 3],
            agent_notes: Default::default(),
            write_error: None,
            recording: None,
            hotkey_note: None,
            field: String::new(),
            menu_apps: Vec::new(),
            armed: None,
            stats: Stats::default(),
            sizes: (0, 0),
            windows_build: system::windows_build(),
            restart_note: false,
            updates: Updates::new(&paths),
            whats_new: updates::whats_new(&updates::read_state(&paths)),
            monitors: Vec::new(),
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
        self.agent_installed = Agent::ALL.map(|a| a.installed(&self.paths));
        self.agent_last = Agent::ALL.map(|a| a.last_event(&self.paths));
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
                for module in file.modules() {
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
        // In the sandbox no ReUI is ours.
        !self.paths.sandbox && self.stats.usage[0].running
    }

    /// About showed What's new: not again after this session.
    pub fn whats_new_seen(&mut self) {
        if self.whats_new.is_some() {
            _ = updates::write_state(&self.paths, vec![("whats_new_seen", serde_json::Value::from(crate::pages::VERSION))]);
        }
    }
}
