//! The settings files and what's in them: the same field names, defaults and
//! fallbacks the island and the hub modules use when they read them (see each module's
//! `settings.rs`), so this window always shows what ReUI really does.

use serde_json::{json, Value};

use super::doc::Doc;
use crate::paths::Paths;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum File {
    /// `settings.json`, read by the island (it watches the folder).
    Island,
    Media,
    Clipboard,
    Osd,
    Devices,
    Timers,
    /// `privacy.json`: the privacy indicators (mic, camera, screen, location in use).
    Privacy,
    /// `coding.json`, read by the three coding agent modules.
    Coding,
    Stats,
}

impl File {
    pub const ALL: [File; 9] = [File::Island, File::Media, File::Clipboard, File::Osd, File::Devices, File::Timers, File::Privacy, File::Coding, File::Stats];

    pub fn name(self) -> &'static str {
        match self {
            File::Island => "settings.json",
            File::Media => "media.json",
            File::Clipboard => "clipboard.json",
            File::Osd => "osd.json",
            File::Devices => "devices.json",
            File::Timers => "timers.json",
            File::Privacy => "privacy.json",
            File::Coding => "coding.json",
            File::Stats => "stats.json",
        }
    }

    /// The hub modules told to `reload` after a write; the island notices on its own
    /// (updates reads `check_updates` from the island's file, `island` its hide shortcut).
    pub fn modules(self) -> &'static [&'static str] {
        match self {
            File::Island => &["updates", "island"],
            File::Media => &["media"],
            File::Clipboard => &["clipboard"],
            File::Osd => &["osd"],
            File::Devices => &["devices"],
            File::Timers => &["timers"],
            File::Privacy => &["privacy"],
            File::Coding => &["claude", "codex", "opencode"],
            File::Stats => &["stats"],
        }
    }

    /// The prefix of row ids bound to this file's keys ("media:lyrics").
    pub fn prefix(self) -> &'static str {
        match self {
            File::Island => "island",
            File::Media => "media",
            File::Clipboard => "clipboard",
            File::Osd => "osd",
            File::Devices => "devices",
            File::Timers => "timers",
            File::Privacy => "privacy",
            File::Coding => "coding",
            File::Stats => "stats",
        }
    }

    pub fn from_prefix(prefix: &str) -> Option<File> {
        File::ALL.into_iter().find(|f| f.prefix() == prefix)
    }

    fn defaults(self) -> Value {
        match self {
            File::Island => json!({
                "visibility": "always",
                "hide_in_fullscreen": true,
                "show_on_desktop": true,
                "view_switcher": true,
                "hide_shortcut": "Win+Alt+I",
                "idle_clock": false,
                "open_on": "click",
                "hover_delay": 250,
                "reveal_zone": "wide",
                "monitor": "primary",
                "check_updates": true,
                "whats_new": true,
                // The look, as reui_design::look::Appearance reads it.
                "style": "island",
                "theme": "dark",
                "accent": "reui",
                "roundness": 1.0,
                "top_gap": 11.0,
                "padding": "regular",
                "scale": 1.0,
            }),
            File::Media => json!({ "enabled": true, "visualizer": "real", "lyrics": false, "retract_after_pause_min": 10, "swipe_to_skip": true, "show_video": true }),
            File::Clipboard => json!({
                "enabled": true,
                "hotkey": "Win+Alt+V",
                "retention_days": 30,
                "ignored_apps": ["keepass.exe", "keepassxc.exe", "bitwarden.exe", "1password.exe", "dashlane.exe", "keeper.exe", "enpass.exe", "lastpass.exe"],
                "link_previews": true,
                "layout": "strip",
            }),
            File::Osd => json!({
                "volume": true,
                "brightness": true,
                "show_percentage": false,
                "replace_windows_flyout": false,
                "show_automatic_brightness": false,
                "lock_keys": false,
                "battery_saver": false,
                "mic_mute": false,
                "scroll_step": 0.02,
            }),
            File::Devices => json!({ "enabled": true, "connect_cards": true, "low_battery": true, "charging": true, "drives": true, "airpods": true, "phones": true, "slow_charger": false }),
            File::Timers => json!({
                "enabled": true,
                "shortcut": "Win+Ctrl+T",
                "presets": [1, 5, 10, 25],
                "sound": "Alarm01",
                "ring_for_s": 60,
                "focus_work_min": 25,
                "focus_short_break_min": 5,
                "focus_long_break_min": 15,
                "focus_rounds": 4,
                "focus_auto_start": false,
                "focus_dnd": false,
            }),
            File::Privacy => json!({ "enabled": true, "microphone": true, "camera": true, "screen": true, "location": false }),
            File::Coding => json!({ "show_sessions": "needed", "show_finished": false }),
            File::Stats => json!({ "enabled": true, "shortcut": "Win+Alt+S", "gpu": true, "disk": true, "network": true, "power": true }),
        }
    }

    /// The key of a file's shortcut, for the files that have one.
    pub fn shortcut_key(self) -> Option<&'static str> {
        match self {
            File::Clipboard => Some("hotkey"),
            File::Timers | File::Stats => Some("shortcut"),
            File::Island => Some("hide_shortcut"),
            _ => None,
        }
    }
}

