//! How timers look: the launcher (a field that previews what it will start), the running
//! timers (one activity, soonest in front, the rest a page away), the finish card, a focus
//! session and its phase ends, and the stopwatch.

use reui_protocol::{Button, ButtonContent, ButtonRole, CardSize, Clock, CompactView, Element, ExpandedView, Headline, Pager, Rgb, Row, Tone};

use super::focus::{Focus, Phase};
use super::model::{Finished, Timer, Timers};
use super::stopwatch::Stopwatch;
use super::parse::Parsed;
use crate::hub::View;

pub const ACCENT: Rgb = Rgb(255, 159, 10);
pub const FOCUS_ACCENT: Rgb = Rgb(94, 92, 230);
pub const BREAK_ACCENT: Rgb = Rgb(48, 209, 88);
pub const STOPWATCH_ACCENT: Rgb = Rgb(48, 209, 88);

const GLYPH_PLAY: char = '\u{E768}';
const GLYPH_PAUSE: char = '\u{E769}';
const GLYPH_CANCEL: char = '\u{E711}';
const GLYPH_BELL: char = '\u{EA8F}';
const GLYPH_TIMER: char = '\u{E916}';

pub const ACTION_PREV: &str = "prev";
pub const ACTION_NEXT: &str = "next";
pub const ACTION_STOP: &str = "stop";
pub const ACTION_MORE: &str = "more";
pub const ACTION_TOGGLE: &str = "toggle";
pub const ACTION_END: &str = "end";
pub const ACTION_SKIP: &str = "skip";
pub const ACTION_NEXT_PHASE: &str = "start";
pub const ACTION_LAP: &str = "lap";
pub const ACTION_RESET: &str = "reset";

const GLYPH_SKIP: char = '\u{E893}';
const GLYPH_LAP: char = '\u{E7C1}';
const GLYPH_RESET: char = '\u{E72C}';

/// The launcher: the field, a live reading of what Enter will start, and the presets.
/// `hint` replaces the detail line (a refusal: "10 timers is the limit").
pub fn launcher(query: &str, parsed: &Parsed, placeholder: &str, presets: &[u32], hint: Option<&str>) -> View {
    let (eyebrow, value, detail) = match parsed {
        Parsed::Timer { ms, label } => ("Timer", clock_text(*ms), label.clone()),
        Parsed::Focus => ("Focus", "Focus".to_string(), None),
        Parsed::Stopwatch => ("Stopwatch", "0:00".to_string(), None),
        Parsed::TooLong => ("Timer", "0:00".to_string(), Some("Up to 24 hours".to_string())),
        Parsed::Unknown => ("Timer", "0:00".to_string(), Some("Type a length, like 10m".to_string())),
        Parsed::Empty => ("Timer", "0:00".to_string(), None),
    };
    let tile = Element::Art(reui_protocol::Art::Glyph(GLYPH_TIMER));
    View {
        accent: Some(ACCENT),
        audio: None,
        compact: CompactView { leading: Element::Glyph { glyph: GLYPH_TIMER, tone: Tone::Accent }, trailing: Element::Empty },
        minimal: Element::Glyph { glyph: GLYPH_TIMER, tone: Tone::Accent },
        expanded: Some(ExpandedView {
            leading: tile,
            eyebrow: Some(eyebrow.into()),
            headline: Headline::Value(value),
            detail: hint.map(str::to_string).or(detail),
            trailing: Element::Empty,
            rows: vec![
                Row::Search { query: query.into(), buttons: vec![], placeholder: Some(placeholder.into()), glyph: Some(GLYPH_TIMER) },
                Row::Buttons(presets.iter().map(|m| text(&format!("preset:{m}"), &format!("{m} min"), ButtonRole::Secondary)).collect()),
            ],
            context: None, size: CardSize::Regular,
            tint: false,
            pager: None,
            open: None,
        }),
    }
}

