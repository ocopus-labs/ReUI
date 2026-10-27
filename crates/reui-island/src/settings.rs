//! User settings for the island, stored as JSON in `%APPDATA%\ReUI\settings.json`.
//! Changed from the right-click / tray menu; unknown or missing fields fall back to defaults.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// When the island shows at all.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Visibility {
    /// The resting pill is always there.
    #[default]
    Always,
    /// Hidden while nothing is happening.
    WhenActive,
    /// Hidden unless something needs you: a decision, a question, a finished or failed task.
    WhenNeeded,
}

impl Visibility {
    pub const ALL: [(Visibility, &'static str); 3] = [
        (Visibility::Always, "Always"),
        (Visibility::WhenActive, "When something is happening"),
        (Visibility::WhenNeeded, "Only when it needs me"),
    ];
}

#[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub visibility: Visibility,
}

impl Settings {
    fn path() -> Option<PathBuf> {
        std::env::var_os("APPDATA").map(|dir| PathBuf::from(dir).join("ReUI").join("settings.json"))
    }

    pub fn load() -> Self {
        Self::path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(path) = Self::path() else { return };
        let Ok(text) = serde_json::to_string_pretty(self) else { return };
        if let Some(dir) = path.parent() {
            _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&path, text + "\n") {
            eprintln!("island: can't save settings to {}: {e}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_what_it_writes_and_tolerates_junk() {
        let text = serde_json::to_string(&Settings { visibility: Visibility::WhenNeeded }).unwrap();
        assert_eq!(text, r#"{"visibility":"when-needed"}"#);
        assert_eq!(serde_json::from_str::<Settings>(&text).unwrap().visibility, Visibility::WhenNeeded);
        // Missing or extra fields fall back to defaults instead of failing.
        assert_eq!(serde_json::from_str::<Settings>(r#"{"other":1}"#).unwrap(), Settings::default());
    }
}