pub const VISIBILITY: [&str; 3] = ["always", "when-active", "when-needed"];
/// `open_on`: what a rest on the pill does (docs/PRD-behaviour.md §4.1).
pub const OPEN_ON: [&str; 2] = ["click", "hover"];
/// `reveal_zone`: how much of the top edge reveals a hidden island (docs/PRD-behaviour.md §4.2).
pub const REVEAL_ZONE: [&str; 3] = ["off", "pill", "wide"];
/// `hover_delay` choices in ms: Quick, Normal (`motion.delay.hover-intent`), Relaxed.
pub const HOVER_DELAYS: [u64; 3] = [150, 250, 500];
pub const VISUALIZER: [&str; 3] = ["real", "simple", "off"];
/// `show_sessions` in `coding.json`.
pub const SHOW_SESSIONS: [&str; 2] = ["needed", "working"];
/// `monitor` values besides a display's device name (`\\.\DISPLAY2`).
pub const MONITOR_PRIMARY: &str = "primary";
pub const MONITOR_CURSOR: &str = "cursor";

pub struct Files {
    docs: Vec<Doc>,
}

impl Files {
    pub fn open(paths: &Paths) -> Self {
        Self { docs: File::ALL.iter().map(|&f| Doc::open(paths.file(f.name()), f.defaults())).collect() }
    }

    pub fn reload(&mut self) {
        self.docs.iter_mut().for_each(Doc::reload);
    }

    pub fn doc(&self, file: File) -> &Doc {
        &self.docs[file as usize]
    }

    pub fn doc_mut(&mut self, file: File) -> &mut Doc {
        &mut self.docs[file as usize]
    }

    /// Index into [`VISIBILITY`].
    pub fn visibility(&self) -> usize {
        let doc = self.doc(File::Island);
        doc.str("visibility").and_then(|v| VISIBILITY.iter().position(|x| *x == v)).unwrap_or(0)
    }

    /// Index into [`OPEN_ON`].
    pub fn open_on(&self) -> usize {
        let doc = self.doc(File::Island);
        doc.str("open_on").and_then(|v| OPEN_ON.iter().position(|x| *x == v)).unwrap_or(0)
    }

    /// Index into [`REVEAL_ZONE`]; Wide when missing or unknown, as the island reads it.
    pub fn reveal_zone(&self) -> usize {
        let doc = self.doc(File::Island);
        doc.str("reveal_zone").and_then(|v| REVEAL_ZONE.iter().position(|x| *x == v)).unwrap_or(2)
    }

    /// Index into [`HOVER_DELAYS`]: the nearest choice to the file's value (it takes any
    /// 50–2000 ms; out of range, the island uses Normal).
    pub fn hover_delay(&self) -> usize {
        let ms = self.doc(File::Island).get("hover_delay").and_then(Value::as_u64).filter(|ms| (50..=2000).contains(ms)).unwrap_or(250);
        (0..HOVER_DELAYS.len()).min_by_key(|&i| HOVER_DELAYS[i].abs_diff(ms)).unwrap_or(1)
    }

