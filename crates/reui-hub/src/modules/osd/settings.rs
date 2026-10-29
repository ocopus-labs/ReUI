//! System OSD settings, in `%APPDATA%\ReUI\osd.json`, written by the Settings window
//! too (which then sends the module `reload`). Same
//! rules as the media settings: created with the defaults when missing, read again whenever
//! a HUD opens, a missing or mistyped field falls back to its default, keys this version
//! doesn't know are kept, and a file that isn't JSON is never overwritten.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// The volume HUD.
    pub volume: bool,
    /// The brightness HUD.
    pub brightness: bool,
    /// The level as a number in the open HUD.
    pub show_percentage: bool,
    /// Hide Windows' own volume / brightness flyout while ReUI runs (all or nothing).
    pub replace_windows_flyout: bool,
    /// Also show brightness changes nobody asked for (adaptive brightness, the charger).
    pub show_automatic_brightness: bool,
    /// Caps Lock / Num Lock moments.
    pub lock_keys: bool,
    pub battery_saver: bool,
    /// The default microphone's mute.
    pub mic_mute: bool,
    /// Every app's own level in the open volume card (the mixer). Its button in the card
    /// flips this too, so the card opens the way it was left.
    pub mixer: bool,
    /// Level change per wheel notch, 0..=1.
    pub scroll_step: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            volume: true,
            brightness: true,
            show_percentage: false,
            replace_windows_flyout: false,
            show_automatic_brightness: false,
            lock_keys: false,
            battery_saver: false,
            mic_mute: false,
            mixer: false,
            scroll_step: 0.02,
        }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        crate::paths::config().join("osd.json")
    }

    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).unwrap_or_else(|| {
                eprintln!("osd: {} isn't valid JSON; using the defaults", path.display());
                Self::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let defaults = Self::default();
                write(path, &defaults.to_json());
                defaults
            }
            Err(e) => {
                eprintln!("osd: can't read {}: {e}", path.display());
                Self::default()
            }
        }
    }

    /// `None` if the text isn't a JSON object.
    pub fn parse(text: &str) -> Option<Self> {
        let Value::Object(map) = serde_json::from_str(text).ok()? else { return None };
        let d = Self::default();
        let flag = |key: &str, default: bool| map.get(key).and_then(Value::as_bool).unwrap_or(default);
        let step = map.get("scroll_step").and_then(Value::as_f64).filter(|s| s.is_finite() && *s > 0.0 && *s <= 0.5).map_or(d.scroll_step, |s| s as f32);
        Some(Self {
            volume: flag("volume", d.volume),
            brightness: flag("brightness", d.brightness),
            show_percentage: flag("show_percentage", d.show_percentage),
            replace_windows_flyout: flag("replace_windows_flyout", d.replace_windows_flyout),
            show_automatic_brightness: flag("show_automatic_brightness", d.show_automatic_brightness),
            lock_keys: flag("lock_keys", d.lock_keys),
            battery_saver: flag("battery_saver", d.battery_saver),
            mic_mute: flag("mic_mute", d.mic_mute),
            mixer: flag("mixer", d.mixer),
            scroll_step: step,
        })
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(&self.to_map()).unwrap_or_default()
    }

    fn to_map(&self) -> Map<String, Value> {
        let Value::Object(map) = json!({
            "volume": self.volume,
            "brightness": self.brightness,
            "show_percentage": self.show_percentage,
            "replace_windows_flyout": self.replace_windows_flyout,
            "show_automatic_brightness": self.show_automatic_brightness,
            "lock_keys": self.lock_keys,
            "battery_saver": self.battery_saver,
            "mic_mute": self.mic_mute,
            "mixer": self.mixer,
            // Rounded so 0.02 doesn't come back as 0.019999999552965164.
            "scroll_step": (f64::from(self.scroll_step) * 1000.0).round() / 1000.0,
        }) else {
            unreachable!()
        };
        map
    }

    /// Applies a tray command (`flyout:on`, `percentage:off`, `lock-keys:on`,
    /// `battery-saver:off`). `false` if it isn't one.
    pub fn apply(&mut self, command: &str) -> bool {
        let Some((name, value)) = command.split_once(':') else { return false };
        let on = match value {
            "on" => true,
            "off" => false,
            _ => return false,
        };
        match name {
            "flyout" => self.replace_windows_flyout = on,
            "percentage" => self.show_percentage = on,
            "lock-keys" => self.lock_keys = on,
            "battery-saver" => self.battery_saver = on,
            "mixer" => self.mixer = on,
            _ => return false,
        }
        true
    }

    /// Saves over `path`, keeping keys this version doesn't know. A file that isn't JSON is
    /// the user's to fix, so it's left as it is (the change then lasts until the hub restarts).
    pub fn save_to(&self, path: &Path) {
        let mut map = match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(Value::Object(map)) => map,
                _ => {
                    eprintln!("osd: {} isn't valid JSON; not saving", path.display());
                    return;
                }
            },
            Err(_) => Map::new(),
        };
        map.extend(self.to_map());
        write(path, &serde_json::to_string_pretty(&Value::Object(map)).unwrap_or_default());
    }
}

