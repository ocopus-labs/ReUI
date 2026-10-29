//! The volume card's mixer: which apps it lists and in what order, their levels as set
//! from the card, and its rows. Pure; the sessions come from `audio::watch_sessions`,
//! which runs only while the card is open with the mixer showing.
//!
//! - **Apps playing now** are listed, in the order they started playing.
//! - **A line never moves or vanishes under the pointer:** an app that stops playing while
//!   the card is open keeps its line (turning down a paused player is normal), and new ones
//!   join at the end. Only an app whose session is gone (it quit) leaves.
//! - Reopening the card starts over with what's playing then.

use reui_protocol::{Channel, ImageId};

use crate::modules::audio::AppSession;

/// One app's line.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub key: u32,
    pub exe: String,
    pub volume: f32,
    pub muted: bool,
}

#[derive(Debug, Default)]
pub struct Mixer {
    lines: Vec<Line>,
}

impl Mixer {
    /// The sessions as they are now.
    pub fn update(&mut self, apps: &[AppSession]) {
        self.lines.retain(|l| apps.iter().any(|a| a.key == l.key));
        for line in &mut self.lines {
            if let Some(a) = apps.iter().find(|a| a.key == line.key) {
                (line.volume, line.muted) = (a.volume, a.muted);
            }
        }
        for a in apps.iter().filter(|a| a.active) {
            if !self.lines.iter().any(|l| l.key == a.key) {
                self.lines.push(Line { key: a.key, exe: a.exe.clone(), volume: a.volume, muted: a.muted });
            }
        }
    }

    /// The card set a level: shown at once, before the session's event confirms it.
    pub fn set_volume(&mut self, key: u32, volume: f32) {
        if let Some(l) = self.lines.iter_mut().find(|l| l.key == key) {
            l.volume = volume;
        }
    }

    pub fn set_muted(&mut self, key: u32, muted: bool) {
        if let Some(l) = self.lines.iter_mut().find(|l| l.key == key) {
            l.muted = muted;
        }
    }

    pub fn muted(&self, key: u32) -> Option<bool> {
        self.lines.iter().find(|l| l.key == key).map(|l| l.muted)
    }

    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    /// The card closed or the mixer was turned off.
    pub fn clear(&mut self) {
        self.lines.clear();
    }
}

/// `app:<key>:<0..1>` → (key, level).
pub fn parse_level(action: &str) -> Option<(u32, f32)> {
    let (key, value) = action.strip_prefix("app:")?.split_once(':')?;
    let value = value.parse::<f32>().ok().filter(|v| v.is_finite())?;
    Some((key.parse().ok()?, value.clamp(0.0, 1.0)))
}

/// `app-mute:<key>` → key (the icon flips the app's mute).
pub fn parse_mute(action: &str) -> Option<u32> {
    action.strip_prefix("app-mute:")?.parse().ok()
}

/// The rows' channels. `name` and `icon` come from the exe path.
pub fn channels(lines: &[Line], name: impl Fn(&str) -> String, icon: impl Fn(&str) -> Option<ImageId>) -> Vec<Channel> {
    lines
        .iter()
        .map(|l| Channel { label: name(&l.exe), icon: icon(&l.exe), value: l.volume.clamp(0.0, 1.0), muted: l.muted, action: format!("app:{}:", l.key), mute: format!("app-mute:{}", l.key) })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(key: u32, active: bool, volume: f32) -> AppSession {
        AppSession { key, exe: format!(r"C:\Apps\app{key}.exe"), volume, muted: false, active }
    }

    #[test]
    fn osd_mixer_lists_whats_playing_and_keeps_its_lines() {
        let mut m = Mixer::default();
        m.update(&[app(1, true, 0.5), app(2, false, 1.0), app(3, true, 0.2)]);
        assert_eq!(m.lines().iter().map(|l| l.key).collect::<Vec<_>>(), [1, 3], "only what's playing");
        // 1 pauses, 2 starts: 1 keeps its place, 2 joins at the end.
        m.update(&[app(1, false, 0.4), app(2, true, 1.0), app(3, true, 0.2)]);
        assert_eq!(m.lines().iter().map(|l| (l.key, l.volume)).collect::<Vec<_>>(), [(1, 0.4), (3, 0.2), (2, 1.0)]);
        // 3 quits.
        m.update(&[app(1, false, 0.4), app(2, true, 1.0)]);
        assert_eq!(m.lines().iter().map(|l| l.key).collect::<Vec<_>>(), [1, 2]);
        m.clear();
        m.update(&[app(1, false, 0.4), app(2, true, 1.0)]);
        assert_eq!(m.lines().len(), 1, "reopened: what's playing then");
    }

    #[test]
    fn osd_mixer_levels_and_actions() {
        let mut m = Mixer::default();
        m.update(&[app(7, true, 0.5)]);
        m.set_volume(7, 0.25);
        m.set_muted(7, true);
        assert_eq!((m.lines()[0].volume, m.muted(7)), (0.25, Some(true)));
        let ch = channels(m.lines(), |exe| exe.rsplit('\\').next().unwrap().into(), |_| None);
        assert_eq!((ch[0].label.as_str(), ch[0].action.as_str(), ch[0].mute.as_str(), ch[0].muted), ("app7.exe", "app:7:", "app-mute:7", true));
        assert_eq!(parse_level("app:7:0.42"), Some((7, 0.42)));
        assert_eq!(parse_level("app:7:9"), Some((7, 1.0)));
        assert_eq!((parse_level("app:x:0.4"), parse_level("app:7:NaN"), parse_level("volume:0.4")), (None, None, None));
        assert_eq!((parse_mute("app-mute:7"), parse_mute("app-mute:")), (Some(7), None));
    }
}
