//! Per-app rules (docs/PRD-behaviour.md §4.3): while a chosen app is in front, hide the
//! island or let only decisions through. The user lists executables in settings.json
//! (`app_rules`: `{ "game.exe": "hide", "obs64.exe": "decisions" }`); the island reads the
//! front process's name once per foreground change (the hook it already has), never by
//! polling. Which rule applies is pure; [`exe_of`] is the one OS call.

use serde_json::Value;
use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

use crate::desktop::Front;

/// What an app in front does to the island.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AppRule {
    /// Hidden, as for fullscreen: nothing shows.
    Hide,
    /// Only decisions (permission requests, questions) show, alone in the pill.
    OnlyDecisions,
}

impl AppRule {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "hide" => Some(AppRule::Hide),
            "decisions" => Some(AppRule::OnlyDecisions),
            _ => None,
        }
    }
}

/// The rules from settings.json's `app_rules` object, executable names lower-cased as
/// [`exe_of`] reports them; entries with an unknown rule are skipped (kept in the file).
pub fn parse(value: Option<&Value>) -> Vec<(String, AppRule)> {
    let Some(Value::Object(map)) = value else { return Vec::new() };
    map.iter().filter_map(|(exe, rule)| Some((exe.trim().to_lowercase(), AppRule::parse(rule.as_str()?)?))).filter(|(exe, _)| !exe.is_empty()).collect()
}

/// The rule for the app in front, if it has one.
pub fn rule_for(rules: &[(String, AppRule)], exe: Option<&str>) -> Option<AppRule> {
    let exe = exe?;
    rules.iter().find(|(name, _)| name == exe).map(|(_, rule)| *rule)
}

/// Which app counts as in front after a foreground change to `front` (its executable `exe`),
/// given the one before. The taskbar, Start and Alt+Tab are passed through on the way
/// somewhere, so they keep the answer (no flash of the island while switching away from a
/// hidden app); the desktop is no app.
pub fn front_app(front: Front, exe: Option<String>, was: Option<String>) -> Option<String> {
    match front {
        Front::Shell => was,
        Front::Desktop => None,
        Front::App => exe,
    }
}

/// The lower-cased executable name ("chrome.exe") of the process that owns `hwnd`.
pub fn exe_of(hwnd: HWND) -> Option<String> {
    if hwnd.is_invalid() {
        return None;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    exe_of_pid(pid)
}

/// The lower-cased executable name of process `pid`.
pub fn exe_of_pid(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buffer = [0u16; 1024];
        let mut len = buffer.len() as u32;
        let named = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &mut len);
        _ = CloseHandle(process);
        named.ok()?;
        let path = String::from_utf16_lossy(&buffer[..len as usize]);
        Some(path.rsplit(['\\', '/']).next().unwrap_or(&path).to_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_are_read_leniently() {
        let value: Value = serde_json::from_str(r#"{ "Game.EXE": "hide", "obs64.exe": "decisions", "x.exe": "later", "": "hide", "n.exe": 3 }"#).unwrap();
        let mut rules = parse(Some(&value));
        rules.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(rules, vec![("game.exe".to_string(), AppRule::Hide), ("obs64.exe".to_string(), AppRule::OnlyDecisions)]);
        assert!(parse(None).is_empty());
        assert!(parse(Some(&Value::from("hide"))).is_empty());
    }

    #[test]
    fn the_app_in_front_picks_its_rule() {
        let rules = vec![("game.exe".to_string(), AppRule::Hide), ("obs64.exe".to_string(), AppRule::OnlyDecisions)];
        assert_eq!(rule_for(&rules, Some("game.exe")), Some(AppRule::Hide));
        assert_eq!(rule_for(&rules, Some("obs64.exe")), Some(AppRule::OnlyDecisions));
        assert_eq!(rule_for(&rules, Some("chrome.exe")), None);
        assert_eq!(rule_for(&rules, None), None);
        assert_eq!(rule_for(&[], Some("game.exe")), None);
    }

    #[test]
    fn passing_through_the_shell_keeps_the_app() {
        let game = || Some("game.exe".to_string());
        assert_eq!(front_app(Front::App, Some("chrome.exe".into()), game()).as_deref(), Some("chrome.exe"));
        assert_eq!(front_app(Front::Shell, None, game()), game(), "the taskbar on the way out doesn't flash the island");
        assert_eq!(front_app(Front::Desktop, None, game()), None);
    }
}