    /// Which monitor the island is on: "primary", "cursor" or a display's device name.
    pub fn monitor(&self) -> String {
        let doc = self.doc(File::Island);
        doc.str("monitor").map(str::trim).filter(|s| !s.is_empty()).unwrap_or(MONITOR_PRIMARY).to_string()
    }

    /// The island's look settings, read exactly as the island reads them.
    pub fn appearance(&self) -> reui_design::look::Appearance {
        let doc = self.doc(File::Island);
        reui_design::look::Appearance::read(|k| doc.str(k).map(str::to_string), |k| doc.get(k).and_then(Value::as_f64))
    }

    pub fn paused_until(&self) -> Option<u64> {
        self.doc(File::Island).get("paused_until").and_then(Value::as_u64)
    }

    /// Index into [`SHOW_SESSIONS`]; the modules are lenient about case and spaces.
    pub fn show_sessions(&self) -> usize {
        let v = self.doc(File::Coding).str("show_sessions").map(|s| s.trim().to_ascii_lowercase());
        v.and_then(|v| SHOW_SESSIONS.iter().position(|x| *x == v)).unwrap_or(0)
    }

    /// Index into [`VISUALIZER`]; the module is lenient about case and spaces.
    pub fn visualizer(&self) -> usize {
        let v = self.doc(File::Media).str("visualizer").map(|s| s.trim().to_ascii_lowercase());
        v.and_then(|v| VISUALIZER.iter().position(|x| *x == v)).unwrap_or(0)
    }

    pub fn retract_min(&self) -> u64 {
        number(self.doc(File::Media), "retract_after_pause_min")
    }

    /// A file's shortcut ("Win+Alt+V"), or "" for a file without one.
    pub fn shortcut(&self, file: File) -> String {
        let Some(key) = file.shortcut_key() else { return String::new() };
        let doc = self.doc(file);
        doc.str(key).map(str::trim).filter(|s| !s.is_empty()).or_else(|| doc.default(key).and_then(Value::as_str)).unwrap_or_default().to_string()
    }

    /// Minutes the module accepts (1 to a day), or the default.
    pub fn minutes(&self, file: File, key: &str) -> u64 {
        let doc = self.doc(file);
        doc.get(key).and_then(Value::as_u64).filter(|m| (1..=1440).contains(m)).or_else(|| doc.default(key).and_then(Value::as_u64)).unwrap_or(1)
    }

    /// The finish sound ("Alarm01"), or "none".
    pub fn timer_sound(&self) -> String {
        let doc = self.doc(File::Timers);
        let valid = |s: &&str| *s == "none" || (!s.is_empty() && s.len() <= 32 && s.chars().all(|c| c.is_ascii_alphanumeric()));
        doc.str("sound").filter(valid).unwrap_or("Alarm01").to_string()
    }

    /// Seconds the finish sound rings; 0 until stopped.
    pub fn ring_for(&self) -> u64 {
        self.doc(File::Timers).get("ring_for_s").and_then(Value::as_u64).filter(|s| *s <= 600).unwrap_or(60)
    }

    pub fn focus_rounds(&self) -> u64 {
        self.doc(File::Timers).get("focus_rounds").and_then(Value::as_u64).filter(|r| (1..=12).contains(r)).unwrap_or(4)
    }

    pub fn retention_days(&self) -> u64 {
        number(self.doc(File::Clipboard), "retention_days")
    }

    pub fn ignored_apps(&self) -> Vec<String> {
        let doc = self.doc(File::Clipboard);
        let list = doc.get("ignored_apps").and_then(Value::as_array).or_else(|| doc.default("ignored_apps").and_then(Value::as_array));
        list.map(|apps| apps.iter().filter_map(Value::as_str).map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect()).unwrap_or_default()
    }

    /// Volume change per wheel notch; the module accepts (0, 0.5].
    pub fn scroll_step(&self) -> f64 {
        let doc = self.doc(File::Osd);
        doc.get("scroll_step").and_then(Value::as_f64).filter(|s| s.is_finite() && *s > 0.0 && *s <= 0.5).unwrap_or(0.02)
    }
}

/// A whole number the module reads with `as_u64`, or the default.
fn number(doc: &Doc, key: &str) -> u64 {
    doc.get(key).and_then(Value::as_u64).or_else(|| doc.default(key).and_then(Value::as_u64)).unwrap_or(0)
}

