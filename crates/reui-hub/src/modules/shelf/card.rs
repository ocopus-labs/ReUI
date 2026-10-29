//! The shelf as the island shows it (PRD §5.4): the pill's mark and how many things are
//! held, and the card: eyebrow, a clear button, then the clipboard's row of cards with the
//! clipboard's faces, newest first. Each card says what it carries when dragged out. Pure:
//! what's on disk (missing files, thumbnails, icons) comes in as [`Facts`].

use std::collections::{HashMap, HashSet};

use reui_protocol::{
    Art, Button, ButtonContent, ButtonRole, CardSize, CompactView, Drag, Element, ExpandedView, Headline, ImageId, Item, ItemAction, ItemKey, Preview, Rgb, Row, Tone,
};

use super::store::{Held, What};
use crate::hub::View;
use crate::modules::clipboard::{age, classify};

/// Package: things set aside to take somewhere.
pub const GLYPH: char = '\u{E7B8}';
const GLYPH_CLEAR: char = '\u{E894}';
pub const ACCENT: Rgb = Rgb(48, 176, 199);

/// What the disk says about the held things, looked up off the hub thread.
#[derive(Debug, Default, Clone)]
pub struct Facts {
    /// Paths that aren't there any more.
    pub missing: HashSet<String>,
    /// A picture of a card that holds one image file.
    pub thumbs: HashMap<u64, ImageId>,
    /// The file type's icon of a card's first file.
    pub icons: HashMap<u64, ImageId>,
}

pub fn view(items: &[Held], facts: &Facts, now_ms: i64) -> View {
    let glyph = Element::Art(Art::Glyph(GLYPH));
    let count = Element::Text { text: items.len().to_string(), tone: Tone::Secondary };
    let (trailing, rows) = if items.is_empty() {
        (Element::Empty, vec![Row::Note("Nothing held".into())])
    } else {
        let clear = Button { action: "clear".into(), content: ButtonContent::Glyph(GLYPH_CLEAR), role: ButtonRole::Plain };
        (Element::Button(clear), vec![Row::Items(items.iter().rev().map(|h| card(h, facts, now_ms)).collect())])
    };
    View {
        accent: Some(ACCENT),
        audio: None,
        compact: CompactView { leading: glyph.clone(), trailing: if items.is_empty() { Element::Empty } else { count } },
        minimal: glyph,
        expanded: Some(ExpandedView {
            context: None,
            leading: Element::Empty,
            eyebrow: Some("Shelf".into()),
            headline: Headline::Text(String::new()),
            detail: None,
            trailing,
            rows,
            size: CardSize::Wide,
            tint: false,
            pager: None,
            open: None,
        }),
    }
}

fn card(held: &Held, facts: &Facts, now_ms: i64) -> Item {
    let id = held.id;
    let (preview, meta, drag) = match &held.what {
        What::Files { paths, .. } => {
            let there: Vec<String> = paths.iter().filter(|p| !facts.missing.contains(*p)).cloned().collect();
            let name = file_name(paths.first().map_or("", String::as_str)).to_string();
            let preview = match facts.thumbs.get(&id) {
                Some(image) if paths.len() == 1 && !there.is_empty() => Preview::Image(*image),
                _ => Preview::Files { name, count: paths.len() as u32 },
            };
            let meta = if there.is_empty() {
                Some("Missing".into())
            } else if there.len() < paths.len() {
                Some(format!("{} missing", paths.len() - there.len()))
            } else if paths.len() > 1 {
                Some(format!("{} files", paths.len()))
            } else {
                None
            };
            (preview, meta, (!there.is_empty()).then_some(Drag::Files(there)))
        }
        What::Link { url } => {
            let preview = match classify(url) {
                link @ Preview::Link { .. } => link,
                _ => Preview::Text(url.clone()),
            };
            let host = match &preview {
                Preview::Link { host, .. } => Some(host.clone()),
                _ => None,
            };
            (preview, host, Some(Drag::Link(url.clone())))
        }
        What::Text { text } => {
            let lines = text.lines().count();
            let preview = match classify(text) {
                code @ Preview::Code(_) => code,
                _ => Preview::Text(text.trim().to_string()),
            };
            (preview, (lines > 1).then(|| format!("{lines} lines")), Some(Drag::Text(text.clone())))
        }
    };
    let gone = drag.is_none();
    let act = |label: &str, verb: &str, key: Option<ItemKey>| ItemAction { label: label.into(), action: format!("{verb}:{id}"), key };
    let mut actions = Vec::new();
    if !gone {
        actions.push(act("Open", "open", None));
    }
    if matches!(held.what, What::Files { .. }) && !gone {
        actions.push(act("Show in folder", "show", Some(ItemKey::CtrlEnter)));
    }
    let copy = match held.what {
        What::Files { .. } => "Copy path",
        What::Link { .. } => "Copy link",
        What::Text { .. } => "Copy",
    };
    actions.push(act(copy, "copy", None));
    actions.push(act("Remove", "remove", Some(ItemKey::Delete)));
    Item {
        // A missing file opens nothing; a click on it does nothing.
        action: if gone { String::new() } else { format!("open:{id}") },
        preview,
        icon: facts.icons.get(&id).copied(),
        age: age(now_ms - held.added_ms),
        meta,
        pinned: false,
        actions,
        drag,
    }
}

