//! The clipboard in the notch (`docs/PRD-clipboard-panel-v3.md`): the clipboard's view
//! (see `clipboard_view`) as the island's panel rows, one toolbar (type filters, search,
//! how many match, Clear, settings) over the cards, and the island's actions turned back
//! into the same messages the module already handles. One set of logic, whatever draws it.

use reui_protocol::{
    Button, ButtonContent, ButtonRole, CardSize, CompactView, Element, ExpandedView, Headline, ImageId, Item, ItemAction, ItemKey, MenuOption, Preview, Rgb, Row, Tab,
    ACTION_SEARCH, ACTION_UNDO,
};
use serde_json::json;

use super::view as cv;
use crate::hub::View;

const GLYPH_CLIPBOARD: char = '\u{E77F}';
const GLYPH_HISTORY: char = '\u{E81C}';
const GLYPH_PINNED: char = '\u{E718}';
const GLYPH_COLLECTION: char = '\u{E8EC}';
const GLYPH_CLEAR: char = '\u{E74D}';
const GLYPH_UNDO: char = '\u{E7A7}';
const GLYPH_SETTINGS: char = '\u{E713}';
/// The type filters after "All": (kind, label, glyph). Text covers code too.
const TYPES: [(&str, &str, char); 5] =
    [("text", "Text", '\u{E8D2}'), ("links", "Links", '\u{E71B}'), ("images", "Images", '\u{EB9F}'), ("files", "Files", '\u{E8A5}'), ("colors", "Colors", '\u{E790}')];

/// The notch panel for a view: the toolbar, then the cards (or one quiet line).
pub fn panel(view: &cv::View, accent: Rgb) -> View {
    let mut buttons = Vec::new();
    // Pausing lives in the tray and the card menu; the panel only says so while paused,
    // in the count's place.
    buttons.push(if view.paused {
        Button { action: "resume".into(), content: ButtonContent::Text("Resume".into()), role: ButtonRole::Secondary }
    } else {
        Button { action: String::new(), content: ButtonContent::Count(view.total), role: ButtonRole::Secondary }
    });
    // Undo takes Clear's place while a delete or a clear can be undone.
    buttons.push(if view.undo {
        Button { action: ACTION_UNDO.into(), content: ButtonContent::Labelled(GLYPH_UNDO, "Undo".into()), role: ButtonRole::Secondary }
    } else {
        Button { action: "clear".into(), content: ButtonContent::Labelled(GLYPH_CLEAR, "Clear".into()), role: ButtonRole::Secondary }
    });
    buttons.push(Button { action: "settings".into(), content: ButtonContent::Glyph(GLYPH_SETTINGS), role: ButtonRole::Plain });

    let content = if view.items.is_empty() {
        Row::Note(view.empty.clone().unwrap_or_else(|| "No matches".into()))
    } else {
        Row::Items(view.items.iter().map(|i| item(i, view)).collect())
    };
    let glyph = Element::Art(reui_protocol::Art::Glyph(GLYPH_CLIPBOARD));
    View {
        accent: Some(accent),
        audio: None,
        compact: CompactView { leading: glyph.clone(), trailing: Element::Empty },
        minimal: glyph,
        expanded: Some(ExpandedView {
            leading: Element::Empty,
            eyebrow: None,
            headline: Headline::Text(String::new()),
            detail: None,
            trailing: Element::Empty,
            rows: vec![Row::Search { query: view.query.clone(), buttons, placeholder: Some("Search history\u{2026}".into()), glyph: None, filters: filters(view) }, content],
            context: None, size: CardSize::Wide,
            tint: false,
            pager: None,
            open: None,
        }),
    }
}

/// "All" and one filter per type. "All" shows which collection it covers (History, Pinned
/// or one of the user's) and, once chosen, offers the others as its menu.
fn filters(view: &cv::View) -> Vec<Tab> {
    let kind = view.filter.kind.as_deref();
    let shown = view.collections.iter().filter(|c| c.builtin || c.count > 0 || c.id == view.collection);
    let menu: Vec<MenuOption> = shown.map(|c| MenuOption { label: c.name.clone(), action: format!("tab:{}", c.id), selected: c.id == view.collection }).collect();
    let current = view.collections.iter().find(|c| c.id == view.collection);
    let (label, glyph) = match view.collection.as_str() {
        "history" => ("History".to_string(), GLYPH_HISTORY),
        "pinned" => ("Pinned".to_string(), GLYPH_PINNED),
        _ => (current.map_or_else(|| "Collection".to_string(), |c| c.name.clone()), GLYPH_COLLECTION),
    };
    // A menu of History alone has nothing to choose.
    let menu = if menu.len() > 1 { menu } else { Vec::new() };
    let all = Tab { label, count: view.total, selected: kind.is_none(), action: "type:all".into(), glyph: Some(glyph), menu };
    let types = TYPES.iter().map(|&(id, label, glyph)| Tab { label: label.into(), count: 0, selected: kind == Some(id), action: format!("type:{id}"), glyph: Some(glyph), menu: Vec::new() });
    std::iter::once(all).chain(types).collect()
}

