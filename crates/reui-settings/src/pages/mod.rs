//! The pages: General, one per feature, Privacy & data, About, and the welcome's three
//! steps (`--welcome`). Each builds its blocks from the [`State`] and handles what its
//! controls do. Rows bound straight to a settings key
//! have the id `file:key` ("media:lyrics"), and their toggles are handled here for all.

mod about;
mod agents;
mod appearance;
mod calendar;
mod clipboard;
mod devices;
mod downloads;
mod general;
mod indicators;
mod now_playing;
mod osd;
mod privacy;
mod shelf;
mod shortcut;
mod stats;
mod timers;
mod welcome;

pub use self::about::VERSION;
pub use self::shortcut::chord;
pub use self::welcome::{finish as finish_welcome, start as start_welcome};

use serde_json::Value;

use crate::model::files::File;
use crate::state::State;
use crate::system;
use crate::ui::{button, footed, group, icons, Act, Block, Icon, Moment, Part, Preview, Role, Row, Target, Tone};
use reui_design::look::Look;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    General,
    Appearance,
    NowPlaying,
    Clipboard,
    Shelf,
    Osd,
    Devices,
    Timers,
    Calendar,
    Indicators,
    Stats,
    /// Downloads & screenshots: the file moments.
    Downloads,
    Agents,
    Privacy,
    About,
    /// The welcome's steps: what ReUI shows, the clipboard shortcut, start with Windows.
    WelcomeFeatures,
    WelcomeShortcut,
    WelcomeStartup,
}

impl Page {
    /// The Settings window's pages, in the nav.
    pub const ALL: [Page; 15] = [Page::General, Page::Appearance, Page::NowPlaying, Page::Clipboard, Page::Shelf, Page::Osd, Page::Devices, Page::Timers, Page::Calendar, Page::Indicators, Page::Stats, Page::Downloads, Page::Agents, Page::Privacy, Page::About];
    /// The welcome's steps, in its nav.
    pub const WELCOME: [Page; 3] = [Page::WelcomeFeatures, Page::WelcomeShortcut, Page::WelcomeStartup];