pub fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).find(|p| !p.is_empty()).unwrap_or(path)
}

/// An island action on a card ("open:3") as its verb and card id.
pub fn parse(action: &str) -> Option<(&str, u64)> {
    let (verb, id) = action.split_once(':')?;
    Some((verb, id.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn held(id: u64, what: What) -> Held {
        Held { id, added_ms: 0, what }
    }

    fn files(paths: &[&str]) -> What {
        What::Files { paths: paths.iter().map(|p| p.to_string()).collect(), owned: false, bytes: 0 }
    }

    fn items(view: &View) -> Vec<Item> {
        match &view.expanded.as_ref().unwrap().rows[0] {
            Row::Items(items) => items.clone(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_pill_says_how_many_and_an_empty_card_says_nothing_is_held() {
        let empty = view(&[], &Facts::default(), 0);
        assert_eq!(empty.compact.trailing, Element::Empty);
        assert_eq!(empty.expanded.as_ref().unwrap().rows, [Row::Note("Nothing held".into())]);
        assert_eq!(empty.expanded.as_ref().unwrap().trailing, Element::Empty, "nothing to clear");
        let two = view(&[held(1, files(&["a"])), held(2, What::Text { text: "hi".into() })], &Facts::default(), 0);
        assert_eq!(two.compact.trailing, Element::Text { text: "2".into(), tone: Tone::Secondary });
        assert!(matches!(two.expanded.as_ref().unwrap().trailing, Element::Button(Button { ref action, .. }) if action == "clear"));
        assert_eq!(items(&two).iter().map(|i| i.action.as_str()).collect::<Vec<_>>(), ["open:2", "open:1"], "newest first");
    }

    #[test]
    fn files_drag_out_by_path_and_missing_ones_say_so() {
        let mut facts = Facts::default();
        let one = view(&[held(1, files(&[r"C:\docs\Report.pdf"]))], &facts, 120_000);
        let card = &items(&one)[0];
        assert_eq!(card.preview, Preview::Files { name: "Report.pdf".into(), count: 1 });
        assert_eq!((card.age.as_str(), card.meta.as_deref()), ("2m", None));
        assert_eq!(card.drag, Some(Drag::Files(vec![r"C:\docs\Report.pdf".into()])));
        let labels: Vec<&str> = card.actions.iter().map(|a| a.label.as_str()).collect();
        assert_eq!(labels, ["Open", "Show in folder", "Copy path", "Remove"]);

        facts.missing.insert(r"C:\b.txt".into());
        let some = &items(&view(&[held(2, files(&[r"C:\a.txt", r"C:\b.txt"]))], &facts, 0))[0];
        assert_eq!((some.meta.as_deref(), some.drag.clone()), (Some("1 missing"), Some(Drag::Files(vec![r"C:\a.txt".into()]))));
        let gone = &items(&view(&[held(3, files(&[r"C:\b.txt"]))], &facts, 0))[0];
        assert_eq!((gone.meta.as_deref(), gone.drag.as_ref(), gone.action.as_str()), (Some("Missing"), None, ""));
        assert_eq!(gone.actions.iter().map(|a| a.label.as_str()).collect::<Vec<_>>(), ["Copy path", "Remove"]);
    }

    #[test]
    fn an_image_shows_itself_and_links_and_text_carry_themselves() {
        let mut facts = Facts::default();
        facts.thumbs.insert(1, ImageId(9));
        facts.icons.insert(1, ImageId(4));
        let shelf = [held(1, files(&[r"C:\x\cat.png"])), held(2, What::Link { url: "https://www.figma.com/file/abc".into() }), held(3, What::Text { text: "one\ntwo".into() })];
        let cards = items(&view(&shelf, &facts, 0));
        assert_eq!((cards[2].preview.clone(), cards[2].icon), (Preview::Image(ImageId(9)), Some(ImageId(4))));
        assert!(matches!(&cards[1].preview, Preview::Link { host, .. } if host == "figma.com"));
        assert_eq!((cards[1].meta.as_deref(), cards[1].drag.clone()), (Some("figma.com"), Some(Drag::Link("https://www.figma.com/file/abc".into()))));
        assert_eq!((cards[0].meta.as_deref(), cards[0].drag.clone()), (Some("2 lines"), Some(Drag::Text("one\ntwo".into()))));
    }

    #[test]
    fn card_actions_parse() {
        assert_eq!(parse("open:12"), Some(("open", 12)));
        assert_eq!(parse("remove:x"), None);
        assert_eq!(parse("clear"), None);
    }
}
