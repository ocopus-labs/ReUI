//! A toast as the island shows it (docs/PRD-notifications.md §5.2). Pure.
//!
//! Always a Brief: a notification is news, never a decision, so it never expands by
//! itself and never takes the pill from music, a timer or an agent (it shows in the
//! bubble beside them). Compact: the app's icon and the title. Open: the app as the
//! eyebrow, the title as the hero, one line of the text as the detail, and Open.

use reui_protocol::{Art, Button, ButtonContent, ButtonRole, CardSize, CompactView, Element, ExpandedView, Headline, ImageId, Row, Tone};

use crate::hub::View;

/// Segoe Fluent's bell, for an app whose logo Windows doesn't hand over.
pub const GLYPH_BELL: char = '\u{EA8F}';
pub const ACTION_OPEN_APP: &str = "open-app";

/// What one toast shows. `title`/`body` are `None` when content is hidden.
#[derive(Debug, Clone, PartialEq)]
pub struct Shown {
    pub name: String,
    pub title: Option<String>,
    pub body: Option<String>,
    pub icon: Option<ImageId>,
}

/// The first line of the text, trimmed: the island has room for one.
fn one_line(text: &str) -> Option<String> {
    text.lines().map(str::trim).find(|l| !l.is_empty()).map(str::to_owned)
}

pub fn brief(shown: &Shown) -> View {
    let art = Element::Art(shown.icon.map_or(Art::Glyph(GLYPH_BELL), Art::Image));
    let title = shown.title.as_deref().and_then(one_line);
    // Hidden content (or a toast with no text): the app's name is all there is to say.
    let label = title.clone().unwrap_or_else(|| shown.name.clone());
    let (eyebrow, headline, detail) = match &title {
        Some(t) => (Some(shown.name.clone()).filter(|n| !n.is_empty()), t.clone(), shown.body.as_deref().and_then(one_line)),
        None => (None, shown.name.clone(), None),
    };
    let expanded = ExpandedView {
        leading: art.clone(),
        eyebrow,
        headline: Headline::Text(headline),
        detail,
        trailing: Element::Empty,
        rows: vec![Row::Buttons(vec![Button { action: ACTION_OPEN_APP.into(), content: ButtonContent::Text("Open".into()), role: ButtonRole::Secondary }])],
        context: None,
        size: CardSize::Regular,
        tint: false,
        pager: None,
        open: None,
    };
    View { accent: None, compact: CompactView { leading: art.clone(), trailing: Element::Text { text: label, tone: Tone::Secondary } }, minimal: art, expanded: Some(expanded), audio: None }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(title: Option<&str>, body: Option<&str>) -> Shown {
        Shown { name: "Teams".into(), title: title.map(str::to_owned), body: body.map(str::to_owned), icon: None }
    }

    #[test]
    fn notifications_brief_reads_app_title_line() {
        let v = brief(&shown(Some("Ana"), Some("\n  Lunch at one?\nsecond line")));
        assert_eq!(v.compact.leading, Element::Art(Art::Glyph(GLYPH_BELL)));
        assert_eq!(v.compact.trailing, Element::Text { text: "Ana".into(), tone: Tone::Secondary });
        let e = v.expanded.unwrap();
        assert_eq!((e.eyebrow.as_deref(), e.headline, e.detail.as_deref()), (Some("Teams"), Headline::Text("Ana".into()), Some("Lunch at one?")));
    }

    #[test]
    fn notifications_hidden_content_shows_the_app_alone() {
        let v = brief(&shown(None, None));
        assert_eq!(v.compact.trailing, Element::Text { text: "Teams".into(), tone: Tone::Secondary });
        let e = v.expanded.unwrap();
        assert_eq!((e.eyebrow, e.headline, e.detail), (None, Headline::Text("Teams".into()), None));
        let icon = brief(&Shown { icon: Some(ImageId(3)), ..shown(Some("  "), None) });
        assert_eq!(icon.minimal, Element::Art(Art::Image(ImageId(3))));
        assert_eq!(icon.compact.trailing, Element::Text { text: "Teams".into(), tone: Tone::Secondary }, "a blank title is no title");
    }
}