    pub fn title(self) -> &'static str {
        match self {
            Page::General => "General",
            Page::Appearance => "Appearance",
            Page::NowPlaying => "Now Playing",
            Page::Clipboard => "Clipboard",
            Page::Shelf => "Shelf",
            Page::Osd => "Volume & brightness",
            Page::Devices => "Devices",
            Page::Timers => "Timers",
            Page::Calendar => "Calendar",
            Page::Indicators => "Mic & camera",
            Page::Stats => "System stats",
            Page::Downloads => "Downloads & screenshots",
            Page::Agents => "Coding agents",
            Page::Privacy => "Privacy & data",
            Page::About => "About",
            Page::WelcomeFeatures => "What ReUI shows",
            Page::WelcomeShortcut => "Clipboard shortcut",
            Page::WelcomeStartup => "Start with Windows",
        }
    }

    /// The pages in the same nav as this one.
    pub fn nav(self) -> &'static [Page] {
        if self.is_welcome() {
            &Page::WELCOME
        } else {
            &Page::ALL
        }
    }

    pub fn is_welcome(self) -> bool {
        Page::WELCOME.contains(&self)
    }

    /// Place in its nav.
    pub fn index(self) -> usize {
        self.nav().iter().position(|p| *p == self).unwrap_or(0)
    }

    /// The sidebar's sections: General; the island's features; privacy and About.
    pub fn section(self) -> usize {
        match self {
            Page::NowPlaying | Page::Clipboard | Page::Shelf | Page::Osd | Page::Devices | Page::Timers | Page::Calendar | Page::Indicators | Page::Stats | Page::Downloads | Page::Agents => 1,
            Page::Privacy | Page::About => 2,
            _ => 0,
        }
    }

    /// Its tile in the sidebar and the header card.
    pub fn icon(self) -> Icon {
        match self {
            Page::General => icons::general(),
            Page::Appearance => icons::appearance(),
            Page::NowPlaying => icons::now_playing(),
            Page::Clipboard => icons::clipboard(),
            Page::Shelf => icons::shelf(),
            Page::Osd => icons::osd(),
            Page::Devices => icons::devices(),
            Page::Timers => icons::timers(),
            Page::Calendar => icons::calendar(),
            Page::Indicators => icons::indicators(),
            Page::Stats => icons::stats(),
            Page::Downloads => icons::downloads(),
            Page::Agents => icons::agents(),
            Page::Privacy => icons::privacy(),
            Page::About => Icon::Mark,
            Page::WelcomeFeatures => icons::welcome(),
            Page::WelcomeShortcut => icons::shortcut(),
            Page::WelcomeStartup => icons::startup(),
        }
    }

    /// The island this page's header card shows.
    pub fn moment(self) -> Option<Moment> {
        match self {
            Page::General => Some(Moment::Compact),
            Page::Appearance => Some(Moment::Timer),
            Page::NowPlaying | Page::WelcomeFeatures => Some(Moment::Music),
            Page::Clipboard | Page::WelcomeShortcut => Some(Moment::Clipboard),
            Page::Osd => Some(Moment::Meter),
            Page::Devices => Some(Moment::Devices),
            Page::Timers => Some(Moment::Timer),
            Page::Stats => Some(Moment::Stats),
            Page::Agents => Some(Moment::Permission),
            _ => None,
        }
    }

    /// The page's island in the user's look.
    pub fn preview(self, state: &State) -> Option<Preview> {
        let switcher = state.files.doc(crate::model::files::File::Island).bool("view_switcher");
        self.moment().map(|moment| Preview { moment, look: look(state), switcher })
    }

    /// `--page clipboard`, for opening on a page.
    pub fn from_arg(arg: &str) -> Option<Page> {
        let arg = arg.to_ascii_lowercase();
        Page::ALL.into_iter().find(|p| p.title().to_ascii_lowercase().starts_with(&arg) || format!("{p:?}").to_ascii_lowercase() == arg)
    }
}

/// What a control asked the window to do besides changing settings.
#[derive(Debug, PartialEq)]
pub enum Effect {
    None,
    Navigate(Page),
    /// Listen for a new shortcut for this file.
    Record(File),
    /// The welcome is done: close, or carry on into Settings.
    Finish { open_settings: bool },
    /// About's Install: download and verify the update.
    Download,
    /// About's Check for updates.
    CheckUpdates,
}

pub fn build(page: Page, state: &State) -> Vec<Block> {
    let mut blocks = match page {
        Page::General => general::build(state),
        Page::Appearance => appearance::build(state),
        Page::NowPlaying => now_playing::build(state),
        Page::Clipboard => clipboard::build(state),
        Page::Shelf => shelf::build(state),
        Page::Osd => osd::build(state),
        Page::Devices => devices::build(state),
        Page::Timers => timers::build(state),
        Page::Calendar => calendar::build(state),
        Page::Indicators => indicators::build(state),
        Page::Stats => stats::build(state),
        Page::Downloads => downloads::build(state),
        Page::Agents => agents::build(state),
        Page::Privacy => privacy::build(state),
        Page::About => about::build(state),
        Page::WelcomeFeatures | Page::WelcomeShortcut | Page::WelcomeStartup => welcome::build(page, state),
    };
    if let Some(error) = &state.write_error {
        let row = Row::new("write-error", "Couldn't save").detail(error.clone()).tone(Tone::Negative).icon(icons::danger(icons::WARNING));
        blocks.insert(1.min(blocks.len()), group("", vec![row]));
    }
    blocks
}

