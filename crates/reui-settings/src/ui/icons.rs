//! Glyphs (Segoe Fluent Icons code points) and the icon tiles pages and rows use. Each page
//! has one colour (`prefs.section`), used for its tile and its island preview's backdrop.

use reui_design::tokens::{color::system, prefs::section};

use super::Icon;

pub const GEAR: char = '\u{E713}';
pub const MUSIC: char = '\u{EC4F}';
pub const PASTE: char = '\u{E77F}';
pub const VOLUME: char = '\u{E767}';
pub const DEVICES: char = '\u{E772}';
pub const BLUETOOTH: char = '\u{E702}';
pub const CODE: char = '\u{E943}';
pub const LOCK: char = '\u{E72E}';
pub const POWER: char = '\u{E7E8}';
pub const MOON: char = '\u{E708}';
pub const DELETE: char = '\u{E74D}';
pub const FOLDER: char = '\u{E8B7}';
pub const HELP: char = '\u{E897}';
pub const BUG: char = '\u{EBE8}';
pub const PAGE: char = '\u{E7C3}';
pub const MONITOR: char = '\u{E7F4}';
pub const FULLSCREEN: char = '\u{E740}';
/// A picture: the wallpaper, so the desktop.
pub const WALLPAPER: char = '\u{E8B9}';
pub const VIEW: char = '\u{E890}';
/// ViewAll: the view switcher.
pub const TABS: char = '\u{E8A9}';
pub const KEYBOARD: char = '\u{E765}';
pub const CLOCK: char = '\u{E823}';
/// TouchPointer: how the island opens.
pub const POINTER: char = '\u{E7C9}';
pub const MIC: char = '\u{E720}';
pub const BATTERY_LOW: char = '\u{E851}';
pub const BATTERY: char = '\u{E859}';
pub const BOLT: char = '\u{E945}';
pub const WARNING: char = '\u{E7BA}';
pub const USB: char = '\u{E88E}';
pub const PHONE: char = '\u{E8EA}';
pub const HEADPHONES: char = '\u{E7F6}';
pub const LINK: char = '\u{E71B}';
pub const LYRICS: char = '\u{E8D2}';
pub const UPDATE: char = '\u{E777}';
pub const DOCUMENT: char = '\u{E8A5}';
pub const SEARCH: char = '\u{E721}';
pub const BACK: char = '\u{E76B}';
pub const FORWARD: char = '\u{E76C}';
pub const UP: char = '\u{E70E}';
pub const DOWN: char = '\u{E70D}';
pub const MINIMIZE: char = '\u{E921}';
pub const CLOSE: char = '\u{E8BB}';
pub const CHECK: char = '\u{E73E}';
pub const CANCEL: char = '\u{E711}';
pub const STOPWATCH: char = '\u{E916}';
pub const SPARKLE: char = '\u{E734}';
pub const PALETTE: char = '\u{E790}';
pub const CAMERA: char = '\u{E722}';
pub const SCREEN: char = MONITOR;
pub const LOCATION: char = '\u{E81D}';
/// A pulse line: the system stats.
pub const DIAGNOSTIC: char = '\u{E9D9}';

pub fn tile(glyph: char, color: reui_design::Color) -> Icon {
    Icon::Glyph(glyph, color)
}

pub fn general() -> Icon {
    tile(GEAR, section::GENERAL)
}
pub fn appearance() -> Icon {
    tile(PALETTE, section::APPEARANCE)
}
pub fn now_playing() -> Icon {
    tile(MUSIC, section::NOW_PLAYING)
}
pub fn clipboard() -> Icon {
    tile(PASTE, section::CLIPBOARD)
}
pub fn osd() -> Icon {
    tile(VOLUME, section::OSD)
}
pub fn devices() -> Icon {
    tile(DEVICES, section::DEVICES)
}
pub fn timers() -> Icon {
    tile(STOPWATCH, section::TIMERS)
}
pub fn indicators() -> Icon {
    tile(MIC, section::INDICATORS)
}
pub fn stats() -> Icon {
    tile(DIAGNOSTIC, section::STATS)
}
pub fn agents() -> Icon {
    tile(CODE, section::AGENTS)
}
pub fn privacy() -> Icon {
    tile(LOCK, section::PRIVACY)
}
pub fn welcome() -> Icon {
    tile(SPARKLE, section::WELCOME)
}
pub fn shortcut() -> Icon {
    tile(KEYBOARD, section::CLIPBOARD)
}
pub fn startup() -> Icon {
    tile(POWER, system::GREEN)
}
pub fn folder() -> Icon {
    tile(FOLDER, section::FOLDER)
}
pub fn danger(glyph: char) -> Icon {
    tile(glyph, section::DANGER)
}
pub fn help(glyph: char) -> Icon {
    tile(glyph, section::HELP)
}
pub fn gray(glyph: char) -> Icon {
    tile(glyph, system::GRAY)
}
