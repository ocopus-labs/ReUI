//! The window's content as plain data: pages are lists of blocks (the intro, groups of
//! rows), and a row is a title, one quiet line and the controls it groups at its right.
//! Pages build this from the settings; [`layout`] places it; `render` draws it.

pub mod layout;

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
}

impl Row {
    pub fn new(id: &str, title: &str) -> Self {
        Self { id: id.into(), title: title.into(), detail: None, tone: Tone::Secondary, mono: false, parts: Vec::new(), enabled: true }
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

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    /// The page's title (the hero) and its one paragraph.
    Intro { title: String, body: Option<String> },
    /// Rows in one well, with an optional eyebrow above.
    Group { label: Option<String>, rows: Vec<Row> },
    /// Big tabular figures with a caption each (About's idle cost), in a well.
    Figures { label: Option<String>, items: Vec<(String, String)> },
}

pub fn group(label: &str, rows: Vec<Row>) -> Block {
    Block::Group { label: (!label.is_empty()).then(|| label.to_string()), rows }
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

/// What the pointer is over.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Hover {
    #[default]
    None,
    Nav(usize),
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
