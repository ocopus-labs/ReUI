//! Devices settings, in `%APPDATA%\ReUI\devices.json` until a settings window exists. Same
//! rules as the OSD's: created with the defaults when missing, a missing or mistyped field
//! falls back to its default, keys this version doesn't know are kept, and a file that isn't
//! JSON is never overwritten.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// A card when a Bluetooth device connects.
    pub connect_cards: bool,
    /// The 20 % / 10 % moments (peripherals and the laptop).
    pub low_battery: bool,
    /// The plug-in moment.
    pub charging: bool,
    pub drives: bool,
    /// AirPods' case and buds from their advertisements (a short BLE scan, D4).
    pub airpods: bool,
    pub phones: bool,
    /// "Charging slowly" (Windows has its own toast, D7).
    pub slow_charger: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { connect_cards: true, low_battery: true, charging: true, drives: true, airpods: true, phones: true, slow_charger: false }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("ReUI")).unwrap_or_else(|| PathBuf::from(".")).join("devices.json")
    }

    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).unwrap_or_else(|| {
                eprintln!("devices: {} isn't valid JSON; using the defaults", path.display());
                Self::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let defaults = Self::default();
                write(path, &defaults.to_json());
                defaults
            }
            Err(e) => {
                eprintln!("devices: can't read {}: {e}", path.display());
                Self::default()
            }
        }
    }

    /// `None` if the text isn't a JSON object.
    pub fn parse(text: &str) -> Option<Self> {
        let Value::Object(map) = serde_json::from_str(text).ok()? else { return None };
        let d = Self::default();
        let flag = |key: &str, default: bool| map.get(key).and_then(Value::as_bool).unwrap_or(default);
        Some(Self {
            connect_cards: flag("connect_cards", d.connect_cards),
            low_battery: flag("low_battery", d.low_battery),
            charging: flag("charging", d.charging),
            drives: flag("drives", d.drives),
            airpods: flag("airpods", d.airpods),
            phones: flag("phones", d.phones),
            slow_charger: flag("slow_charger", d.slow_charger),
        })
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(&self.to_map()).unwrap_or_default()
    }

    fn to_map(&self) -> Map<String, Value> {
        let Value::Object(map) = json!({
            "connect_cards": self.connect_cards,
            "low_battery": self.low_battery,
            "charging": self.charging,
            "drives": self.drives,
            "airpods": self.airpods,
            "phones": self.phones,
            "slow_charger": self.slow_charger,
        }) else {
            unreachable!()
        };
        map
    }

    /// Applies a tray command (`connect:on`, `low-battery:off`, `drives:on`, `airpods:off`,
    /// and `charging:`, `phones:`, `slow-charger:`). `false` if it isn't one.
    pub fn apply(&mut self, command: &str) -> bool {
        let Some((name, value)) = command.split_once(':') else { return false };
        let on = match value {
            "on" => true,
            "off" => false,
            _ => return false,
        };
        let field = match name {
            "connect" => &mut self.connect_cards,
            "low-battery" => &mut self.low_battery,
            "charging" => &mut self.charging,
            "drives" => &mut self.drives,
            "airpods" => &mut self.airpods,
            "phones" => &mut self.phones,
            "slow-charger" => &mut self.slow_charger,
            _ => return false,
        };
        *field = on;
        true
    }

    /// Saves over `path`, keeping keys this version doesn't know. A file that isn't JSON is
    /// the user's to fix, so it's left as it is (the change then lasts until the hub restarts).
    pub fn save_to(&self, path: &Path) {
        let mut map = match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(Value::Object(map)) => map,
                _ => {
                    eprintln!("devices: {} isn't valid JSON; not saving", path.display());
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
        eprintln!("devices: can't write {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("reui-devices-settings-{name}-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&dir);
        dir.join("devices.json")
    }

    #[test]
    fn devices_settings_defaults_round_trip() {
        let d = Settings::default();
        assert!(d.connect_cards && d.low_battery && d.charging && d.drives && d.airpods && d.phones && !d.slow_charger);
        assert_eq!(Settings::parse(&d.to_json()), Some(d));
    }

    #[test]
    fn devices_settings_partial_and_wrong_fields_fall_back() {
        let s = Settings::parse(r#"{ "drives": false, "airpods": "no", "slow_charger": true }"#).unwrap();
        assert!(!s.drives && s.airpods && s.slow_charger && s.connect_cards);
        assert_eq!(Settings::parse("[1]"), None);
        assert_eq!(Settings::parse("{}"), Some(Settings::default()));
    }

    #[test]
    fn devices_settings_tray_commands() {
        let mut s = Settings::default();
        assert!(s.apply("connect:off") && !s.connect_cards);
        assert!(s.apply("low-battery:off") && !s.low_battery);
        assert!(s.apply("drives:off") && !s.drives);
        assert!(s.apply("airpods:off") && !s.airpods);
        assert!(s.apply("slow-charger:on") && s.slow_charger);
        assert!(!s.apply("connect:maybe") && !s.apply("overview") && !s.apply("volume:on"));
    }

    #[test]
    fn devices_settings_files_are_created_saved_and_never_clobbered() {
        let path = temp("files");
        assert_eq!(Settings::load_from(&path), Settings::default());
        assert_eq!(Settings::parse(&std::fs::read_to_string(&path).unwrap()), Some(Settings::default()));

        std::fs::write(&path, r#"{ "future": 1, "drives": true }"#).unwrap();
        let s = Settings { drives: false, ..Settings::default() };
        s.save_to(&path);
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!((saved["future"].as_u64(), saved["drives"].as_bool()), (Some(1), Some(false)));

        std::fs::write(&path, "{ broken").unwrap();
        assert_eq!(Settings::load_from(&path), Settings::default());
        s.save_to(&path);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ broken");
        _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