/// `m:ss` below an hour, `h:mm:ss` from there.
pub fn clock_text(ms: u64) -> String {
    let secs = ms.div_ceil(1000);
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

/// The running timers' card, showing `page` (a timer id) or the soonest. `None` when there
/// are none.
pub fn running(timers: &Timers, page: Option<u32>, now: u64, now_unix: u64) -> Option<View> {
    let ordered = timers.ordered(now);
    let soonest = *ordered.first()?;
    let index = page.and_then(|id| ordered.iter().position(|t| t.id == id)).unwrap_or(0);
    let shown = ordered[index];
    let ring = |t: &Timer| Element::ClockRing { clock: clock(t, now, now_unix), total_ms: t.length_ms };
    let detail = if shown.running() { format!("Ends {}", time_of_day(now_unix + shown.left(now))) } else { "Paused".into() };
    let pager = (ordered.len() > 1).then(|| Pager { index: index as u32, count: ordered.len() as u32, prev: ACTION_PREV.into(), next: ACTION_NEXT.into() });
    Some(View {
        accent: Some(ACCENT),
        audio: None,
        compact: CompactView { leading: ring(soonest), trailing: Element::Clock(clock(soonest, now, now_unix)) },
        minimal: ring(soonest),
        expanded: Some(ExpandedView {
            leading: ring(shown),
            eyebrow: Some(shown.label.clone().unwrap_or_else(|| "Timer".into())),
            headline: Headline::Clock { clock: clock(shown, now, now_unix), tenths: false },
            detail: Some(detail),
            trailing: Element::Empty,
            rows: vec![Row::Buttons(vec![
                glyph(&format!("toggle:{}", shown.id), if shown.running() { GLYPH_PAUSE } else { GLYPH_PLAY }, ButtonRole::Accent),
                glyph(&format!("cancel:{}", shown.id), GLYPH_CANCEL, ButtonRole::Secondary),
            ])],
            context: None, size: CardSize::Regular,
            tint: false,
            pager,
            open: None,
        }),
    })
}

/// The finish card for one or more timers that reached zero. `late_at`: they ended a while
/// ago (sleep, ReUI closed), at that time of day.
pub fn finished(done: &[Finished], late_at: Option<u64>) -> View {
    let bell = Element::Glyph { glyph: GLYPH_BELL, tone: Tone::Accent };
    let (eyebrow, headline, detail) = match done {
        [one] => (
            one.label.clone().unwrap_or_else(|| "Timer".into()),
            late_at.map_or_else(|| "Done".into(), |at| format!("Finished at {}", time_of_day(at))),
            format!("{} timer", length_text(one.length_ms)),
        ),
        many => (
            "Timers".into(),
            format!("{} timers done", many.len()),
            many.iter().map(|d| d.label.clone().unwrap_or_else(|| length_text(d.length_ms))).collect::<Vec<_>>().join(", "),
        ),
    };
    View {
        accent: Some(ACCENT),
        audio: None,
        compact: CompactView { leading: bell.clone(), trailing: Element::Text { text: "Done".into(), tone: Tone::Accent } },
        minimal: bell,
        expanded: Some(ExpandedView {
            leading: Element::Art(reui_protocol::Art::Glyph(GLYPH_BELL)),
            eyebrow: Some(eyebrow),
            headline: Headline::Text(headline),
            detail: Some(detail),
            trailing: Element::Empty,
            rows: vec![Row::Buttons(vec![text(ACTION_MORE, "+1 min", ButtonRole::Secondary), text(ACTION_STOP, "Stop", ButtonRole::Primary)])],
            context: None, size: CardSize::Regular,
            tint: false,
            pager: None,
            open: None,
        }),
    }
}

/// A running (or paused) focus phase. `dnd` is the Do Not Disturb line, if any.
pub fn focus(f: &Focus, now: u64, now_unix: u64, dnd: Option<&str>) -> View {
    let clock = Clock { value_ms: f.left(now), at_unix_ms: now_unix, running: f.running(), down: true };
    let ring = Element::ClockRing { clock, total_ms: f.length_ms };
    let on_break = f.phase.is_break();
    let round = format!("Round {} of {}", f.round(), f.plan.rounds);
    let detail = match (f.running(), dnd) {
        (false, _) => "Paused".to_string(),
        (true, Some(dnd)) if !on_break => format!("{round} · {dnd}"),
        (true, _) => round,
    };
    let mut buttons = vec![glyph(ACTION_TOGGLE, if f.running() { GLYPH_PAUSE } else { GLYPH_PLAY }, ButtonRole::Accent)];
    if on_break {
        buttons.push(glyph(ACTION_SKIP, GLYPH_SKIP, ButtonRole::Secondary));
    }
    buttons.push(glyph(ACTION_END, GLYPH_CANCEL, ButtonRole::Secondary));
    View {
        accent: Some(if on_break { BREAK_ACCENT } else { FOCUS_ACCENT }),
        audio: None,
        compact: CompactView { leading: ring.clone(), trailing: Element::Clock(clock) },
        minimal: ring.clone(),
        expanded: Some(ExpandedView {
            leading: ring,
            eyebrow: Some(if on_break { "Break" } else { "Focus" }.into()),
            headline: Headline::Clock { clock, tenths: false },
            detail: Some(detail),
            trailing: Element::Empty,
            rows: vec![Row::Buttons(buttons)],
            context: None, size: CardSize::Regular,
            tint: false,
            pager: None,
            open: None,
        }),
    }
}

/// A focus phase ended and the next waits to be started.
pub fn phase_end(f: &Focus) -> View {
    let next = f.next_phase();
    let bell = Element::Glyph { glyph: GLYPH_BELL, tone: Tone::Accent };
    let (eyebrow, headline, detail, buttons) = match next {
        Phase::Work => (
            "Break",
            "Back to it",
            format!("Round {} of {}", f.round(), f.plan.rounds),
            vec![text(ACTION_END, "End", ButtonRole::Secondary), text(ACTION_NEXT_PHASE, "Start focus", ButtonRole::Primary)],
        ),
        Phase::ShortBreak | Phase::LongBreak => {
            let length = if next == Phase::LongBreak { f.plan.long_break_ms } else { f.plan.short_break_ms };
            (
                "Focus",
                "Break time",
                format!("{} break", length_text(length)),
                vec![text(ACTION_SKIP, "Skip break", ButtonRole::Secondary), text(ACTION_NEXT_PHASE, "Start break", ButtonRole::Primary)],
            )
        }
    };
    View {
        accent: Some(if next == Phase::Work { BREAK_ACCENT } else { FOCUS_ACCENT }),
        audio: None,
        compact: CompactView { leading: bell.clone(), trailing: Element::Text { text: headline.into(), tone: Tone::Accent } },
        minimal: bell,
        expanded: Some(ExpandedView {
            leading: Element::Art(reui_protocol::Art::Glyph(GLYPH_BELL)),
            eyebrow: Some(eyebrow.into()),
            headline: Headline::Text(headline.into()),
            detail: Some(detail),
            trailing: Element::Empty,
            rows: vec![Row::Buttons(buttons)],
            context: None, size: CardSize::Regular,
            tint: false,
            pager: None,
            open: None,
        }),
    }
}

pub fn stopwatch(s: &Stopwatch, now: u64, now_unix: u64) -> View {
    let clock = Clock { value_ms: s.elapsed(now), at_unix_ms: now_unix, running: s.running(), down: false };
    let mark = Element::Glyph { glyph: GLYPH_TIMER, tone: if s.running() { Tone::Accent } else { Tone::Secondary } };
    let detail = match s.laps.last() {
        Some(lap) => format!("Lap {} · {}", s.laps.len(), tenths_text(*lap)),
        None if !s.running() => "Paused".into(),
        None => String::new(),
    };
    View {
        accent: Some(STOPWATCH_ACCENT),
        audio: None,
        compact: CompactView { leading: mark.clone(), trailing: Element::Clock(clock) },
        minimal: mark,
        expanded: Some(ExpandedView {
            leading: Element::Art(reui_protocol::Art::Glyph(GLYPH_TIMER)),
            eyebrow: Some("Stopwatch".into()),
            headline: Headline::Clock { clock, tenths: true },
            detail: (!detail.is_empty()).then_some(detail),
            trailing: Element::Empty,
            rows: vec![Row::Buttons(vec![
                glyph(ACTION_TOGGLE, if s.running() { GLYPH_PAUSE } else { GLYPH_PLAY }, ButtonRole::Accent),
                glyph(ACTION_LAP, GLYPH_LAP, ButtonRole::Secondary),
                glyph(ACTION_RESET, GLYPH_RESET, ButtonRole::Secondary),
            ])],
            context: None, size: CardSize::Regular,
            tint: false,
            pager: None,
            open: None,
        }),
    }
}

/// `m:ss.t` (a lap).
fn tenths_text(ms: u64) -> String {
    let tenths = ms / 100;
    let secs = tenths / 10;
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    let whole = if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") };
    format!("{whole}.{}", tenths % 10)
}

fn clock(t: &Timer, now: u64, now_unix: u64) -> Clock {
    Clock { value_ms: t.left(now), at_unix_ms: now_unix, running: t.running(), down: true }
}

fn glyph(action: &str, glyph: char, role: ButtonRole) -> Button {
    Button { action: action.into(), content: ButtonContent::Glyph(glyph), role }
}

fn text(action: &str, label: &str, role: ButtonRole) -> Button {
    Button { action: action.into(), content: ButtonContent::Text(label.into()), role }
}

/// "4 min", "1 h 30 min", "45 s", "1 min 30 s".
pub fn length_text(ms: u64) -> String {
    let secs = ms.div_ceil(1000);
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    let parts: Vec<String> = [(h, "h"), (m, "min"), (s, "s")].iter().filter(|(v, _)| *v > 0).map(|(v, u)| format!("{v} {u}")).collect();
    if parts.is_empty() { "0 s".into() } else { parts.join(" ") }
}

/// The time of day in the user's own format ("3:42 PM", "15:42").
pub fn time_of_day(unix_ms: u64) -> String {
    use chrono::{Datelike, TimeZone, Timelike};
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::SYSTEMTIME;
    use windows::Win32::Globalization::{GetTimeFormatEx, TIME_NOSECONDS};
    let Some(t) = chrono::Local.timestamp_millis_opt(unix_ms as i64).single() else { return String::new() };
    let st = SYSTEMTIME {
        wYear: t.year() as u16,
        wMonth: t.month() as u16,
        wDayOfWeek: t.weekday().num_days_from_sunday() as u16,
        wDay: t.day() as u16,
        wHour: t.hour() as u16,
        wMinute: t.minute() as u16,
        wSecond: 0,
        wMilliseconds: 0,
    };
    let mut buf = [0u16; 64];
    let n = unsafe { GetTimeFormatEx(PCWSTR::null(), TIME_NOSECONDS, Some(&st), PCWSTR::null(), Some(&mut buf)) };
    if n > 1 {
        String::from_utf16_lossy(&buf[..n as usize - 1])
    } else {
        t.format("%H:%M").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_launcher_previews_what_enter_starts() {
        let card = launcher("10m tea", &Parsed::Timer { ms: 600_000, label: Some("Tea".into()) }, "25m", &[5, 10], None).expanded.unwrap();
        assert_eq!(card.headline, Headline::Value("10:00".into()));
        assert_eq!(card.detail.as_deref(), Some("Tea"));
        assert!(matches!(&card.rows[1], Row::Buttons(b) if b.len() == 2));
        let refused = launcher("x", &Parsed::Unknown, "25m", &[], Some("10 timers is the limit")).expanded.unwrap();
        assert_eq!(refused.detail.as_deref(), Some("10 timers is the limit"));
        assert_eq!(clock_text(90 * 60_000), "1:30:00");
    }

    #[test]
    fn focus_reads_its_round_and_asks_before_the_break() {
        use super::super::focus::Plan;
        let plan = Plan { work_ms: 25 * 60_000, short_break_ms: 5 * 60_000, long_break_ms: 15 * 60_000, rounds: 4 };
        let mut f = Focus::start(plan, 0);
        let card = focus(&f, 0, 0, Some("Do Not Disturb on")).expanded.unwrap();
        assert_eq!(card.detail.as_deref(), Some("Round 1 of 4 · Do Not Disturb on"));
        assert!(f.end_phase(25 * 60_000, false));
        let ask = phase_end(&f).expanded.unwrap();
        assert_eq!((ask.headline, ask.detail.as_deref()), (Headline::Text("Break time".into()), Some("5 min break")));
        assert_eq!(tenths_text(72_480), "1:12.4");
    }

    #[test]
    fn lengths_read_naturally() {
        assert_eq!(length_text(4 * 60_000), "4 min");
        assert_eq!(length_text(90 * 60_000), "1 h 30 min");
        assert_eq!(length_text(45_000), "45 s");
        assert_eq!(length_text(90_000), "1 min 30 s");
    }

    #[test]
    fn soonest_in_compact_and_pages_in_the_card() {
        let mut t = Timers::default();
        let bread = t.start(Some("Bread".into()), 40 * 60_000, 0).unwrap();
        let _pasta = t.start(Some("Pasta".into()), 9 * 60_000, 0).unwrap();
        let view = running(&t, Some(bread), 0, 0).unwrap();
        let Element::Clock(c) = view.compact.trailing else { panic!() };
        assert_eq!(c.value_ms, 9 * 60_000);
        let card = view.expanded.unwrap();
        assert_eq!(card.eyebrow.as_deref(), Some("Bread"));
        assert_eq!(card.pager.map(|p| (p.index, p.count)), Some((1, 2)));
        assert!(running(&Timers::default(), None, 0, 0).is_none());
    }
}
