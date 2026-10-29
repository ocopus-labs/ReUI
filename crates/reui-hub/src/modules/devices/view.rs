//! The Devices moments and the overview as views (PRD 5.1). Pure. Colour comes from the
//! activity accent only: red when a battery is at 20 % or below, green while charging above
//! it; otherwise the island's default.

use reui_protocol::{Art, Button, ButtonContent, ButtonRole, CardSize, CompactView, Element, ExpandedView, Headline, Rgb, Row, Tone};

use super::airpods::Pods;
use super::drives::{self, Drive};
use super::kinds::{Kind, GLYPH_BOLT, GLYPH_DEVICES, GLYPH_DRIVE, GLYPH_SD_CARD};
use crate::hub::View;

/// Apple's dark system green and red (the island's `color.system` tokens).
pub const GREEN: Rgb = Rgb(48, 209, 88);
pub const RED: Rgb = Rgb(255, 69, 58);
const LOW: u8 = 20;

fn art(glyph: char) -> Element {
    Element::Art(Art::Glyph(glyph))
}

fn ring(level: u8) -> Element {
    Element::Ring(f32::from(level.min(100)) / 100.0)
}

fn percent(level: u8) -> String {
    format!("{}%", level.min(100))
}

fn low_accent(level: Option<u8>) -> Option<Rgb> {
    level.is_some_and(|l| l <= LOW).then_some(RED)
}

fn expanded(leading: Element, eyebrow: Option<&str>, headline: Headline, detail: Option<String>, trailing: Element, rows: Vec<Row>) -> ExpandedView {
    ExpandedView { leading, eyebrow: eyebrow.map(str::to_owned), headline, detail, trailing, rows, context: None, size: CardSize::Regular, tint: false, pager: None, open: None }
}

/// What a connect card or overview row knows about a device.
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceInfo {
    pub name: String,
    pub kind: Kind,
    /// The PnP level (or the buds' lowest).
    pub level: Option<u8>,
    pub pods: Option<Pods>,
}

/// Left, right and case as progress rows ("Left · 80%"), the parts that are known.
fn pod_rows(pods: &Pods) -> Vec<Row> {
    [("Left", pods.left, pods.left_charging), ("Right", pods.right, pods.right_charging), ("Case", pods.case, pods.case_charging)]
        .into_iter()
        .filter_map(|(label, level, charging)| {
            let level = level?;
            let trailing = if charging { format!("Charging · {}", percent(level)) } else { percent(level) };
            Some(Row::Progress { fraction: f32::from(level) / 100.0, leading: Some(label.into()), trailing: Some(trailing) })
        })
        .collect()
}

/// S1: glyph, and the battery ring once it's known (never a placeholder). Open: the name,
/// and the buds and case when read.
pub fn connect_view(d: &DeviceInfo) -> View {
    let glyph = art(d.kind.glyph());
    let level = d.level.map_or(Element::Empty, ring);
    let rows = d.pods.as_ref().map(pod_rows).unwrap_or_default();
    let detail = if rows.is_empty() { d.level.map(percent) } else { None };
    View {
        accent: low_accent(d.level),
        compact: CompactView { leading: glyph.clone(), trailing: level.clone() },
        minimal: if d.level.is_some() { level.clone() } else { glyph.clone() },
        expanded: Some(expanded(glyph, Some("Connected"), Headline::Text(d.name.clone()), detail, if rows.is_empty() { level } else { Element::Empty }, rows)),
        audio: None,
    }
}

/// Several devices at once: the first one's glyph and how many. Clicking opens the overview.
pub fn merged_view(kinds: &[Kind]) -> View {
    let glyph = art(kinds.first().copied().unwrap_or_default().glyph());
    View {
        accent: None,
        compact: CompactView { leading: glyph.clone(), trailing: Element::Text { text: format!("{} devices", kinds.len()), tone: Tone::Secondary } },
        minimal: glyph,
        expanded: None,
        audio: None,
    }
}

/// S2/S4: the device's glyph and its level in a red ring. Clicking opens the overview.
pub fn low_view(glyph: char, level: u8) -> View {
    View { accent: Some(RED), compact: CompactView { leading: art(glyph), trailing: ring(level) }, minimal: ring(level), expanded: None, audio: None }
}