fn write(path: &Path, text: &str) {
    if let Some(dir) = path.parent() {
        _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(path, text) {
        eprintln!("osd: can't write {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("reui-osd-settings-{name}-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&dir);
        dir.join("osd.json")
    }

    #[test]
    fn osd_defaults_round_trip() {
        let d = Settings::default();
        assert!(d.volume && d.brightness);
        assert!(!d.show_percentage && !d.replace_windows_flyout && !d.show_automatic_brightness && !d.lock_keys && !d.battery_saver && !d.mic_mute && !d.mixer);

        assert_eq!(d.scroll_step, 0.02);
        assert_eq!(Settings::parse(&d.to_json()), Some(d));
    }

    #[test]
    fn osd_partial_and_wrong_fields_fall_back() {
        let s = Settings::parse(r#"{ "volume": false, "lock_keys": "yes", "scroll_step": 0.05 }"#).unwrap();
        assert!(!s.volume && s.brightness && !s.lock_keys);
        assert_eq!(s.scroll_step, 0.05);
        assert_eq!(Settings::parse(r#"{ "scroll_step": -1 }"#).unwrap().scroll_step, 0.02);
        assert_eq!(Settings::parse(r#"{ "scroll_step": 3 }"#).unwrap().scroll_step, 0.02);
        assert_eq!(Settings::parse("[1]"), None);
        assert_eq!(Settings::parse("{}"), Some(Settings::default()));
    }

    #[test]
    fn osd_tray_commands() {
        let mut s = Settings::default();
        assert!(s.apply("flyout:on") && s.replace_windows_flyout);
        assert!(s.apply("flyout:off") && !s.replace_windows_flyout);
        assert!(s.apply("percentage:on") && s.show_percentage);
        assert!(s.apply("lock-keys:on") && s.lock_keys);
        assert!(s.apply("battery-saver:on") && s.battery_saver);
        assert!(s.apply("mixer:on") && s.mixer);
        assert!(!s.apply("flyout:maybe") && !s.apply("volume:0.4") && !s.apply("mute"));
    }

    #[test]
    fn osd_files_are_created_saved_and_never_clobbered() {
        let path = temp("files");
        assert_eq!(Settings::load_from(&path), Settings::default());
        assert_eq!(Settings::parse(&std::fs::read_to_string(&path).unwrap()), Some(Settings::default()));

        std::fs::write(&path, r#"{ "future": 1, "lock_keys": false }"#).unwrap();
        let s = Settings { lock_keys: true, ..Settings::default() };
        s.save_to(&path);
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!((saved["future"].as_u64(), saved["lock_keys"].as_bool(), saved["scroll_step"].as_f64()), (Some(1), Some(true), Some(0.02)));

        std::fs::write(&path, "{ broken").unwrap();
        assert_eq!(Settings::load_from(&path), Settings::default());
        s.save_to(&path);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ broken");
        _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
