//! What onboarding puts in the island: the first-run demo card, the "Hover me" hint, and
//! the one-time tips and offers. The only teaching text in the island, each shown once.

use reui_protocol::{Art, Button, ButtonContent, ButtonRole, CardSize, CompactView, Element, ExpandedView, Headline, ImageId, Rgb, Row, Tone};

use crate::hub::View;

const GLYPH_MUSIC: char = '\u{E8D6}';
const GLYPH_POINTER: char = '\u{E7C9}';
const GLYPH_PASTE: char = '\u{E77F}';
const GLYPH_VOLUME: char = '\u{E767}';
const GLYPH_LYRICS: char = '\u{E8D2}';
const GLYPH_CHECK: char = '\u{E73E}';
const GLYPH_ERROR: char = '\u{E783}';

/// ReUI's own accent (system blue) for the demo, the hint and tips about ReUI itself.
pub const ACCENT: Rgb = Rgb(10, 132, 255);
const MEDIA_ACCENT: Rgb = Rgb(255, 55, 95);
const CLIPBOARD_ACCENT: Rgb = Rgb(100, 210, 255);

/// Actions an offer's buttons send.
pub const ACCEPT: &str = "accept";
pub const DECLINE: &str = "decline";
/// The hotkey tip's button.
pub const OPEN_HISTORY: &str = "open-history";

/// How long the demo track pretends to be.
pub const DEMO_LENGTH_MS: u64 = 3 * 60_000 + 24_000;

fn text_button(action: &str, label: &str, role: ButtonRole) -> Button {
    Button { action: action.into(), content: ButtonContent::Text(label.into()), role }
}

fn card(leading: Element, eyebrow: &str, headline: Headline, detail: Option<String>, rows: Vec<Row>) -> ExpandedView {
    ExpandedView { leading, eyebrow: Some(eyebrow.into()), headline, detail, trailing: Element::Empty, rows, context: None, size: CardSize::Regular, tint: false, pager: None, open: None }
}

/// The first-run demo: a Now Playing-style card with a moving timeline. `open` keeps its
/// card; without it the island folds back to compact.
pub fn demo(open: bool, started_unix_ms: u64) -> View {
    let art = Element::Art(Art::Glyph(GLYPH_MUSIC));
    let timeline = Row::Timeline { position_ms: 41_000, length_ms: DEMO_LENGTH_MS, playing: true, at_unix_ms: started_unix_ms, seekable: false, rate: 1.0 };
    View {
        accent: Some(ACCENT),
        audio: None,
        compact: CompactView { leading: art.clone(), trailing: Element::Bars { live: true } },
        minimal: Element::Bars { live: true },
        expanded: open.then(|| ExpandedView {
            trailing: Element::Bars { live: true },
            ..card(art, "Now Playing", Headline::Text("Welcome to ReUI".into()), Some("Hover the island any time".into()), vec![timeline])
        }),
    }
}

/// The one-time "Hover me" hint inside the pill, until the first hover.
pub fn hint() -> View {
    let glyph = Element::Glyph { glyph: GLYPH_POINTER, tone: Tone::Accent };
    View {
        accent: Some(ACCENT),
        audio: None,
        compact: CompactView { leading: glyph.clone(), trailing: Element::Text { text: "Hover me".into(), tone: Tone::Primary } },
        minimal: glyph,
        expanded: None,
    }
}

/// The clipboard history hotkey, after a copy.
pub fn hotkey(label: &str) -> View {
    let tile = Element::Art(Art::Glyph(GLYPH_PASTE));
    View {
        accent: Some(CLIPBOARD_ACCENT),
        audio: None,
        compact: CompactView { leading: tile.clone(), trailing: Element::Text { text: label.into(), tone: Tone::Accent } },
        minimal: tile.clone(),
        expanded: Some(card(
            tile,
            "Clipboard",
            Headline::Text(label.into()),
            Some("Opens your clipboard history".into()),
            vec![Row::Buttons(vec![text_button(OPEN_HISTORY, "Open history", ButtonRole::Secondary)])],
        )),
    }
}

/// An offer: a question with two answers. `short` is the compact pill's question.
fn offer(accent: Rgb, tile: Element, eyebrow: &str, question: &str, detail: String, short: &str, (no, yes): (&str, &str)) -> View {
    View {
        accent: Some(accent),
        audio: None,
        compact: CompactView { leading: tile.clone(), trailing: Element::Text { text: short.into(), tone: Tone::Accent } },
        minimal: tile.clone(),
        expanded: Some(card(
            tile,
            eyebrow,
            Headline::Text(question.into()),
            Some(detail),
            vec![Row::Buttons(vec![text_button(DECLINE, no, ButtonRole::Secondary), text_button(ACCEPT, yes, ButtonRole::Primary)])],
        )),
    }
}

pub fn flyout() -> View {
    let tile = Element::Art(Art::Glyph(GLYPH_VOLUME));
    offer(ACCENT, tile, "Volume", "Replace Windows' pop-up?", "Hidden only while ReUI runs".into(), "Replace?", ("Not now", "Yes"))
}

pub fn lyrics() -> View {
    let tile = Element::Art(Art::Glyph(GLYPH_LYRICS));
    offer(MEDIA_ACCENT, tile, "Now Playing", "Show synced lyrics?", "Song titles are sent to lrclib.net".into(), "Lyrics?", ("No thanks", "Turn on"))
}

/// "Show Claude Code in the island?", naming the file Connect changes.
pub fn agent(display: &str, accent: Rgb, logo: Option<ImageId>, glyph: char, file: &str) -> View {
    let tile = Element::Art(logo.map_or(Art::Glyph(glyph), |image| Art::Logo { image, animated: false }));
    offer(accent, tile, display, &format!("Show {display} in the island?"), format!("Changes {file}"), "Connect?", ("Not now", "Connect"))
}

/// What came of Connect.
pub fn connected(display: &str, accent: Rgb, logo: Option<ImageId>, glyph: char, result: Result<(), String>) -> View {
    let tile = Element::Art(logo.map_or(Art::Glyph(glyph), |image| Art::Logo { image, animated: false }));
    let (mark, text, detail) = match result {
        Ok(()) => (Element::Glyph { glyph: GLYPH_CHECK, tone: Tone::Positive }, "Connected".to_string(), "Its next session shows here".to_string()),
        Err(e) => (Element::Glyph { glyph: GLYPH_ERROR, tone: Tone::Negative }, "Couldn't connect".to_string(), e),
    };
    View {
        accent: Some(accent),
        audio: None,
        compact: CompactView { leading: tile.clone(), trailing: mark.clone() },
        minimal: tile.clone(),
        expanded: Some(ExpandedView { trailing: mark, ..card(tile, display, Headline::Text(text), Some(detail), Vec::new()) }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_folds_by_dropping_its_card_and_offers_have_two_answers() {
        assert!(demo(true, 0).expanded.is_some());
        assert!(demo(false, 0).expanded.is_none());
        for view in [flyout(), lyrics(), agent("Codex", ACCENT, None, 'x', r"~\.codex\hooks.json")] {
            let rows = view.expanded.unwrap().rows;
            let Some(Row::Buttons(buttons)) = rows.first() else { panic!("an offer has buttons") };
            let actions: Vec<&str> = buttons.iter().map(|b| b.action.as_str()).collect();
            assert_eq!(actions, [DECLINE, ACCEPT]);
        }
    }
}
