//! System stats: the open card (`reui-hub/src/modules/stats/view.rs`): "System" with Task
//! Manager at its end, CPU and Memory as gauges with their last minute, then GPU, Disk and
//! Network as one line of facts. A desktop, so the CPU has no detail line (no battery).

use reui_design::layout::gauges;
use reui_design::look::Look;
use reui_design::tokens::{component, icon, opacity, typography};
use reui_design::Color;
use windows::core::Result;

use super::parts::{glyph, line, ring, Card};
use super::rgb;
use crate::render::{Align, Painter};
use crate::ui::layout::{Font, IslandFont, Rect};
use crate::ui::stage;
use crate::ui::Moment;

/// The module's teal.
pub const ACCENT: Color = rgb(64, 200, 224);
const CPU: char = '\u{E950}';
const MEMORY: char = '\u{E964}';
const TASK_MANAGER: char = '\u{E9D9}';
/// The history's length, `reui_protocol::GAUGE_HISTORY` (Settings doesn't see the protocol).
const MINUTE: usize = 60;

struct Sample {
    label: &'static str,
    glyph: char,
    value: f32,
    detail: &'static str,
    history: fn(usize) -> f32,
}

/// A quiet desktop: CPU idling around a tenth with a few short bursts (a build, a page
/// loading), memory steady a little under half.
fn cpu(i: usize) -> f32 {
    let t = i as f32;
    let burst = |at: f32, width: f32, height: f32| height * (-((t - at) / width).powi(2)).exp();
    0.09 + 0.03 * (t * 0.7).sin() * (t * 0.23).cos() + burst(14.0, 2.5, 0.38) + burst(37.0, 4.0, 0.22) + burst(55.0, 1.5, 0.12)
}

fn memory(i: usize) -> f32 {
    0.42 + 0.012 * (i as f32 / 12.0).sin() + if i > 36 { 0.009 } else { 0.0 }
}

pub fn draw(p: &Painter, look: &Look, e: f32) -> Result<()> {
    let open = stage::card(Moment::Stats, look).expect("an open card");
    let c = Card::new(p, look, ACCENT, open.card);
    let (width, pal) = (c.width, &look.palette);
    c.shape(open.height)?;
    let inset = look.inset();

    // The header: the eyebrow, and Task Manager bare on its line.
    let mid = open.header.mid;
    line(p, "System", IslandFont::Eyebrow, inset, open.header.text_top, width - 2.0 * inset, Align::Start, ACCENT)?;
    let g = icon::HEADER;
    glyph(p, TASK_MANAGER, g, width - inset - g, mid - g / 2.0, pal.label_secondary)?;

    let samples = [
        Sample { label: "CPU", glyph: CPU, value: cpu(MINUTE - 1), detail: "", history: cpu },
        Sample { label: "Memory", glyph: MEMORY, value: memory(MINUTE - 1), detail: "13.7 of 31.9 GB", history: memory },
    ];
    let (lefts, col_w) = gauges::columns(look, width, samples.len());
    let geo = gauges::gauge(look, col_w, true);
    let y = open.rows;
    for (s, x) in samples.iter().zip(lefts) {
        let (cx, cy) = (x + geo.ring.0, y + geo.ring.1);
        ring(p, look, cx, cy, component::ring::EXPANDED, component::ring::STROKE_EXPANDED, s.value * e, ACCENT)?;
        let gs = component::gauge::GLYPH;
        glyph(p, s.glyph, gs, cx - gs / 2.0, cy - gs / 2.0, pal.label_secondary)?;
        let (tx, tw) = (x + geo.text_left, geo.text_w);
        line(p, s.label, IslandFont::GaugeLabel, tx, y + geo.label, tw, Align::Start, pal.label_secondary)?;
        let value = format!("{}", (s.value * 100.0).round() as u32);
        let r = Rect::at(tx, y + geo.value, tw, typography::VALUE.line_height);
        p.figures_with_unit(&value, "%", Font::Island(IslandFont::Value), typography::GAUGE_UNIT, r, pal.label_primary, pal.label_secondary)?;
        if !s.detail.is_empty() {
            line(p, s.detail, IslandFont::Detail, tx, y + geo.detail, tw, Align::Start, pal.label_secondary)?;
        }
        let history: Vec<f32> = (0..MINUTE).map(|i| (s.history)(i) * e).collect();
        sparkline(p, look, &history, x, x + col_w, y + geo.spark)?;
    }

    // GPU, Disk and Network, spread from inset to inset.
    let facts = [("GPU", "4% · 826 MB"), ("Disk", "1%"), ("Network", "↓ 1.8 MB/s  ↑ 120 KB/s")];
    let top = y + geo.height + reui_design::layout::row_gap(look, 1, false);
    let label = Font::Island(IslandFont::FactLabel);
    let value = Font::Island(IslandFont::Fact);
    let widths: Vec<f32> = facts.iter().map(|(l, v)| p.measure(l, label).max(p.measure(v, value))).collect();
    for ((l, v), (x, w)) in facts.iter().zip(gauges::facts(look, width, &widths)) {
        line(p, l, IslandFont::FactLabel, x, top, w, Align::Start, pal.label_tertiary)?;
        line(p, v, IslandFont::Fact, x, top + gauges::fact_value_top(), w, Align::Start, pal.label_primary)?;
    }
    Ok(())
}

/// As the island draws it: the zero line, the fading area, the line, the newest point.
fn sparkline(p: &Painter, look: &Look, history: &[f32], left: f32, right: f32, top: f32) -> Result<()> {
    let zero = gauges::spark_zero(top);
    p.segment((left, zero), (right, zero), component::gauge::SPARK_TRACK, look.palette.fill_track)?;
    let points = gauges::spark_points(history, MINUTE, left, right, top);
    p.area_fading(&points, gauges::spark_fill_top(&points, zero), zero, ACCENT.with_alpha(opacity::SPARK_FILL))?;
    p.polyline(&points, component::gauge::SPARK_STROKE, ACCENT)?;
    if let Some(&(x, y)) = points.last() {
        p.circle(x, y, component::gauge::SPARK_DOT / 2.0, ACCENT)?;
    }
    Ok(())
}
