//! The screenshot card: the shot's thumbnail, whether it's on the clipboard, its size, and
//! (once it's saved) Open and Show in folder.

use reui_protocol::{Art, Button, ButtonContent, ButtonRole, CardSize, CompactView, Element, ExpandedView, Headline, ImageId, Row, Tone};

use super::shots::Shot;
use crate::hub::View;

const GLYPH_SHOT: char = '\u{E722}';

pub const OPEN: &str = "open";
pub const SHOW: &str = "show";

fn text_button(action: &str, label: &str, role: ButtonRole) -> Button {
    Button { action: action.into(), content: ButtonContent::Text(label.into()), role }
}

/// `thumb` is the shot's thumbnail in the image store, once there is one.
pub fn shot(shot: &Shot, thumb: Option<ImageId>) -> View {
    let mark = Element::Art(thumb.map_or(Art::Glyph(GLYPH_SHOT), Art::Image));
    // On the clipboard is what the user can do something with right away; a shot only
    // saved says so.
    let hero = if shot.copied { "Copied" } else { "Saved" };
    let size = shot.size.map(|(w, h)| format!("{w} × {h}"));
    let folder = shot.file.as_deref().and_then(|f| f.parent()?.file_name()).map(|n| n.to_string_lossy().into_owned());
    let detail = [size, folder].into_iter().flatten().collect::<Vec<_>>().join(" · ");
    let rows = match shot.file {
        Some(_) => vec![Row::Buttons(vec![text_button(SHOW, "Show in folder", ButtonRole::Secondary), text_button(OPEN, "Open", ButtonRole::Accent)])],
        None => Vec::new(),
    };
    View {
        accent: None,
        audio: None,
        compact: CompactView { leading: mark.clone(), trailing: Element::Text { text: if shot.copied { "Copied" } else { "Screenshot" }.into(), tone: Tone::Secondary } },
        minimal: mark.clone(),
        expanded: Some(ExpandedView {
            context: None,
            leading: mark,
            eyebrow: Some("Screenshot".into()),
            headline: Headline::Text(hero.into()),
            detail: (!detail.is_empty()).then_some(detail),
            trailing: Element::Empty,
            rows,
            size: CardSize::Regular,
            tint: false,
            pager: None,
            open: shot.file.as_ref().map(|_| OPEN.into()),
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::modules::screenshots::shots::Shots;

    #[test]
    fn a_copied_shot_says_so_and_a_saved_one_offers_its_file() {
        let mut shots = Shots::default();
        shots.copied(vec![1], Some((1280, 720)), 0);
        let view = shot(shots.current().unwrap(), Some(ImageId(5)));
        assert_eq!(view.compact.leading, Element::Art(Art::Image(ImageId(5))));
        assert_eq!(view.compact.trailing, Element::Text { text: "Copied".into(), tone: Tone::Secondary });
        let card = view.expanded.unwrap();
        assert_eq!((card.headline, card.detail.as_deref(), card.rows.len(), card.open), (Headline::Text("Copied".into()), Some("1280 × 720"), 0, None), "nothing to open until it's saved");

        let file = PathBuf::from(r"C:\Users\me\Pictures\Screenshots\Screenshot 1.png");
        shots.file_changed(&file, 100);
        let due = shots.due_read(1_000).unwrap();
        shots.file_read(due, Some((vec![2], None)), 1_000);
        let card = shot(shots.current().unwrap(), None).expanded.unwrap();
        assert_eq!(card.detail.as_deref(), Some("1280 × 720 · Screenshots"));
        assert_eq!(card.rows, vec![Row::Buttons(vec![text_button(SHOW, "Show in folder", ButtonRole::Secondary), text_button(OPEN, "Open", ButtonRole::Accent)])]);
        assert_eq!(card.open.as_deref(), Some(OPEN));
        assert_eq!(card.leading, Element::Art(Art::Glyph(GLYPH_SHOT)), "a glyph until the thumbnail is stored");
    }

    #[test]
    fn a_shot_only_saved_reads_saved() {
        let mut shots = Shots::default();
        let file = PathBuf::from(r"C:\s\a.png");
        shots.file_changed(&file, 0);
        let due = shots.due_read(1_000).unwrap();
        shots.file_read(due, Some((vec![2], None)), 1_000);
        let view = shot(shots.current().unwrap(), None);
        assert_eq!(view.compact.trailing, Element::Text { text: "Screenshot".into(), tone: Tone::Secondary });
        assert_eq!(view.expanded.unwrap().headline, Headline::Text("Saved".into()));
    }
}
