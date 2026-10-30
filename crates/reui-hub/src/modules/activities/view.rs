//! How a script's activity reads. Running: a ring for progress (else its glyph) and the
//! percentage (else the title) in the pill; open, the title as eyebrow, the detail as hero
//! (else the time since it started, counting up by itself) and a progress row. Done or
//! failed: a Brief with a check or a cross.

use reui_protocol::{Art, CardSize, Clock, CompactView, Element, ExpandedView, Headline, Rgb, Row, Tone};

use super::board::Entry;
use super::request::{Glyph, Phase};
use crate::hub::View;

const RUNNING_ACCENT: Rgb = Rgb(10, 132, 255);
const DONE_ACCENT: Rgb = Rgb(48, 209, 88);
const FAILED_ACCENT: Rgb = Rgb(255, 69, 58);
const GLYPH_DONE: char = '\u{E73E}';
const GLYPH_FAILED: char = '\u{E711}';

pub fn view(e: &Entry, now: u64, now_unix: u64) -> View {
    match e.phase {
        Phase::Running => running(e, now, now_unix),
        Phase::Done | Phase::Failed => finished(e, now),
    }
}

fn running(e: &Entry, now: u64, now_unix: u64) -> View {
    let mark = Element::Glyph { glyph: glyph(e.glyph), tone: Tone::Accent };
    let (compact_leading, trailing) = match e.progress {
        Some(p) => (Element::Ring(p), Element::Text { text: percent(p), tone: Tone::Primary }),
        None => (mark.clone(), Element::Text { text: short(&title(e)), tone: Tone::Primary }),
    };
    let elapsed = Clock { value_ms: now.saturating_sub(e.started), at_unix_ms: now_unix, running: true, down: false };
    let headline = match &e.detail {
        Some(d) => Headline::Text(d.clone()),
        None => Headline::Clock { clock: elapsed, tenths: false },
    };
    let rows = e.progress.map(|p| Row::Progress { fraction: p, leading: Some(percent(p)), trailing: None }).into_iter().collect();
    View {
        accent: Some(RUNNING_ACCENT),
        audio: None,
        minimal: compact_leading.clone(),
        compact: CompactView { leading: compact_leading, trailing },
        expanded: Some(ExpandedView {
            leading: Element::Art(Art::Glyph(glyph(e.glyph))),
            eyebrow: Some(title(e)),
            headline,
            detail: None,
            trailing: Element::Empty,
            rows,
            context: None,
            size: CardSize::Regular,
            tint: false,
            pager: None,
            open: None,
        }),
    }
}

fn finished(e: &Entry, now: u64) -> View {
    let ok = e.phase == Phase::Done;
    let (word, sign, accent, tone) = if ok { ("Done", GLYPH_DONE, DONE_ACCENT, Tone::Positive) } else { ("Failed", GLYPH_FAILED, FAILED_ACCENT, Tone::Negative) };
    let took = e.finished.unwrap_or(now).saturating_sub(e.started);
    let detail = e.detail.clone().or_else(|| (took >= 1_000).then(|| format!("Took {}", length_text(took))));
    View {
        accent: Some(accent),
        audio: None,
        compact: CompactView { leading: Element::Glyph { glyph: sign, tone }, trailing: Element::Text { text: word.into(), tone } },
        minimal: Element::Glyph { glyph: sign, tone },
        expanded: Some(ExpandedView {
            leading: Element::Art(Art::Glyph(glyph(e.glyph))),
            eyebrow: Some(title(e)),
            headline: Headline::Text(word.into()),
            detail,
            trailing: Element::Empty,
            rows: Vec::new(),
            context: None,
            size: CardSize::Regular,
            tint: false,
            pager: None,
            open: None,
        }),
    }
}

/// The title, else the id as a name ("nightly-backup" → "Nightly backup").
fn title(e: &Entry) -> String {
    if let Some(t) = &e.title {
        return t.clone();
    }
    let spaced = e.id.replace(['-', '_', '.'], " ");
    let mut chars = spaced.trim().chars();
    chars.next().map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
}

