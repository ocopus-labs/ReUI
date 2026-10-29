//! The stats card as a view. Pure.
//!
//! The hero is two gauges, CPU and memory: what "how busy is it" means first, each with
//! its last minute as a line (only what was read since the card opened). Under them one
//! quiet line of facts for the rest (GPU, disk, network), the ones turned on and known.
//! A part that isn't known (no counter, a desktop's battery) is left out, never shown as 0.
//! Colour carries meaning only: the accent everywhere, and a number that has stayed over
//! 90 % turns orange (`history.rs`).
//!
//! The header is the eyebrow alone, with Task Manager at its end: the gauges are the hero,
//! so there is no headline.

use reui_protocol::{Button, ButtonContent, ButtonRole, CardSize, CompactView, Element, ExpandedView, Fact, Gauge, Headline, Rgb, Row, Tone};

use super::history::{History, Series};
use super::maths::{bytes, bytes_per_second, memory, percent, whole_percent};
use super::sampler::{Reading, Wanted};
use crate::hub::View;

/// Apple's dark system teal (the island's `color.system` tokens).
pub const ACCENT: Rgb = Rgb(64, 200, 224);

/// The header button's action: open Task Manager.
pub const ACTION_TASK_MANAGER: &str = "taskmgr";

/// Segoe Fluent Icons: a processor, a memory module, and Diagnostic (a pulse on a screen,
/// Task Manager's own mark in Settings).
const CPU: char = '\u{E950}';
const MEMORY: char = '\u{E964}';
const TASK_MANAGER: char = '\u{E9D9}';

pub fn card(r: &Reading, show: Wanted, history: &History) -> View {
    let cpu = r.cpu.unwrap_or(0.0);
    let ring = Element::Ring(cpu);
    let compact_text = r.cpu.map_or_else(|| "–".to_string(), percent);
    let mut rows = vec![Row::Gauges(gauges(r, show, history))];
    let facts = facts(r, show, history);
    if !facts.is_empty() {
        rows.push(Row::Facts(facts));
    }
    View {
        accent: Some(ACCENT),
        audio: None,
        compact: CompactView { leading: ring.clone(), trailing: Element::Text { text: compact_text, tone: Tone::Primary } },
        minimal: ring,
        expanded: Some(ExpandedView {
            context: None,
            leading: Element::Empty,
            eyebrow: Some("System".into()),
            headline: Headline::Text(String::new()),
            detail: None,
            trailing: Element::Button(Button { action: ACTION_TASK_MANAGER.into(), content: ButtonContent::Glyph(TASK_MANAGER), role: ButtonRole::Plain }),
            rows,
            size: CardSize::Roomy,
            tint: false,
            pager: None,
            open: None,
        }),
    }
}

fn gauge(label: &str, glyph: char, fraction: Option<f32>, detail: Option<String>, series: &Series) -> Gauge {
    Gauge {
        label: label.into(),
        value: fraction.map_or_else(|| "–".into(), |f| whole_percent(f).to_string()),
        unit: if fraction.is_some() { "%".into() } else { String::new() },
        detail,
        fraction: fraction.unwrap_or(0.0).clamp(0.0, 1.0),
        glyph,
        tone: series.tone(),
        history: series.values(),
    }
}

/// CPU always (a dash until its first delta), memory when it's known.
fn gauges(r: &Reading, show: Wanted, history: &History) -> Vec<Gauge> {
    let mut gauges = vec![gauge("CPU", CPU, r.cpu, power(r.power_mw.filter(|_| show.power)), &history.cpu)];
    if let Some((used, total)) = r.memory.filter(|(_, total)| *total > 0) {
        gauges.push(gauge("Memory", MEMORY, Some(used as f32 / total as f32), Some(memory(used, total)), &history.memory));
    }
    gauges
}

/// The battery's flow under the CPU, when there is one: "On battery, 12 W". A desktop's
/// CPU has no line there; nothing else about it is worth a line at a glance.
fn power(power_mw: Option<i32>) -> Option<String> {
    let watts = |mw: i32| {
        let w = f64::from(mw.unsigned_abs()) / 1000.0;
        if w < 9.95 { format!("{w:.1} W") } else { format!("{w:.0} W") }
    };
    match power_mw {
        Some(mw) if mw < 0 => Some(format!("On battery, {}", watts(mw))),
        Some(mw) if mw > 0 => Some(format!("Charging, {}", watts(mw))),
        _ => None,
    }
}