fn image(id: &Option<String>) -> Option<ImageId> {
    id.as_deref().and_then(|s| s.parse().ok()).map(ImageId)
}

fn item(it: &cv::Item, view: &cv::View) -> Item {
    let text = it.text.clone().unwrap_or_default();
    let preview = match it.kind.as_str() {
        "image" => image(&it.image).map_or_else(|| Preview::Text("Image".into()), Preview::Image),
        "link" => {
            let url = it.url.clone().unwrap_or_else(|| text.clone());
            let rest = url.split_once("://").map_or(url.as_str(), |(_, r)| r);
            let (host, path) = rest.split_once('/').map_or((rest, ""), |(h, p)| (h, p));
            Preview::Link { host: host.trim_start_matches("www.").to_string(), path: format!("/{path}"), title: it.title.clone(), site: it.site.clone(), image: image(&it.image) }
        }
        "color" => it.color.as_deref().and_then(hex).map_or_else(|| Preview::Text(text.clone()), Preview::Color),
        "files" => {
            let files = it.files.clone().unwrap_or_default();
            Preview::Files { name: files.first().cloned().unwrap_or_default(), count: files.len() as u32 }
        }
        "code" => Preview::Code(text),
        _ => Preview::Text(text),
    };
    Item {
        action: format!("paste:{}", it.id),
        preview,
        icon: image(&it.icon),
        age: it.age.clone(),
        meta: it.meta.clone(),
        pinned: it.pinned,
        actions: actions(it, view),
        drag: None,
    }
}

/// The card's menu (right-click) and the shortcuts on the selected card.
fn actions(it: &cv::Item, view: &cv::View) -> Vec<ItemAction> {
    let id = it.id;
    let act = |label: &str, action: String, key: Option<ItemKey>| ItemAction { label: label.into(), action, key };
    let text_like = matches!(it.kind.as_str(), "text" | "code" | "link");
    let mut list = Vec::new();
    if text_like {
        list.push(act("Paste as plain text", format!("paste-plain:{id}"), Some(ItemKey::ShiftEnter)));
    }
    list.push(act("Copy", format!("copy:{id}"), Some(ItemKey::CtrlEnter)));
    if text_like {
        for (label, mode) in [("Paste as UPPERCASE", "upper"), ("Paste as lowercase", "lower"), ("Paste as Title Case", "title"), ("Paste trimmed", "trim"), ("Paste as one line", "single-line"), ("Paste as formatted JSON", "json"), ("Paste as Markdown", "markdown")] {
            list.push(act(label, format!("paste-as:{mode}:{id}"), None));
        }
    }
    if it.ocr {
        list.push(act("Copy text from image", format!("ocr:{id}"), None));
    }
    for c in view.collections.iter().filter(|c| !c.builtin) {
        if it.collections.contains(&c.id) {
            list.push(act(&format!("Remove from {}", c.name), format!("uncollect:{}:{id}", c.id), None));
        } else {
            list.push(act(&format!("Add to {}", c.name), format!("collect:{}:{id}", c.id), None));
        }
    }
    list.push(if it.pinned { act("Unpin", format!("unpin:{id}"), Some(ItemKey::CtrlP)) } else { act("Pin", format!("pin:{id}"), Some(ItemKey::CtrlP)) });
    list.push(act("Delete", format!("delete:{id}"), Some(ItemKey::Delete)));
    list
}

fn hex(text: &str) -> Option<Rgb> {
    let h = text.trim().trim_start_matches('#');
    if h.len() != 6 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let c = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some(Rgb(c(0)?, c(2)?, c(4)?))
}

