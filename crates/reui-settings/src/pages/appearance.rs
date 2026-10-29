//! Appearance: the island's style, theme, accent and shape (`settings.json`; the island
//! watches the file and rebuilds itself with the new look). The header shows the island
//! drawn with the same `Look` the island uses.

use reui_design::look::{Accent, Look, Padding, Style, Theme};
use reui_design::tokens::{color::system as palette, look as range};
use serde_json::Value;

use super::{file_error, group, Effect, Page};
use crate::model::files::File;
use crate::state::State;
use crate::system;
use crate::ui::{icons, Act, Block, Part, Preview, Row, Target};

pub fn build(state: &State) -> Vec<Block> {
    let a = state.files.appearance();
    let look = Look::new(&a, system::apps_light(), system::accent_color());
    let mut blocks = vec![Block::Intro {
        eyebrow: None,
        title: Page::Appearance.title().into(),
        body: Some("How the island looks: floating or joined to the top edge, dark or light, and its shape.".into()),
        icon: Some(Page::Appearance.icon()),
        preview: Some(Preview::Appearance(look)),
        row: None,
    }];
    blocks.extend(file_error(state, File::Island));
    let ok = state.files.doc(File::Island).error().is_none();

    let style = Row::new("style", "Style")
        .detail(if a.style == Style::Notch { "Joined to the top edge, like a notch" } else { "Floating below the top edge" })
        .part(Part::Segmented { options: vec!["Island".into(), "Notch".into()], selected: a.style.index() })
        .enabled(ok)
        .icon(icons::tile(icons::VIEW, palette::BLUE));
    let theme = Row::new("theme", "Theme")
        .detail(match a.theme {
            Theme::Auto if look.light => "Light, as Windows' apps are now",
            Theme::Auto => "Dark, as Windows' apps are now",
            Theme::Dark => "Black, blending into the top of the screen",
            Theme::Light => "White, with a hairline edge",
        })
        .part(Part::Segmented { options: vec!["Auto".into(), "Dark".into(), "Light".into()], selected: a.theme.index() })
        .enabled(ok)
        .icon(icons::tile(icons::MOON, palette::INDIGO));
    let accent = Row::new("accent", "Accent")
        .detail("For what has no colour of its own")
        .part(Part::Segmented { options: vec!["ReUI blue".into(), "Windows".into()], selected: a.accent.index() })
        .enabled(ok)
        .icon(icons::tile(icons::PALETTE, look.accent));
    blocks.push(group("Look", vec![style, theme, accent]));

    let roundness = Row::new("roundness", "Roundness")
        .detail("From square-ish corners to a capsule")
        .part(Part::Slider { value: a.roundness, reading: format!("{} %", (a.roundness * 100.0).round()) })
        .enabled(ok);
    // A notch has no gap: it's joined to the edge.
    let gap = Row::new("top_gap", "Gap from the top")
        .detail(if a.style == Style::Notch { "None: the notch is joined to the edge" } else { "Between the screen's edge and the island" })
        .part(Part::Slider { value: a.top_gap / range::top_gap::MAX, reading: format!("{}", a.top_gap.round()) })
        .enabled(ok && a.style == Style::Island);
    let padding = Row::new("padding", "Padding")
        .detail("Space inside an open card")
        .part(Part::Segmented { options: vec!["Compact".into(), "Regular".into(), "Roomy".into()], selected: a.padding.index() })
        .enabled(ok);
    let size = Row::new("scale", "Size")
        .detail("The island and its text")
        .part(Part::Slider { value: size_value(a.size), reading: format!("{} %", (a.size * 100.0).round()) })
        .enabled(ok);
    blocks.push(group("Shape", vec![roundness, gap, padding, size]));
    blocks
}

/// The size (`look.size.min..=max`) as a slider position.
fn size_value(size: f32) -> f32 {
    ((size - range::size::MIN) / (range::size::MAX - range::size::MIN)).clamp(0.0, 1.0)
}