pub fn act(page: Page, state: &mut State, target: &Target, act: Act) -> Effect {
    // Bound toggles and the error cards work the same on every page.
    if let Some(prefix) = target.row.strip_prefix("error:") {
        if let Some(file) = File::from_prefix(prefix) {
            system::edit_file(&state.files.doc(file).path);
        }
        return Effect::None;
    }
    if let Some((file, key)) = bound(&target.row) {
        if act == Act::Press {
            let on = state.files.doc(file).bool(key);
            state.set(file, key, Value::Bool(!on));
            if key == "enabled" {
                // Modules are started (or not) with the hub.
                state.restart_note = state.hub_running();
            }
            return Effect::None;
        }
    }
    match page {
        Page::General => general::act(state, target, act),
        Page::Appearance => appearance::act(state, target, act),
        Page::NowPlaying => now_playing::act(state, target, act),
        Page::Clipboard => clipboard::act(state, target, act),
        Page::Shelf => shelf::act(state, target, act),
        Page::Osd => osd::act(state, target, act),
        Page::Devices => devices::act(state, target, act),
        Page::Timers => timers::act(state, target, act),
        Page::Calendar => calendar::act(state, target, act),
        Page::Indicators => Effect::None,
        Page::Stats => stats::act(state, target, act),
        Page::Downloads => downloads::act(state, target, act),
        Page::Agents => agents::act(state, target, act),
        Page::Privacy => privacy::act(state, target, act),
        Page::About => about::act(state, target, act),
        Page::WelcomeFeatures | Page::WelcomeShortcut | Page::WelcomeStartup => welcome::act(page, state, target, act),
    }
}

/// A dropdown's entries and the checked one.
pub fn menu(page: Page, state: &mut State, target: &Target) -> Option<(Vec<String>, Option<usize>)> {
    match page {
        Page::NowPlaying => now_playing::menu(state, target),
        Page::Clipboard => clipboard::menu(state, target),
        Page::Shelf => shelf::menu(state, target),
        Page::General => general::menu(state, target),
        Page::Timers => timers::menu(state, target),
        Page::Calendar => calendar::menu(state, target),
        _ => None,
    }
}

/// `"media:lyrics"` → (Media, "lyrics").
fn bound(row: &str) -> Option<(File, &str)> {
    let (prefix, key) = row.split_once(':')?;
    Some((File::from_prefix(prefix)?, key))
}

/// A toggle row bound to `file`'s `key`.
fn flag(state: &State, file: File, key: &str, title: &str, detail: &str) -> Row {
    let doc = state.files.doc(file);
    let row = Row::new(&format!("{}:{key}", file.prefix()), title).part(Part::Toggle { on: doc.bool(key) }).enabled(doc.error().is_none());
    if detail.is_empty() {
        row
    } else {
        row.detail(detail)
    }
}

/// The error card for a file that isn't valid JSON (P-4): its rows are disabled meanwhile.
fn file_error(state: &State, file: File) -> Option<Block> {
    let doc = state.files.doc(file);
    let error = doc.error()?;
    let row = Row::new(&format!("error:{}", file.prefix()), &format!("{} isn't valid JSON", doc.name()))
        .icon(icons::danger(icons::WARNING))
        .detail(error.to_string())
        .tone(Tone::Negative)
        .part(button("Open file", Role::Secondary));
    Some(group("", vec![row]))
}

/// The look the island has now (Settings > Appearance, Windows' app mode and accent).
pub fn look(state: &State) -> Look {
    Look::new(&state.files.appearance(), system::apps_light(), system::accent_color())
}

/// The page's header card: its tile, title, paragraph and island.
fn intro(state: &State, page: Page, body: &str) -> Block {
    Block::Intro { eyebrow: None, title: page.title().into(), body: (!body.is_empty()).then(|| body.into()), icon: Some(page.icon()), preview: page.preview(state), row: None }
}