/// S3: bolt and level; open: the level and, once stable, the time to full.
pub fn charging_view(level: u8, full_in: Option<String>) -> View {
    let accent = if level <= LOW { RED } else { GREEN };
    View {
        accent: Some(accent),
        compact: CompactView { leading: Element::Glyph { glyph: GLYPH_BOLT, tone: Tone::Accent }, trailing: Element::Text { text: percent(level), tone: Tone::Primary } },
        minimal: ring(level),
        expanded: Some(expanded(
            art(GLYPH_BOLT),
            Some("Charging"),
            Headline::Value(percent(level)),
            full_in,
            Element::Empty,
            vec![Row::Progress { fraction: f32::from(level) / 100.0, leading: None, trailing: None }],
        )),
        audio: None,
    }
}

/// A quiet one-line moment (phones, UPS, slow charger).
pub fn state_view(glyph: char, text: &str) -> View {
    View {
        accent: None,
        compact: CompactView { leading: art(glyph), trailing: Element::Text { text: text.to_owned(), tone: Tone::Secondary } },
        minimal: art(glyph),
        expanded: None,
        audio: None,
    }
}

fn drive_glyph(drive: &Drive) -> char {
    if drive.card {
        GLYPH_SD_CARD
    } else {
        GLYPH_DRIVE
    }
}

fn eject(drive: &Drive, label: &str) -> Button {
    Button { action: format!("eject:{}", drive.letter), content: ButtonContent::Text(label.into()), role: ButtonRole::Accent }
}

/// One drive's lines in a list: "BACKUP (E:) · 12.4 GB free of 64 GB", Eject, and why it
/// couldn't eject.
fn drive_rows(drive: &Drive, refused: Option<&String>) -> Vec<Row> {
    let mut rows = vec![Row::Note(format!("{} ({}:) · {}", drives::name(drive), drive.letter, drives::space(drive))), Row::Buttons(vec![eject(drive, "Eject")])];
    rows.extend(refused.map(|r| Row::Note(r.clone())));
    rows
}

/// S5/S6: drive glyph and name; open: space, Eject, and why it's blocked. Several drives
/// at once share one card.
pub fn drive_view(list: &[(Drive, Option<String>)]) -> Option<View> {
    let (first, refused) = list.first()?;
    let glyph = art(drive_glyph(first));
    let (compact_text, headline, detail, rows) = if list.len() == 1 {
        let mut rows = vec![Row::Buttons(vec![eject(first, "Eject")])];
        rows.extend(refused.iter().map(|r| Row::Note(r.clone())));
        (drives::name(first), drives::name(first), Some(drives::space(first)), rows)
    } else {
        let rows = list.iter().flat_map(|(d, r)| drive_rows(d, r.as_ref())).collect();
        (format!("{} drives", list.len()), format!("{} drives", list.len()), None, rows)
    };
    let eyebrow = (list.len() == 1).then(|| format!("{}:", first.letter));
    Some(View {
        accent: None,
        compact: CompactView { leading: glyph.clone(), trailing: Element::Text { text: compact_text, tone: Tone::Secondary } },
        minimal: glyph.clone(),
        expanded: Some(expanded(glyph, eyebrow.as_deref(), Headline::Text(headline), detail, Element::Empty, rows)),
        audio: None,
    })
}

/// The laptop's battery, for the overview.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Laptop {
    pub level: u8,
    pub charging: bool,
}

