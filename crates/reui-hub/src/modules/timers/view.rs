//! How timers look: the running timers (one activity, soonest in front, the rest a page
//! away) and the finish card.

use reui_protocol::{Button, ButtonContent, ButtonRole, Clock, CompactView, Element, ExpandedView, Headline, Pager, Rgb, Row, Tone};

use super::model::{Finished, Timer, Timers};
use crate::hub::View;

pub const ACCENT: Rgb = Rgb(255, 159, 10);

const GLYPH_PLAY: char = '\u{E768}';
const GLYPH_PAUSE: char = '\u{E769}';
const GLYPH_CANCEL: char = '\u{E711}';
const GLYPH_BELL: char = '\u{EA8F}';

pub const ACTION_PREV: &str = "prev";
pub const ACTION_NEXT: &str = "next";
pub const ACTION_STOP: &str = "stop";
pub const ACTION_MORE: &str = "more";

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
            wide: false,
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
            wide: false,
            tint: false,
            pager: None,
            open: None,
        }),
    }
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