/// A feature's on/off switch (`enabled` in its file), in its header card.
fn enable(blocks: &mut [Block], state: &State, file: File, title: &str) {
    let row = flag(state, file, "enabled", title, "");
    let row = if state.restart_note { row.detail("Takes effect when ReUI restarts").tone(Tone::Warning) } else { row };
    if let Some(Block::Intro { row: slot, .. }) = blocks.first_mut() {
        *slot = Some(row);
    }
}

/// The header card, then the file's error card if it has one.
fn head(state: &State, file: File, page: Page, body: &str) -> Vec<Block> {
    let mut blocks = vec![intro(state, page, body)];
    blocks.extend(file_error(state, file));
    blocks
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::model::doc::tests::temp_dir;
    use crate::paths::Paths;

    pub fn state(name: &str) -> (State, std::path::PathBuf) {
        let dir = temp_dir(name);
        let paths = Paths { config: dir.join("roaming"), local: dir.join("local"), home: dir.join("home"), exe_dir: dir.clone(), sandbox: true };
        (State::new(paths), dir)
    }

    fn rows(blocks: &[Block]) -> Vec<&Row> {
        blocks.iter().flat_map(|b| if let Block::Group { rows, .. } = b { rows.iter().collect() } else { Vec::new() }).collect()
    }

    #[test]
    fn every_page_builds_and_bound_toggles_write_their_key() {
        let (mut state, dir) = state("pages");
        for page in Page::ALL.into_iter().chain(Page::WELCOME) {
            let blocks = build(page, &state);
            assert!(matches!(blocks[0], Block::Intro { .. }), "{page:?} starts with its intro");
        }
        act(Page::NowPlaying, &mut state, &Target::new("media:lyrics", 0), Act::Press);
        assert!(state.files.doc(File::Media).bool("lyrics"));
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("roaming").join("media.json")).unwrap()).unwrap();
        assert_eq!(saved["lyrics"], true);
        assert_eq!(saved["visualizer"], "real", "a new file gets every default");
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_invalid_file_shows_its_error_and_disables_its_rows() {
        let (mut state, dir) = state("pages-invalid");
        std::fs::create_dir_all(dir.join("roaming")).unwrap();
        std::fs::write(dir.join("roaming").join("osd.json"), "{ nope").unwrap();
        state.refresh();
        let blocks = build(Page::Osd, &state);
        let all = rows(&blocks);
        assert_eq!(all[0].id, "error:osd");
        assert!(all.iter().filter(|r| r.id.starts_with("osd:")).all(|r| !r.enabled));
        act(Page::Osd, &mut state, &Target::new("osd:volume", 0), Act::Press);
        assert_eq!(std::fs::read_to_string(dir.join("roaming").join("osd.json")).unwrap(), "{ nope");
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pages_are_found_by_argument() {
        assert_eq!(Page::from_arg("clip"), Some(Page::Clipboard));
        assert_eq!(Page::from_arg("osd"), Some(Page::Osd));
        assert_eq!(Page::from_arg("about"), Some(Page::About));
        assert_eq!(Page::from_arg("zzz"), None);
        assert_eq!(Page::from_arg("what"), None, "welcome steps aren't Settings pages");
        // About is always the last page, however many come before it.
        assert_eq!((Page::WelcomeShortcut.index(), Page::About.index()), (1, Page::ALL.len() - 1));
        assert_eq!(Page::from_arg("calendar"), Some(Page::Calendar));
        assert_eq!(Page::from_arg("downloads"), Some(Page::Downloads));
        assert_eq!(Page::from_arg("appearance"), Some(Page::Appearance));
        assert_eq!(Page::from_arg("timers"), Some(Page::Timers));
        assert_eq!(Page::from_arg("mic"), Some(Page::Indicators));
        assert_eq!(Page::from_arg("indicators"), Some(Page::Indicators));
        assert_eq!(Page::from_arg("privacy"), Some(Page::Privacy));
        assert_eq!(Page::from_arg("stats"), Some(Page::Stats));
        assert!(Page::WelcomeStartup.is_welcome() && !Page::General.is_welcome());
    }
}