fn percent(p: f32) -> String {
    // Rounded down, so 99.6 % never reads as done.
    format!("{}%", (p * 100.0).floor() as u32)
}

/// Short enough for the pill's side.
fn short(text: &str) -> String {
    const MAX: usize = 14;
    if text.chars().count() <= MAX {
        return text.to_string();
    }
    let cut: String = text.chars().take(MAX - 1).collect();
    format!("{}…", cut.trim_end())
}

/// "45 s", "3 min", "1 h 5 min".
fn length_text(ms: u64) -> String {
    let secs = ms / 1000;
    match (secs / 3600, secs / 60 % 60, secs % 60) {
        (0, 0, s) => format!("{s} s"),
        (0, m, _) => format!("{m} min"),
        (h, 0, _) => format!("{h} h"),
        (h, m, _) => format!("{h} h {m} min"),
    }
}

/// Segoe Fluent Icons.
pub fn glyph(g: Glyph) -> char {
    match g {
        Glyph::Script => '\u{E756}',
        Glyph::Build => '\u{E90F}',
        Glyph::Test => '\u{E9D5}',
        Glyph::Download => '\u{E896}',
        Glyph::Upload => '\u{E898}',
        Glyph::Sync => '\u{E895}',
        Glyph::Backup => '\u{EDA2}',
        Glyph::Package => '\u{E7B8}',
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> Entry {
        Entry {
            id: "nightly-backup".into(),
            source: "pwsh".into(),
            title: None,
            detail: None,
            progress: None,
            phase: Phase::Running,
            glyph: Glyph::Backup,
            keep: false,
            started: 1_000,
            heard: 1_000,
            finished: None,
            gone_at: None,
        }
    }

    #[test]
    fn progress_is_a_ring_and_a_row() {
        let v = view(&Entry { progress: Some(0.426), title: Some("Build".into()), detail: Some("Linking".into()), ..entry() }, 5_000, 60_000);
        assert_eq!(v.compact.leading, Element::Ring(0.426));
        assert_eq!(v.compact.trailing, Element::Text { text: "42%".into(), tone: Tone::Primary });
        let card = v.expanded.unwrap();
        assert_eq!((card.eyebrow.as_deref(), card.headline), (Some("Build"), Headline::Text("Linking".into())));
        assert_eq!(card.rows, [Row::Progress { fraction: 0.426, leading: Some("42%".into()), trailing: None }]);
        assert_eq!(percent(0.996), "99%");
    }

    #[test]
    fn no_detail_counts_up_from_the_start() {
        let v = view(&entry(), 61_000, 120_000);
        assert_eq!(v.compact.trailing, Element::Text { text: "Nightly backup".into(), tone: Tone::Primary });
        let card = v.expanded.unwrap();
        assert_eq!(card.headline, Headline::Clock { clock: Clock { value_ms: 60_000, at_unix_ms: 120_000, running: true, down: false }, tenths: false });
        assert!(card.rows.is_empty());
    }

    #[test]
    fn finished_says_how_it_went() {
        let done = view(&Entry { phase: Phase::Done, finished: Some(181_000), ..entry() }, 200_000, 0).expanded.unwrap();
        assert_eq!((done.headline, done.detail.as_deref()), (Headline::Text("Done".into()), Some("Took 3 min")));
        let failed = view(&Entry { phase: Phase::Failed, detail: Some("exit 101".into()), finished: Some(2_000), ..entry() }, 2_000, 0);
        assert_eq!(failed.compact.trailing, Element::Text { text: "Failed".into(), tone: Tone::Negative });
        assert_eq!(failed.expanded.unwrap().detail.as_deref(), Some("exit 101"));
        assert_eq!(length_text(3_900_000), "1 h 5 min");
        assert_eq!(short("A very long build title"), "A very long b…");
    }
}