/// S9: battery devices, lowest first (buds and case under their device), the laptop, then
/// drives with Eject.
pub fn overview_view(devices: &[DeviceInfo], laptop: Option<Laptop>, drive_list: &[(Drive, Option<String>)]) -> View {
    let mut sorted: Vec<&DeviceInfo> = devices.iter().filter(|d| d.level.is_some()).collect();
    sorted.sort_by_key(|d| d.level);
    let lowest = sorted.first().and_then(|d| d.level).into_iter().chain(laptop.filter(|l| !l.charging).map(|l| l.level)).min();
    let mut rows = Vec::new();
    for d in &sorted {
        match d.pods.as_ref().map(pod_rows).filter(|r| !r.is_empty()) {
            Some(parts) => {
                rows.push(Row::Note(d.name.clone()));
                rows.extend(parts);
            }
            None => rows.push(Row::Progress { fraction: f32::from(d.level.unwrap_or(0)) / 100.0, leading: Some(d.name.clone()), trailing: Some(percent(d.level.unwrap_or(0))) }),
        }
    }
    if let Some(l) = laptop {
        let trailing = if l.charging { format!("Charging · {}", percent(l.level)) } else { percent(l.level) };
        rows.push(Row::Progress { fraction: f32::from(l.level) / 100.0, leading: Some("Battery".into()), trailing: Some(trailing) });
    }
    rows.extend(drive_list.iter().flat_map(|(d, r)| drive_rows(d, r.as_ref())));
    if rows.is_empty() {
        rows.push(Row::Note("No devices".into()));
    }
    let glyph = art(GLYPH_DEVICES);
    View {
        accent: low_accent(lowest),
        compact: CompactView { leading: glyph.clone(), trailing: lowest.map_or(Element::Empty, ring) },
        minimal: lowest.map_or_else(|| glyph.clone(), ring),
        expanded: Some(expanded(glyph, None, Headline::Text("Devices".into()), None, Element::Empty, rows)),
        audio: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(name: &str, kind: Kind, level: Option<u8>) -> DeviceInfo {
        DeviceInfo { name: name.into(), kind, level, pods: None }
    }

    fn pods() -> Pods {
        Pods { model: 0x0E20, left: Some(80), right: Some(70), case: None, left_charging: false, right_charging: false, case_charging: false }
    }

    #[test]
    fn devices_connect_card_upgrades_in_place() {
        let unknown = connect_view(&device("MX Master", Kind::Mouse, None));
        assert_eq!(unknown.compact.trailing, Element::Empty);
        assert_eq!(unknown.minimal, art(Kind::Mouse.glyph()));
        let known = connect_view(&device("MX Master", Kind::Mouse, Some(15)));
        assert_eq!(known.compact.trailing, Element::Ring(0.15));
        assert_eq!(known.accent, Some(RED));
        assert_eq!(known.expanded.as_ref().unwrap().detail.as_deref(), Some("15%"));

        let buds = connect_view(&DeviceInfo { pods: Some(pods()), ..device("AirPods Pro", Kind::Earbuds, Some(70)) });
        let rows = &buds.expanded.as_ref().unwrap().rows;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], Row::Progress { fraction: 0.8, leading: Some("Left".into()), trailing: Some("80%".into()) });
        assert_eq!(buds.accent, None);
    }

    #[test]
    fn devices_overview_sorts_lowest_first() {
        let devices = [device("Keyboard", Kind::Keyboard, Some(60)), device("Speaker", Kind::Speaker, None), device("Mouse", Kind::Mouse, Some(12))];
        let drive = Drive { letter: 'E', label: "BACKUP".into(), free: 12_400_000_000, total: 64_000_000_000, card: false };
        let v = overview_view(&devices, Some(Laptop { level: 55, charging: true }), &[(drive, Some("In use by Explorer".into()))]);
        let rows = &v.expanded.as_ref().unwrap().rows;
        let labels: Vec<String> = rows
            .iter()
            .map(|r| match r {
                Row::Progress { leading, trailing, .. } => format!("{} {}", leading.clone().unwrap_or_default(), trailing.clone().unwrap_or_default()),
                Row::Note(n) => n.clone(),
                Row::Buttons(b) => b[0].action.clone(),
                _ => String::new(),
            })
            .collect();
        assert_eq!(labels, ["Mouse 12%", "Keyboard 60%", "Battery Charging · 55%", "BACKUP (E:) · 12.4 GB free of 64 GB", "eject:E", "In use by Explorer"]);
        assert_eq!(v.compact.trailing, Element::Ring(0.12));
        assert_eq!(v.accent, Some(RED));
        let empty = overview_view(&[], None, &[]);
        assert_eq!(empty.expanded.unwrap().rows, [Row::Note("No devices".into())]);
    }

    #[test]
    fn devices_power_and_drive_views() {
        assert_eq!(charging_view(80, None).accent, Some(GREEN));
        assert_eq!(charging_view(20, None).accent, Some(RED));
        let c = charging_view(64, Some("Full in 1 h".into()));
        let e = c.expanded.unwrap();
        assert_eq!((e.headline, e.detail.as_deref()), (Headline::Value("64%".into()), Some("Full in 1 h")));
        assert_eq!(low_view(Kind::Mouse.glyph(), 10).minimal, Element::Ring(0.1));
        let d = Drive { letter: 'F', label: String::new(), free: 1, total: 2, card: true };
        let one = drive_view(&[(d.clone(), None)]).unwrap();
        assert_eq!(one.compact.leading, art(GLYPH_SD_CARD));
        assert_eq!(one.expanded.as_ref().unwrap().eyebrow.as_deref(), Some("F:"));
        let two = drive_view(&[(d.clone(), None), (Drive { letter: 'G', ..d }, None)]).unwrap();
        assert_eq!(two.compact.trailing, Element::Text { text: "2 drives".into(), tone: Tone::Secondary });
        assert!(drive_view(&[]).is_none());
        assert_eq!(merged_view(&[Kind::Mouse, Kind::Keyboard, Kind::Earbuds]).compact.trailing, Element::Text { text: "3 devices".into(), tone: Tone::Secondary });
    }
}
