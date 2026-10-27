//! What the user has already been shown, in `%APPDATA%\ReUI\onboarding.json`:
//! `{ "welcomed": bool, "tips": { "<id>": true }, "offers": { "<id>": "accepted" | "declined" },
//! "counts": { "<id>": n } }`. Same rules as the other settings files: missing or mistyped
//! fields fall back to "not shown yet", keys this version doesn't know are kept, and a file
//! that isn't a JSON object is never overwritten (nothing is shown then, rather than the
//! same tip on every start).
//!
//! And the queue tips wait in, so they never stack: one shows at a time, each only while
//! it's still due.

use std::collections::{BTreeMap, VecDeque};
use std::path::Path;

use serde_json::{Map, Value};

/// The clipboard hotkey tip rides along the first this many copies.
pub const HOTKEY_TIMES: u32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Accepted,
    Declined,
}

impl Answer {
    fn as_str(self) -> &'static str {
        match self {
            Answer::Accepted => "accepted",
            Answer::Declined => "declined",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "accepted" => Some(Answer::Accepted),
            "declined" => Some(Answer::Declined),
            _ => None,
        }
    }
}

/// One-time teaching moments. Tips teach (shown, then done); offers ask (answered once).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tip {
    /// The clipboard history hotkey, with its current label ("Win+Alt+V").
    Hotkey(String),
    /// Replace Windows' volume pop-up?
    Flyout,
    /// Show this coding agent (module name: "claude") in the island?
    Agent(&'static str),
    /// Synced lyrics (network)?
    Lyrics,
}

impl Tip {
    pub fn id(&self) -> String {
        match self {
            Tip::Hotkey(_) => "clipboard-hotkey".into(),
            Tip::Flyout => "volume-flyout".into(),
            Tip::Agent(agent) => format!("agent-{agent}"),
            Tip::Lyrics => "lyrics".into(),
        }
    }

    pub fn is_offer(&self) -> bool {
        !matches!(self, Tip::Hotkey(_))
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Record {
    /// The first-run demo and welcome have happened (even if the welcome was closed with ✕).
    pub welcomed: bool,
    pub tips: BTreeMap<String, bool>,
    pub offers: BTreeMap<String, Answer>,
    /// How many times a repeating tip has shown.
    pub counts: BTreeMap<String, u32>,
    /// The file is there but isn't a JSON object: show nothing, write nothing.
    pub frozen: bool,
}

impl Record {
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).unwrap_or_else(|| {
                eprintln!("onboarding: {} isn't a JSON object; leaving it alone", path.display());
                Self { welcomed: true, frozen: true, ..Self::default() }
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => {
                eprintln!("onboarding: can't read {}: {e}", path.display());
                Self { welcomed: true, frozen: true, ..Self::default() }
            }
        }
    }

    /// `None` if the text isn't a JSON object.
    pub fn parse(text: &str) -> Option<Self> {
        let Value::Object(map) = serde_json::from_str(text).ok()? else { return None };
        let object = |key: &str| map.get(key).and_then(Value::as_object).cloned().unwrap_or_default();
        Some(Self {
            welcomed: map.get("welcomed").and_then(Value::as_bool).unwrap_or(false),
            tips: object("tips").into_iter().filter_map(|(k, v)| Some((k, v.as_bool()?))).collect(),
            offers: object("offers").into_iter().filter_map(|(k, v)| Some((k, Answer::parse(v.as_str()?)?))).collect(),
            counts: object("counts").into_iter().filter_map(|(k, v)| Some((k, u32::try_from(v.as_u64()?).ok()?))).collect(),
            frozen: false,
        })
    }

    /// Writes our keys over the file's, keeping the rest, atomically. Never over a file that
    /// isn't a JSON object.
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if self.frozen {
            return Ok(());
        }
        let mut map = match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<Value>(&text) {
                Ok(Value::Object(map)) => map,
                _ => return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "not a JSON object; left as it is")),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Map::new(),
            Err(e) => return Err(e),
        };
        map.insert("welcomed".into(), Value::Bool(self.welcomed));
        map.insert("tips".into(), Value::Object(self.tips.iter().map(|(k, v)| (k.clone(), Value::Bool(*v))).collect()));
        map.insert("offers".into(), Value::Object(self.offers.iter().map(|(k, v)| (k.clone(), Value::String(v.as_str().into()))).collect()));
        map.insert("counts".into(), Value::Object(self.counts.iter().map(|(k, v)| (k.clone(), Value::from(*v))).collect()));
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, serde_json::to_string_pretty(&Value::Object(map)).unwrap_or_default() + "\n")?;
        std::fs::rename(&temp, path)
    }

    /// Still worth showing: an offer nobody answered, a tip not yet done.
    pub fn due(&self, tip: &Tip) -> bool {
        if self.frozen {
            return false;
        }
        let id = tip.id();
        if tip.is_offer() {
            !self.offers.contains_key(&id)
        } else {
            !self.tips.get(&id).copied().unwrap_or(false)
        }
    }

    /// A tip was on screen. The hotkey tip is done after [`HOTKEY_TIMES`], others at once;
    /// offers wait for their answer.
    pub fn shown(&mut self, tip: &Tip) {
        if tip.is_offer() {
            return;
        }
        let id = tip.id();
        let n = self.counts.entry(id.clone()).or_insert(0);
        *n += 1;
        let times = if matches!(tip, Tip::Hotkey(_)) { HOTKEY_TIMES } else { 1 };
        if *n >= times {
            self.tips.insert(id, true);
        }
    }

    /// An offer was answered; "Not now" (or ignoring it) is final too.
    pub fn answer(&mut self, tip: &Tip, answer: Answer) {
        if tip.is_offer() {
            self.offers.insert(tip.id(), answer);
        }
    }
}

