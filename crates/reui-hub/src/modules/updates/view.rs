//! What the updates module puts in the island: the update card in each of its phases
//! (shaped after the iOS system alerts, PRD agent moments §5.8, without the glyph: the
//! version as the hero, the notes' first line wrapping under it, two equal capsules with
//! the main action tinted) and the "Updated" brief after an upgrade.

use reui_protocol::{Button, ButtonContent, ButtonRole, CardSize, CompactView, Element, ExpandedView, Headline, Rgb, Row, Tone};

use super::card::{Card, Phase};
use crate::hub::View;

/// ReUI's own accent (system blue), as for its other cards about itself.
const ACCENT: Rgb = Rgb(10, 132, 255);
const GLYPH_DOWNLOAD: char = '\u{E896}';
const GLYPH_NEW: char = '\u{E735}';
const GLYPH_ERROR: char = '\u{E783}';

/// Actions the card's buttons send.
pub const INSTALL: &str = "install";
pub const LATER: &str = "later";
/// The "Updated" brief's button.
pub const OPEN: &str = "open";

fn text_button(action: &str, label: &str, role: ButtonRole) -> Button {
    Button { action: action.into(), content: ButtonContent::Text(label.into()), role }
}

fn text(text: &str, tone: Tone) -> Element {
    Element::Text { text: text.into(), tone }
}

/// The update card for where `card` is.
pub fn card(card: &Card) -> View {
    let version = format!("Version {}", card.offer.version);
    let download = Element::Glyph { glyph: GLYPH_DOWNLOAD, tone: Tone::Accent };
    let answers = |main: &str| vec![Row::Buttons(vec![text_button(LATER, "Later", ButtonRole::Secondary), text_button(INSTALL, main, ButtonRole::Accent)])];
    let (mark, trailing, headline, detail, rows) = match &card.phase {
        Phase::Offered => (download, text("Update", Tone::Accent), version, card.offer.line.clone(), answers("Install")),
        Phase::Downloading(fraction) => {
            let percent = format!("{} %", (fraction * 100.0).round() as u32);
            let bar = Row::Progress { fraction: *fraction, leading: None, trailing: Some(percent) };
            (download, Element::Ring(*fraction), "Downloading".into(), Some(version), vec![bar])
        }
        Phase::Installing => (download, text("Installing", Tone::Accent), "Installing".into(), Some(version), Vec::new()),
        Phase::Failed(why) => (Element::Glyph { glyph: GLYPH_ERROR, tone: Tone::Negative }, text("Failed", Tone::Negative), "Couldn't update".into(), Some(why.clone()), answers("Try again")),
    };
    View {
        accent: Some(ACCENT),
        audio: None,
        compact: CompactView { leading: mark.clone(), trailing },
        minimal: mark,
        expanded: Some(ExpandedView {
            context: None,
            // No mark in the card: the eyebrow and the hero say what it is, and the text
            // column stays put from offer to progress to failure.
            leading: Element::Empty,
            eyebrow: Some("ReUI".into()),
            headline: Headline::Text(headline),
            detail,
            trailing: Element::Empty,
            rows,
            // Roomy lets the notes' line and an error's reason wrap rather than be cut.
            size: CardSize::Roomy,
            tint: false,
            pager: None,
            open: None,
        }),
    }
}

/// "Updated to ReUI 0.2.0", once after an upgrade, with What's new.
pub fn updated(version: &str) -> View {
    let art = Element::Glyph { glyph: GLYPH_NEW, tone: Tone::Secondary };
    View {
        accent: None,
        audio: None,
        compact: CompactView { leading: art.clone(), trailing: text("Updated", Tone::Secondary) },
        minimal: art.clone(),
        expanded: Some(ExpandedView {
            leading: art,
            eyebrow: Some("ReUI".into()),
            headline: Headline::Text(format!("Updated to ReUI {version}")),
            detail: None,
            trailing: Element::Empty,
            context: None,
            size: CardSize::Regular,
            tint: false,
            pager: None,
            open: None,
            rows: vec![Row::Buttons(vec![text_button(OPEN, "What's new", ButtonRole::Primary)])],
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::updates::state::Offer;

    fn buttons(view: &View) -> Vec<(String, ButtonRole)> {
        let rows = &view.expanded.as_ref().unwrap().rows;
        rows.iter()
            .flat_map(|r| match r {
                Row::Buttons(b) => b.iter().map(|b| (b.action.clone(), b.role)).collect(),
                _ => Vec::new(),
            })
            .collect()
    }

    #[test]
    fn the_card_asks_then_shows_progress_then_says_why_it_failed() {
        let mut card = Card::new(Offer { version: "0.2.0".into(), line: Some("Faster clipboard".into()) });
        let offered = super::card(&card);
        let expanded = offered.expanded.as_ref().unwrap();
        assert_eq!(expanded.headline, Headline::Text("Version 0.2.0".into()));
        assert_eq!(expanded.detail.as_deref(), Some("Faster clipboard"));
        assert_eq!(buttons(&offered), [(LATER.into(), ButtonRole::Secondary), (INSTALL.into(), ButtonRole::Accent)]);

        card.start();
        card.step(reui_protocol::install::InstallStep::Progress(0.42));
        let downloading = super::card(&card);
        assert!(buttons(&downloading).is_empty(), "nothing to press mid-download");
        assert!(matches!(downloading.expanded.unwrap().rows[0], Row::Progress { fraction, ref trailing, .. } if fraction == 0.42 && trailing.as_deref() == Some("42 %")));

        card.step(reui_protocol::install::InstallStep::Error("Not found on github.com".into()));
        let failed = super::card(&card);
        assert_eq!(failed.expanded.as_ref().unwrap().detail.as_deref(), Some("Not found on github.com"));
        assert_eq!(buttons(&failed), [(LATER.into(), ButtonRole::Secondary), (INSTALL.into(), ButtonRole::Accent)]);
    }
}