/// An island action as the message the module handles (`None` for ones it handles itself:
/// open, expanded, collapsed, pause, resume, clear).
pub fn to_message(action: &str, view: &cv::View) -> Option<String> {
    let query = |text: &str, collection: &str| json!({ "type": "query", "query": text, "collection": collection, "filter": { "type": view.filter.kind, "app": view.filter.app } });
    if let Some(kind) = action.strip_prefix("type:") {
        let kind = (kind != "all").then_some(kind);
        return Some(json!({ "type": "query", "query": view.query, "collection": view.collection, "filter": { "type": kind, "app": view.filter.app } }).to_string());
    }
    if let Some(text) = action.strip_prefix(ACTION_SEARCH) {
        return Some(query(text, &view.collection).to_string());
    }
    if action == ACTION_UNDO {
        return Some(json!({ "type": "undo" }).to_string());
    }
    let (verb, rest) = action.split_once(':')?;
    let id = |s: &str| s.parse::<i64>().ok();
    let message = match verb {
        "tab" => query(&view.query, rest),
        // The island never takes focus, so the app the user is in still has it.
        "paste" => json!({ "type": "paste", "ids": [id(rest)?], "mode": "normal", "focused": true }),
        "paste-plain" => json!({ "type": "paste", "ids": [id(rest)?], "mode": "plain", "focused": true }),
        "paste-as" => {
            let (mode, item) = rest.split_once(':')?;
            json!({ "type": "paste", "ids": [id(item)?], "mode": mode, "focused": true })
        }
        "copy" => json!({ "type": "copy", "ids": [id(rest)?], "mode": "normal" }),
        "pin" | "unpin" => json!({ "type": verb, "id": id(rest)? }),
        "delete" => json!({ "type": "delete", "ids": [id(rest)?] }),
        "ocr" => json!({ "type": "ocr", "id": id(rest)? }),
        "collect" | "uncollect" => {
            let (collection, item) = rest.split_once(':')?;
            json!({ "type": verb, "ids": [id(item)?], "collection": collection })
        }
        _ => return None,
    };
    Some(message.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> cv::View {
        cv::View { collection: "history".into(), query: "fig".into(), ..Default::default() }
    }

    #[test]
    fn island_actions_become_module_messages() {
        let v = view();
        let parse = |a: &str| serde_json::from_str::<serde_json::Value>(&to_message(a, &v).unwrap()).unwrap();
        assert_eq!(parse("search:abc")["query"], "abc");
        assert_eq!(parse("search:abc")["collection"], "history");
        assert_eq!(parse("tab:c7")["collection"], "c7");
        assert_eq!(parse("tab:c7")["query"], "fig");
        assert_eq!(parse("paste:12"), json!({ "type": "paste", "ids": [12], "mode": "normal", "focused": true }));
        assert_eq!(parse("paste-as:upper:12")["mode"], "upper");
        assert_eq!(parse("collect:c7:12"), json!({ "type": "collect", "ids": [12], "collection": "c7" }));
        assert_eq!(parse("undo"), json!({ "type": "undo" }));
        assert_eq!(parse("type:links")["filter"]["type"], "links");
        assert_eq!(parse("type:links")["query"], "fig");
        assert!(parse("type:all")["filter"]["type"].is_null());
        assert!(to_message("expanded", &v).is_none());
        assert!(to_message("settings", &v).is_none());
        assert!(to_message("paste:abc", &v).is_none());
    }

    #[test]
    fn cards_carry_their_menu() {
        let mut v = view();
        v.collections = vec![cv::CollectionTab { id: "c7".into(), name: "Prompts".into(), count: 1, color: None, builtin: false }];
        let it = cv::Item { id: 3, kind: "text".into(), text: Some("hi".into()), age: "2m".into(), ..Default::default() };
        let item = item(&it, &v);
        assert_eq!(item.action, "paste:3");
        let labels: Vec<&str> = item.actions.iter().map(|a| a.label.as_str()).collect();
        assert!(labels.contains(&"Paste as plain text") && labels.contains(&"Add to Prompts") && labels.contains(&"Delete"));
        assert_eq!(hex("#0A84FF"), Some(Rgb(10, 132, 255)));
    }

    #[test]
    fn the_toolbar_has_filters_a_count_clear_and_settings() {
        let mut v = view();
        v.total = 4;
        v.collections = vec![
            cv::CollectionTab { id: "history".into(), name: "History".into(), count: 9, color: None, builtin: true },
            cv::CollectionTab { id: "pinned".into(), name: "Pinned".into(), count: 0, color: None, builtin: true },
        ];
        let p = panel(&v, Rgb(0, 0, 0));
        let rows = p.expanded.unwrap().rows;
        let Row::Search { filters, buttons, placeholder, .. } = &rows[0] else { panic!("toolbar first") };
        assert_eq!(placeholder.as_deref(), Some("Search history\u{2026}"));
        let actions: Vec<&str> = filters.iter().map(|t| t.action.as_str()).collect();
        assert_eq!(actions, ["type:all", "type:text", "type:links", "type:images", "type:files", "type:colors"]);
        assert!(filters[0].selected && filters[0].glyph == Some(GLYPH_HISTORY));
        // History and Pinned: something to choose between.
        assert_eq!(filters[0].menu.len(), 2);
        assert_eq!(buttons.iter().map(|b| b.content.clone()).collect::<Vec<_>>(), [ButtonContent::Count(4), ButtonContent::Labelled(GLYPH_CLEAR, "Clear".into()), ButtonContent::Glyph(GLYPH_SETTINGS)]);
        assert!(buttons[0].action.is_empty());

        // In Pinned with a type chosen: "All" says Pinned; a pending undo replaces Clear.
        v.collection = "pinned".into();
        v.filter.kind = Some("links".into());
        v.undo = true;
        let rows = panel(&v, Rgb(0, 0, 0)).expanded.unwrap().rows;
        let Row::Search { filters, buttons, .. } = &rows[0] else { panic!() };
        assert!(!filters[0].selected && filters[0].glyph == Some(GLYPH_PINNED) && filters[2].selected);
        assert_eq!(buttons[1].action, ACTION_UNDO);
    }
}
