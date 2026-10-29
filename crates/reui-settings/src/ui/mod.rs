//! The window's content as plain data: pages are lists of blocks (the intro, groups of
//! rows), and a row is a title, one quiet line and the controls it groups at its right.
//! Pages build this from the settings; [`layout`] places it; `render` draws it.

pub mod chrome;
pub mod icons;
pub mod layout;
pub mod stage;

use reui_design::Color;

/// A coloured icon tile: a Segoe Fluent Icons glyph in white on a rounded square of the
/// colour (sidebar, header card, important rows), or one of the marks drawn as they are.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Icon {
    Glyph(char, Color),
    /// ReUI's mark: a light screen with the black island at its top.
    Mark,
    /// The Claude logo (design/assets/agents/claude.svg) on a dark tile.
    Claude,
}

/// What a header card's stage shows: one moment of the island, drawn in the user's
/// look. The Look is carried whole (a few hundred bytes, one per page build) so the
/// preview draws exactly what the island draws, and the stage is as tall as it needs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Preview {
    pub moment: Moment,
    pub look: reui_design::look::Look,
}

/// The one state of the island a page is about (sizes and drawing: [`stage`] and
/// `render::preview`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Moment {
    /// The compact pill with a second activity beside it (General).
    Compact,
    /// The open Now Playing card.
    Music,
    /// The clipboard panel.
    Clipboard,
    /// What was just copied (the clipboard's toast): the panel's stand-in on a stage too
    /// narrow for it at the look's size.
    Toast,
    /// The volume meter: the whole pill.
    Meter,
    /// Earbuds just connected.
    Devices,
    /// A running timer's card (Timers, and Appearance).
    Timer,
    /// How busy the PC is: System stats' gauges and facts.
    Stats,
    /// A coding agent asking permission.
    Permission,
}

/// A control's identity: its row and its place among the row's controls.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Target {
    pub row: String,
    pub part: usize,
}

impl Target {
    pub fn new(row: &str, part: usize) -> Self {
        Self { row: row.to_string(), part }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// White capsule, black text: the one thing to do.
    Primary,
    Secondary,
    /// Accent-tinted.
    Accent,
    /// Red-tinted; a second press confirms ([`Part::Button::armed`]).
    Destructive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Secondary,
    Positive,
    Warning,
    Negative,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    Toggle { on: bool },
    Segmented { options: Vec<String>, selected: usize },
    /// `value` 0..=1, with its reading at the left ("2 %").
    Slider { value: f32, reading: String },
    /// `armed`: a destructive button waiting for its confirming press (it then fills red).
    Button { label: String, role: Role, armed: bool },
    /// Key caps of the current hotkey; while recording, a quiet prompt instead.
    Hotkey { caps: Vec<String>, recording: bool },
    /// Shows the current choice; opens a menu.
    Dropdown { label: String },
    Field { text: String, placeholder: String },
    /// A circle that removes the row's item.
    Remove,
    /// A fact at the right, tabular ("12 MB").
    Value(String),
}

impl Part {
    /// Whether it takes the keyboard (and clicks).
    pub fn interactive(&self) -> bool {
        !matches!(self, Part::Value(_))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: String,
    pub title: String,
    pub detail: Option<String>,
    pub tone: Tone,
    /// Title set in the mono face (executable names, paths).
    pub mono: bool,
    pub parts: Vec<Part>,
    /// Off: drawn quieter and skipped by the keyboard.
    pub enabled: bool,
    /// No well: just its controls (the welcome's Next / Done).
    pub bare: bool,
    /// A leading icon tile, for the rows that lead somewhere or matter most.
    pub icon: Option<Icon>,
}

impl Row {
    pub fn new(id: &str, title: &str) -> Self {
        Self { id: id.into(), title: title.into(), detail: None, tone: Tone::Secondary, mono: false, parts: Vec::new(), enabled: true, bare: false, icon: None }
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }

    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }

    pub fn part(mut self, part: Part) -> Self {
        self.parts.push(part);
        self
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn bare(mut self) -> Self {
        self.bare = true;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum Block {
    /// The page's header card: its icon, title (the hero) and one paragraph, with an
    /// optional eyebrow above the title ("Step 1 of 3"), the feature's island above it all,
    /// and the feature's on/off row at its right (only that row's controls and its quiet
    /// line show).
    Intro { eyebrow: Option<String>, title: String, body: Option<String>, icon: Option<Icon>, preview: Option<Preview>, row: Option<Row> },
    /// Rows in one container, with an optional section header above and a footnote under.
    Group { label: Option<String>, rows: Vec<Row>, footer: Option<String> },
    /// Big tabular figures with a caption each (About's idle cost), in a well.
    Figures { label: Option<String>, items: Vec<(String, String)> },
    /// A title and wrapped plain text in a well (release notes, What's new); `accent`
    /// draws the well tinted, for something new.
    Text { label: Option<String>, title: String, body: String, accent: bool },
}

pub fn group(label: &str, rows: Vec<Row>) -> Block {
    Block::Group { label: (!label.is_empty()).then(|| label.to_string()), rows, footer: None }
}

/// A group with a footnote under its container.
pub fn footed(label: &str, rows: Vec<Row>, footer: &str) -> Block {
    Block::Group { label: (!label.is_empty()).then(|| label.to_string()), rows, footer: Some(footer.to_string()) }
}

pub fn button(label: &str, role: Role) -> Part {
    Part::Button { label: label.into(), role, armed: false }
}

/// What a control was asked to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    /// A press (button, toggle, remove, dropdown, field's Enter).
    Press,
    /// A segment, or a menu entry.
    Choose(usize),
    /// A slider at 0..=1.
    Value(f32),
}

/// What the pointer is over. Also the key of a hover fill's eased amount.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum Hover {
    #[default]
    None,
    /// The sidebar's nth visible page.
    Nav(usize),
    /// The ReUI row at the top of the sidebar (opens About).
    Account,
    Search,
    /// 0 back, 1 forward.
    History(usize),
    /// 0 minimise, 1 close.
    Caption(usize),
    /// A control, and which of its cells (segment) if any.
    Part(Target, Option<usize>),
    Scrollbar,
    Menu(usize),
}

/// Where the keyboard is.
#[derive(Clone, Debug, PartialEq)]
pub enum Focus {
    /// The sidebar's search field (typing filters the pages).
    Search,
    /// The nav list (arrows switch pages).
    Nav,
    Part(Target),
}

/// An open dropdown menu, in window coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct Menu {
    pub target: Target,
    pub items: Vec<String>,
    pub checked: Option<usize>,
    pub rect: layout::Rect,
    /// The highlighted entry (pointer or arrows).
    pub hot: Option<usize>,
    pub scroll: f32,
}
