//! The stats card as a view: CPU as the hero (a ring and the share), then one meter per
//! thing the machine is busy with (GPU, memory, disk, network), each with one fact. Pure.
//! A part that isn't known (no counter, a desktop's battery) is left out, never shown as 0.

use reui_protocol::{CardSize, CompactView, Element, ExpandedView, Headline, Rgb, Row, Tone};

use super::maths::{bytes, bytes_per_second, memory, percent};
use super::sampler::{Reading, Wanted};
use crate::hub::View;

/// Apple's dark system teal (the island's `color.system` tokens).
pub const ACCENT: Rgb = Rgb(64, 200, 224);

/// A link speed beyond this is a driver's "unknown" (some report all ones), not a link.
const MAX_LINK_BPS: u64 = 1_000_000_000_000;

pub fn card(r: &Reading, show: Wanted) -> View {
    let cpu = r.cpu.unwrap_or(0.0);
    let hero = r.cpu.map_or_else(|| "–".to_string(), percent);
    let ring = Element::Ring(cpu);
    View {
        accent: Some(ACCENT),
        audio: None,
        compact: CompactView { leading: ring.clone(), trailing: Element::Text { text: hero.clone(), tone: Tone::Primary } },
        minimal: ring.clone(),
        expanded: Some(ExpandedView {
            context: None,
            leading: ring,
            eyebrow: Some("System".into()),
            headline: Headline::Value(hero),
            detail: Some(detail(r.power_mw.filter(|_| show.power))),
            trailing: Element::Empty,
            rows: rows(r, show),
            size: CardSize::Regular,
            tint: false,
            pager: None,
            open: None,
        }),
    }
}

/// What the hero is, and the battery's flow when there is one: "CPU · On battery, 12 W".
fn detail(power_mw: Option<i32>) -> String {
    let watts = |mw: i32| {
        let w = f64::from(mw.unsigned_abs()) / 1000.0;
        if w < 9.95 { format!("{w:.1} W") } else { format!("{w:.0} W") }
    };
    match power_mw {
        Some(mw) if mw < 0 => format!("CPU · On battery, {}", watts(mw)),
        Some(mw) if mw > 0 => format!("CPU · Charging, {}", watts(mw)),
        _ => "CPU".into(),
    }
}

fn rows(r: &Reading, show: Wanted) -> Vec<Row> {
    let meter = |fraction: f32, label: &str, fact: String| Row::Progress { fraction: fraction.clamp(0.0, 1.0), leading: Some(label.into()), trailing: Some(fact) };
    let mut rows = Vec::new();
    if let Some(gpu) = r.gpu.filter(|_| show.gpu) {
        let fact = match r.gpu_memory.filter(|b| *b > 0) {
            Some(b) => format!("{} · {}", percent(gpu), bytes(b)),
            None => percent(gpu),
        };
        rows.push(meter(gpu, "GPU", fact));
    }
    if let Some((used, total)) = r.memory.filter(|(_, total)| *total > 0) {
        rows.push(meter(used as f32 / total as f32, "Memory", memory(used, total)));
    }
    if let Some(disk) = r.disk.filter(|_| show.disk) {
        rows.push(meter(disk, "Disk", percent(disk)));
    }
    if let Some((down, up, link)) = r.network.filter(|_| show.network) {
        // How full the fastest link is, the busier direction.
        let full = if link > 0 && link < MAX_LINK_BPS { (down.max(up) * 8.0 / link as f64) as f32 } else { 0.0 };
        rows.push(meter(full, "Network", format!("↓ {}  ↑ {}", bytes_per_second(down), bytes_per_second(up))));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: Wanted = Wanted { gpu: true, disk: true, network: true, power: true };

    fn facts(v: &View) -> Vec<String> {
        v.expanded.as_ref().unwrap().rows.iter().map(|r| match r {
            Row::Progress { leading, trailing, .. } => format!("{} {}", leading.clone().unwrap_or_default(), trailing.clone().unwrap_or_default()),
            other => format!("{other:?}"),
        }).collect()
    }

    #[test]
    fn stats_card_shows_what_is_known() {
        let r = Reading {
            cpu: Some(0.234),
            memory: Some((12_025_908_428, 34_037_615_820)),
            gpu: Some(0.41),
            gpu_memory: Some(2_100_000_000),
            disk: Some(0.03),
            network: Some((1_234_000.0, 80_000.0, 1_000_000_000)),
            power_mw: Some(-12_400),
            ..Reading::default()
        };
        let v = card(&r, ALL);
        let e = v.expanded.as_ref().unwrap();
        assert_eq!((e.eyebrow.as_deref(), &e.headline, e.detail.as_deref()), (Some("System"), &Headline::Value("23%".into()), Some("CPU · On battery, 12 W")));
        assert_eq!(facts(&v), ["GPU 41% · 2.1 GB", "Memory 11.2 of 31.7 GB", "Disk 3%", "Network ↓ 1.2 MB/s  ↑ 80 KB/s"]);
        match &e.rows[3] {
            Row::Progress { fraction, .. } => assert!((fraction - 0.009_872).abs() < 1e-4, "{fraction}"),
            _ => unreachable!(),
        }

        // A desktop without GPU counters, the disk row turned off: those rows are left out.
        let r = Reading { cpu: Some(0.05), memory: r.memory, network: Some((0.0, 0.0, u64::MAX)), ..Reading::default() };
        let v = card(&r, Wanted { disk: false, ..ALL });
        assert_eq!(facts(&v), ["Memory 11.2 of 31.7 GB", "Network ↓ 0 KB/s  ↑ 0 KB/s"]);
        assert_eq!(v.expanded.unwrap().detail.as_deref(), Some("CPU"));
    }

    #[test]
    fn stats_power_reads_its_direction() {
        assert_eq!(detail(Some(30_000)), "CPU · Charging, 30 W");
        assert_eq!(detail(Some(-8_260)), "CPU · On battery, 8.3 W");
        assert_eq!(detail(Some(0)), "CPU");
        assert_eq!(detail(None), "CPU");
    }
}