/// Tips waiting their turn, and the one on screen.
#[derive(Default, Debug)]
pub struct Queue {
    waiting: VecDeque<Tip>,
    pub showing: Option<Tip>,
}

impl Queue {
    /// Queues a tip if it's due and not already waiting or showing. `true` if queued.
    pub fn push(&mut self, tip: Tip, record: &Record) -> bool {
        let id = tip.id();
        let known = self.showing.as_ref().is_some_and(|t| t.id() == id) || self.waiting.iter().any(|t| t.id() == id);
        if known || !record.due(&tip) {
            return false;
        }
        self.waiting.push_back(tip);
        true
    }

    /// The next tip to show, unless one is showing or it's a bad moment (`quiet`:
    /// fullscreen, paused). Tips that stopped being due while they waited are dropped.
    pub fn next(&mut self, quiet: bool, record: &Record) -> Option<Tip> {
        if self.showing.is_some() || quiet {
            return None;
        }
        while let Some(tip) = self.waiting.pop_front() {
            if record.due(&tip) {
                self.showing = Some(tip.clone());
                return Some(tip);
            }
        }
        None
    }

    /// The tip on screen is gone.
    pub fn finish(&mut self) -> Option<Tip> {
        self.showing.take()
    }

    pub fn is_waiting(&self) -> bool {
        !self.waiting.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_what_it_writes_keeps_unknown_keys_and_never_clobbers_junk() {
        let dir = std::env::temp_dir().join(format!("reui-onboarding-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("onboarding.json");
        // First run: no file.
        let mut record = Record::load(&path);
        assert!(!record.welcomed && !record.frozen);
        record.welcomed = true;
        record.shown(&Tip::Hotkey("Win+Alt+V".into()));
        record.answer(&Tip::Lyrics, Answer::Declined);
        record.save_to(&path).unwrap();
        assert_eq!(Record::load(&path), record);

        std::fs::write(&path, r#"{ "future": 1, "welcomed": true, "offers": { "lyrics": "maybe", "agent-claude": "accepted" } }"#).unwrap();
        let loaded = Record::load(&path);
        assert_eq!(loaded.offers.get("agent-claude"), Some(&Answer::Accepted));
        assert!(!loaded.offers.contains_key("lyrics"));
        loaded.save_to(&path).unwrap();
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["future"].as_u64(), Some(1));

        std::fs::write(&path, "{ broken").unwrap();
        let broken = Record::load(&path);
        assert!(broken.welcomed && broken.frozen && !broken.due(&Tip::Lyrics));
        broken.save_to(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ broken");
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tips_show_once_and_the_hotkey_three_times() {
        let mut record = Record::default();
        let hotkey = Tip::Hotkey("Win+Alt+V".into());
        for _ in 0..HOTKEY_TIMES {
            assert!(record.due(&hotkey));
            record.shown(&hotkey);
        }
        assert!(!record.due(&hotkey));
    }

    #[test]
    fn offers_are_asked_until_answered_either_way() {
        let mut record = Record::default();
        record.shown(&Tip::Flyout);
        assert!(record.due(&Tip::Flyout), "showing alone doesn't settle an offer");
        record.answer(&Tip::Flyout, Answer::Accepted);
        assert!(!record.due(&Tip::Flyout));
        record.answer(&Tip::Agent("codex"), Answer::Declined);
        assert!(!record.due(&Tip::Agent("codex")));
        assert!(record.due(&Tip::Agent("claude")));
    }

    #[test]
    fn queue_shows_one_at_a_time_never_repeats_and_waits_out_quiet_moments() {
        let mut record = Record::default();
        let mut queue = Queue::default();
        assert!(queue.push(Tip::Flyout, &record));
        assert!(!queue.push(Tip::Flyout, &record), "already waiting");
        assert!(queue.push(Tip::Lyrics, &record));
        // Fullscreen or paused: nothing comes out, nothing is lost.
        assert_eq!(queue.next(true, &record), None);
        assert_eq!(queue.next(false, &record), Some(Tip::Flyout));
        // Never two at once.
        assert_eq!(queue.next(false, &record), None);
        assert!(!queue.push(Tip::Flyout, &record), "already showing");
        record.answer(&Tip::Flyout, Answer::Declined);
        assert_eq!(queue.finish(), Some(Tip::Flyout));
        assert!(!queue.push(Tip::Flyout, &record), "answered: never again");
        // Answered elsewhere (the Settings window) while waiting: dropped.
        record.answer(&Tip::Lyrics, Answer::Accepted);
        assert_eq!(queue.next(false, &record), None);
        assert!(!queue.is_waiting());
    }
}