fn facts(r: &Reading, show: Wanted, history: &History) -> Vec<Fact> {
    let mut facts = Vec::new();
    if let Some(gpu) = r.gpu.filter(|_| show.gpu) {
        let value = match r.gpu_memory.filter(|b| *b > 0) {
            Some(b) => format!("{} · {}", percent(gpu), bytes(b)),
            None => percent(gpu),
        };
        facts.push(Fact { label: "GPU".into(), value, tone: history.gpu.tone() });
    }
    if let Some(disk) = r.disk.filter(|_| show.disk) {
        facts.push(Fact { label: "Disk".into(), value: percent(disk), tone: history.disk.tone() });
    }
    if let Some((down, up, _)) = r.network.filter(|_| show.network) {
        let value = format!("↓ {}  ↑ {}", bytes_per_second(down), bytes_per_second(up));
        facts.push(Fact { label: "Network".into(), value, tone: Tone::Primary });
    }
    facts
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: Wanted = Wanted { gpu: true, disk: true, network: true, power: true };

    fn expanded(v: &View) -> &ExpandedView {
        v.expanded.as_ref().unwrap()
    }

    fn gauges_of(v: &View) -> &[Gauge] {
        match &expanded(v).rows[0] {
            Row::Gauges(g) => g,
            other => panic!("{other:?}"),
        }
    }

    fn facts_of(v: &View) -> Vec<String> {
        match expanded(v).rows.get(1) {
            Some(Row::Facts(f)) => f.iter().map(|f| format!("{} {}", f.label, f.value)).collect(),
            _ => Vec::new(),
        }
    }

    fn reading() -> Reading {
        Reading {
            cpu: Some(0.234),
            memory: Some((12_025_908_428, 34_037_615_820)),
            gpu: Some(0.41),
            gpu_memory: Some(2_100_000_000),
            disk: Some(0.03),
            network: Some((1_234_000.0, 80_000.0, 1_000_000_000)),
            power_mw: Some(-12_400),
            ..Reading::default()
        }
    }

    #[test]
    fn stats_card_is_two_gauges_and_a_line_of_facts() {
        let mut history = History::default();
        history.push(Some(0.1), Some(0.35), Some(0.41), Some(0.03));
        history.push(Some(0.234), Some(0.353), Some(0.41), Some(0.03));
        let v = card(&reading(), ALL, &history);
        let e = expanded(&v);
        assert_eq!((e.eyebrow.as_deref(), &e.headline, e.size), (Some("System"), &Headline::Text(String::new()), CardSize::Roomy));
        assert!(matches!(&e.trailing, Element::Button(b) if b.action == ACTION_TASK_MANAGER));
        let g = gauges_of(&v);
        assert_eq!(g.iter().map(|g| (g.label.as_str(), g.value.as_str(), g.unit.as_str(), g.detail.as_deref())).collect::<Vec<_>>(), [("CPU", "23", "%", Some("On battery, 12 W")), ("Memory", "35", "%", Some("11.2 of 31.7 GB"))]);
        assert_eq!(g[0].history, [0.1, 0.234], "the line is what was read since the card opened");
        assert_eq!(facts_of(&v), ["GPU 41% · 2.1 GB", "Disk 3%", "Network ↓ 1.2 MB/s  ↑ 80 KB/s"]);
        assert!(matches!(v.compact.trailing, Element::Text { ref text, .. } if text == "23%"));
    }

    #[test]
    fn stats_card_leaves_out_what_it_does_not_know() {
        // A desktop without GPU counters, the disk row turned off, the first second.
        let r = Reading { cpu: None, memory: reading().memory, network: Some((0.0, 0.0, u64::MAX)), ..Reading::default() };
        let v = card(&r, Wanted { disk: false, ..ALL }, &History::default());
        let g = gauges_of(&v);
        assert_eq!((g[0].value.as_str(), g[0].unit.as_str(), g[0].detail.as_deref()), ("–", "", None));
        assert!(g[0].history.is_empty());
        assert_eq!(facts_of(&v), ["Network ↓ 0 KB/s  ↑ 0 KB/s"]);
        // Nothing but CPU and memory: no line of facts at all.
        let r = Reading { cpu: Some(0.05), memory: reading().memory, ..Reading::default() };
        assert_eq!(expanded(&card(&r, ALL, &History::default())).rows.len(), 1);
    }

    #[test]
    fn stats_numbers_turn_orange_only_when_they_stay_high() {
        let mut history = History::default();
        for _ in 0..3 {
            history.push(Some(0.97), Some(0.4), Some(0.95), Some(0.2));
        }
        let r = Reading { cpu: Some(0.97), gpu: Some(0.95), disk: Some(0.2), ..reading() };
        let v = card(&r, ALL, &history);
        assert_eq!(gauges_of(&v).iter().map(|g| g.tone).collect::<Vec<_>>(), [Tone::Warning, Tone::Primary]);
        let Some(Row::Facts(f)) = expanded(&v).rows.get(1) else { unreachable!() };
        assert_eq!(f.iter().map(|f| f.tone).collect::<Vec<_>>(), [Tone::Warning, Tone::Primary, Tone::Primary]);
    }

    #[test]
    fn stats_power_reads_its_direction() {
        assert_eq!(power(Some(30_000)).as_deref(), Some("Charging, 30 W"));
        assert_eq!(power(Some(-8_260)).as_deref(), Some("On battery, 8.3 W"));
        assert_eq!(power(Some(0)), None);
        assert_eq!(power(None), None);
    }
}