/// Adds an executable name to a list, lower-cased like the module reads it; `None` if it
/// isn't one or is already there.
pub fn add_app(list: &[String], name: &str) -> Option<Vec<String>> {
    let name = name.trim().trim_matches('"');
    let name = name.rsplit(['\\', '/']).next().unwrap_or(name).to_lowercase();
    let name = if name.ends_with(".exe") { name } else if name.is_empty() { return None } else { format!("{name}.exe") };
    let valid = name.len() > 4 && !name.chars().any(|c| matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') || c.is_control());
    if !valid || list.contains(&name) {
        return None;
    }
    let mut list = list.to_vec();
    list.push(name);
    Some(list)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::doc::tests::temp_dir;

    fn files(dir: &std::path::Path) -> Files {
        let paths = Paths { config: dir.to_path_buf(), local: dir.join("l"), home: dir.join("h"), exe_dir: dir.to_path_buf(), sandbox: true };
        Files::open(&paths)
    }

    #[test]
    fn missing_files_read_as_the_modules_defaults() {
        let dir = temp_dir("files-defaults");
        let f = files(&dir);
        assert_eq!((f.visibility(), f.paused_until(), f.visualizer(), f.retract_min()), (0, None, 0, 10));
        assert_eq!(f.monitor(), "primary");
        assert_eq!((f.open_on(), f.hover_delay(), f.reveal_zone()), (0, 1, 2));
        let island = f.doc(File::Island);
        assert!(island.bool("hide_in_fullscreen") && island.bool("check_updates") && island.bool("whats_new"));
        assert!(f.doc(File::Media).bool("enabled") && f.doc(File::Clipboard).bool("enabled") && f.doc(File::Devices).bool("enabled"));
        assert_eq!((f.shortcut(File::Clipboard).as_str(), f.retention_days(), f.scroll_step()), ("Win+Alt+V", 30, 0.02));
        assert_eq!(f.ignored_apps().len(), 8);
        assert!(f.doc(File::Osd).bool("volume") && !f.doc(File::Osd).bool("lock_keys"));
        assert!(f.doc(File::Devices).bool("airpods") && !f.doc(File::Devices).bool("slow_charger"));
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn values_are_read_with_the_modules_fallbacks() {
        let dir = temp_dir("files-values");
        std::fs::write(dir.join("media.json"), r#"{ "visualizer": " Simple ", "retract_after_pause_min": 0 }"#).unwrap();
        std::fs::write(dir.join("clipboard.json"), r#"{ "hotkey": "  ", "ignored_apps": ["Foo.EXE", 3, ""], "retention_days": -1 }"#).unwrap();
        std::fs::write(dir.join("osd.json"), r#"{ "scroll_step": 3 }"#).unwrap();
        std::fs::write(dir.join("settings.json"), r#"{ "visibility": "when-needed", "paused_until": 99, "open_on": "hover", "hover_delay": 600 }"#).unwrap();
        let f = files(&dir);
        assert_eq!((f.visualizer(), f.retract_min()), (1, 0));
        assert_eq!((f.shortcut(File::Clipboard).as_str(), f.ignored_apps(), f.retention_days()), ("Win+Alt+V", vec!["foo.exe".to_string()], 30));
        assert_eq!(f.scroll_step(), 0.02);
        assert_eq!((f.visibility(), f.paused_until()), (2, Some(99)));
        assert_eq!((f.open_on(), f.hover_delay()), (1, 2), "600 ms reads as Relaxed");
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn apps_are_added_as_the_module_matches_them() {
        let list = vec!["keepass.exe".to_string()];
        assert_eq!(add_app(&list, " Code.EXE "), Some(vec!["keepass.exe".into(), "code.exe".into()]));
        assert_eq!(add_app(&list, "notepad"), Some(vec!["keepass.exe".into(), "notepad.exe".into()]));
        assert_eq!(add_app(&list, r#""C:\Program Files\App\App.exe""#).unwrap()[1], "app.exe");
        assert_eq!(add_app(&list, "KeePass.exe"), None);
        assert_eq!(add_app(&list, "  "), None);
        assert_eq!(add_app(&list, "a|b.exe"), None);
    }
}