/// A slider position as a size, in steps of 5 %.
fn size_at(value: f32) -> f32 {
    let percent = (range::size::MIN + value.clamp(0.0, 1.0) * (range::size::MAX - range::size::MIN)) * 100.0;
    (percent / 5.0).round() * 5.0 / 100.0
}

/// A slider position as roundness, in whole percents.
fn roundness_at(value: f32) -> f32 {
    (value.clamp(0.0, 1.0) * 100.0).round() / 100.0
}

/// A slider position as the gap, in whole DIPs.
fn gap_at(value: f32) -> f32 {
    (value.clamp(0.0, 1.0) * range::top_gap::MAX).round()
}

pub fn act(state: &mut State, target: &Target, act: Act) -> Effect {
    let a = state.files.appearance();
    let name = |names: &[&str], i: usize| names.get(i).map(|n| Value::String((*n).into()));
    let number = |v: f32| Value::from(f64::from(v));
    let change = match (target.row.as_str(), act) {
        ("style", Act::Choose(i)) => name(Style::NAMES, i).map(|v| ("style", v)),
        ("theme", Act::Choose(i)) => name(Theme::NAMES, i).map(|v| ("theme", v)),
        ("accent", Act::Choose(i)) => name(Accent::NAMES, i).map(|v| ("accent", v)),
        ("padding", Act::Choose(i)) => name(Padding::NAMES, i).map(|v| ("padding", v)),
        // Sliders write only when the value they stand for moves, so a drag isn't a stream
        // of identical writes (each one makes the island rebuild).
        ("roundness", Act::Value(v)) => Some(roundness_at(v)).filter(|r| (r - a.roundness).abs() > 1e-4).map(|r| ("roundness", number(r))),
        ("top_gap", Act::Value(v)) => Some(gap_at(v)).filter(|g| (g - a.top_gap).abs() > 1e-4).map(|g| ("top_gap", number(g))),
        ("scale", Act::Value(v)) => Some(size_at(v)).filter(|s| (s - a.size).abs() > 1e-4).map(|s| ("scale", number(s))),
        _ => None,
    };
    if let Some((key, value)) = change {
        state.set(File::Island, key, value);
    }
    Effect::None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::tests::state;
    use reui_design::look::Appearance;

    #[test]
    fn choices_and_sliders_write_what_the_island_reads() {
        let (mut state, dir) = state("appearance");
        assert_eq!(state.files.appearance(), Appearance::default());
        act(&mut state, &Target::new("style", 0), Act::Choose(1));
        act(&mut state, &Target::new("theme", 0), Act::Choose(2));
        act(&mut state, &Target::new("padding", 0), Act::Choose(2));
        act(&mut state, &Target::new("accent", 0), Act::Choose(1));
        act(&mut state, &Target::new("roundness", 0), Act::Value(0.333));
        act(&mut state, &Target::new("scale", 0), Act::Value(1.0));
        act(&mut state, &Target::new("top_gap", 0), Act::Value(0.5));
        let a = state.files.appearance();
        assert_eq!((a.style, a.theme, a.padding, a.accent), (Style::Notch, Theme::Light, Padding::Roomy, Accent::Windows));
        assert_eq!((a.roundness, a.size, a.top_gap), (0.33, range::size::MAX, 12.0));
        // The island parses the same file the same way.
        let text = std::fs::read_to_string(dir.join("roaming").join("settings.json")).unwrap();
        let saved: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(saved["style"], "notch");
        assert_eq!(saved["visibility"], "always", "the island's other keys stay");
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sliders_move_in_whole_steps() {
        assert_eq!(size_at(0.0), range::size::MIN);
        assert_eq!(size_at(1.0), range::size::MAX);
        assert!((size_at(size_value(1.1)) - 1.1).abs() < 1e-6);
        assert_eq!(roundness_at(0.456), 0.46);
        assert_eq!(gap_at(11.0 / 24.0), 11.0);
    }

    #[test]
    fn the_page_builds_with_its_preview() {
        let (state, dir) = state("appearance-build");
        let blocks = build(&state);
        assert!(matches!(&blocks[0], Block::Intro { preview: Some(Preview::Appearance(_)), .. }));
        _ = std::fs::remove_dir_all(&dir);
    }
}
