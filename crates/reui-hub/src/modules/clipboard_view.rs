//! The clipboard window's data, in the shape of `docs/clipboard-window-contract.md`: the
//! `View` and `Item` the page renders and the hub → window messages that carry them. Field
//! names are the contract's; tests pin them.
//!
//! Images (thumbnails, app icons, page images, the full image of a preview) go to the hub's
//! image store on the way out, and every message lists the ids it refers to so the host
//! has them before the page asks.

use reui_protocol::{ImageId, Preview, Rgb};
use serde::{Deserialize, Serialize};

use super::clipboard::{classify, prefix};
use super::clipboard_os::Target;
use super::clipboard_store::{Kind, Page, Query, Saved};

/// Text sent for a card (E-X1); the full text comes with `preview`.
pub const ITEM_CHARS: usize = 2000;
/// Text sent for the preview pane.
pub const PREVIEW_BYTES: usize = 200 * 1024;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct View {
    pub query: String,
    pub collection: String,
    pub filter: Filter,
    pub layout: String,
    pub collections: Vec<CollectionTab>,
    pub types: Vec<TypeChip>,
    pub apps: Vec<AppChip>,
    pub items: Vec<Item>,
    pub total: u32,
    pub paused: bool,
    pub undo: bool,
    pub stack: Stack,
    pub empty: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Filter {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub app: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CollectionTab {
    pub id: String,
    pub name: String,
    pub count: u32,
    pub color: Option<String>,
    pub builtin: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct TypeChip {
    pub id: String,
    pub label: String,
    pub count: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct AppChip {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub count: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Stack {
    pub active: bool,
    pub remaining: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Item {
    pub id: i64,
    pub kind: String,
    pub text: Option<String>,
    pub rich: bool,
    pub title: Option<String>,
    pub site: Option<String>,
    pub url: Option<String>,
    pub image: Option<String>,
    pub icon: Option<String>,
    pub app: Option<String>,
    pub age: String,
    pub meta: Option<String>,
    pub color: Option<String>,
    pub files: Option<Vec<String>>,
    pub pinned: bool,
    pub collections: Vec<String>,
    pub cut: bool,
    pub missing: bool,
    pub lines: u32,
    pub ocr: bool,
}

/// Window state that isn't in the database.
#[derive(Debug, Clone, Default)]
pub struct Ui {
    pub query: Query,
    pub layout: String,
    pub paused: bool,
    pub undo: bool,
    pub stack: Stack,
}

/// Registers image bytes and remembers the ids a message refers to.
pub struct Images<'a> {
    add: &'a dyn Fn(Vec<u8>) -> ImageId,
    pub ids: Vec<ImageId>,
}

impl<'a> Images<'a> {
    pub fn new(add: &'a dyn Fn(Vec<u8>) -> ImageId) -> Self {
        Self { add, ids: Vec::new() }
    }

    /// The id as a decimal string: real ids are 64-bit and JavaScript numbers would round them.
    pub fn add(&mut self, bytes: Vec<u8>) -> String {
        let id = (self.add)(bytes);
        if !self.ids.contains(&id) {
            self.ids.push(id);
        }
        id.0.to_string()
    }
}

pub fn view(page: &Page, ui: &Ui, now_ms: i64, images: &mut Images) -> View {
    let q = &ui.query;
    let empty = if page.history == 0 {
        Some("Nothing copied yet".to_string())
    } else if page.total == 0 {
        Some("No matches".to_string())
    } else {
        None
    };
    View {
        query: q.text.clone(),
        collection: q.scope.id(),
        filter: Filter { kind: q.kind.map(|k| k.name().to_string()), app: q.app.clone() },
        layout: ui.layout.clone(),
        collections: page
            .scopes
            .iter()
            .map(|s| CollectionTab { id: s.scope.id(), name: s.name.clone(), count: s.count, color: s.color.clone(), builtin: !matches!(s.scope, super::clipboard_store::Scope::Collection(_)) })
            .collect(),
        types: page.kinds.iter().map(|&(k, count)| TypeChip { id: k.name().into(), label: k.label().into(), count }).collect(),
        apps: page.apps.iter().map(|a| AppChip { id: a.id.clone(), name: a.name.clone(), icon: a.icon.clone().map(|i| images.add(i)), count: a.count }).collect(),
        items: page.items.iter().map(|s| item(s, now_ms, images)).collect(),
        total: page.total,
        paused: ui.paused,
        undo: ui.undo,
        stack: ui.stack.clone(),
        empty,
    }
}

pub fn item(saved: &Saved, now_ms: i64, images: &mut Images) -> Item {
    let kind = Kind::parse(&saved.category).unwrap_or(Kind::Text);
    let text = saved.text.as_deref().unwrap_or_default();
    let lines = (saved.lines > 1).then(|| format!("{} lines", saved.lines));
    let formatted = || lines.clone().or_else(|| saved.rich.then(|| "Formatted".into()));
    let mut it = Item {
        id: saved.id,
        kind: kind.item_kind().into(),
        rich: saved.rich,
        icon: saved.icon.clone().map(|png| images.add(png)),
        app: saved.app.clone(),
        age: age(now_ms - saved.created_ms),
        pinned: saved.pinned,
        collections: saved.collections.iter().map(|c| format!("c{c}")).collect(),
        lines: saved.lines,
        ocr: saved.ocr,
        ..Default::default()
    };
    match kind {
        Kind::Images => {
            it.image = saved.thumb.clone().map(|png| images.add(png));
            it.meta = saved.size.map(|(w, h)| format!("{w} × {h}"));
        }
        Kind::Files => {
            it.files = Some(saved.files.iter().map(|f| file_name(f).to_string()).collect());
            it.cut = saved.cut;
            it.missing = saved.missing;
            it.meta = if saved.missing {
                Some("Missing".into())
            } else if saved.cut {
                Some("Cut".into())
            } else {
                (saved.files.len() > 1).then(|| format!("{} files", saved.files.len()))
            };
        }
        Kind::Links => match classify(text) {
            Preview::Link { host, path, .. } => {
                it.url = Some(display_url(text.trim(), &host, &path));
                it.text = it.url.clone();
                it.title = saved.link.title.clone();
                it.site = saved.link.site.clone();
                it.image = saved.link.image.clone().map(|png| images.add(png));
                it.meta = Some(saved.link.site.clone().unwrap_or(host));
            }
            _ => it.text = Some(prefix(text, ITEM_CHARS).to_string()),
        },
        Kind::Colors => {
            it.text = Some(text.trim().to_string());
            if let Preview::Color(Rgb(r, g, b)) = classify(text) {
                it.color = Some(format!("#{r:02X}{g:02X}{b:02X}"));
            }
            it.meta = Some(if text.trim().starts_with('#') { "HEX" } else { "RGB" }.into());
        }
        Kind::Text | Kind::Code => {
            it.text = Some(prefix(text, ITEM_CHARS).to_string());
            it.meta = formatted();
        }
    }
    it
}

/// The URL as shown: without `user:password@`.
fn display_url(url: &str, host: &str, path: &str) -> String {
    let scheme = url.split_once("://").map_or("https", |(s, _)| s);
    let full_host = url.split_once("://").map_or(url, |(_, r)| r).split(['/', '?', '#']).next().unwrap_or("");
    if full_host.contains('@') {
        let host = full_host.rsplit_once('@').map_or(host, |(_, h)| h);
        return format!("{scheme}://{host}{path}");
    }
    url.to_string()
}

fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).find(|p| !p.is_empty()).unwrap_or(path)
}

/// How long ago, as the card footer says it: "now", "2m", "3h", "4d".
pub fn age(ms: i64) -> String {
    let minutes = ms / 60_000;
    match minutes {
        i64::MIN..1 => "now".into(),
        1..60 => format!("{minutes}m"),
        60..1440 => format!("{}h", minutes / 60),
        _ => format!("{}d", minutes / 1440),
    }
}

/// `{"type":"open","target":…,"view":…}`.
#[allow(dead_code)] // the separate window is parked; the clipboard lives in the notch
pub fn open_message(target: &Target, view: &View) -> String {
    let caret = target.caret.map(|c| serde_json::json!(c));
    serde_json::json!({ "type": "open", "target": { "hwnd": target.hwnd, "focus": target.focus, "caret": caret }, "view": view }).to_string()
}

#[allow(dead_code)] // the separate window is parked; the clipboard lives in the notch
pub fn state_message(view: &View) -> String {
    serde_json::json!({ "type": "state", "view": view }).to_string()
}

pub fn preview_message(id: i64, text: Option<&str>, image: Option<String>) -> String {
    serde_json::json!({ "type": "preview", "id": id, "text": text, "image": image }).to_string()
}

#[allow(dead_code)] // the separate window is parked; the clipboard lives in the notch
pub fn close_message() -> String {
    serde_json::json!({ "type": "close" }).to_string()
}

/// The drag-out payload: `text`, `png` (an image id) or `files`, whichever the item has.
pub fn export_message(id: i64, text: Option<&str>, png: Option<String>, files: Option<&[String]>) -> String {
    let mut message = serde_json::json!({ "type": "export", "id": id });
    if let Some(text) = text {
        message["text"] = text.into();
    }
    if let Some(png) = png {
        message["png"] = png.into();
    }
    if let Some(files) = files {
        message["files"] = files.into();
    }
    message.to_string()
}

/// The start of `text`, at most `bytes` long, cut on a character boundary.
pub fn cap_bytes(text: &str, bytes: usize) -> &str {
    if text.len() <= bytes {
        return text;
    }
    let mut end = bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::clipboard_store::{AppCount, Scope, ScopeCount};
    use crate::modules::link::LinkMeta;
    use serde_json::Value;

    fn fake(bytes: Vec<u8>) -> ImageId {
        ImageId(1000 + bytes.len() as u64)
    }

    fn saved(category: &str) -> Saved {
        Saved { id: 7, category: category.into(), created_ms: 0, ..Default::default() }
    }

    fn one(s: Saved) -> Item {
        item(&s, 0, &mut Images::new(&fake))
    }

    #[test]
    fn ages_read_like_footers() {
        assert_eq!(age(10_000), "now");
        assert_eq!(age(2 * 60_000), "2m");
        assert_eq!(age(3 * 3_600_000), "3h");
        assert_eq!(age(30 * 3_600_000), "1d");
        assert_eq!(age(-5), "now");
    }

    #[test]
    fn items_per_kind() {
        let text = one(Saved { text: Some("a\nb\nc".into()), lines: 3, rich: true, collections: vec![3, 9], ..saved("text") });
        assert_eq!((text.kind.as_str(), text.meta.as_deref(), text.lines, text.collections.clone()), ("text", Some("3 lines"), 3, vec!["c3".to_string(), "c9".into()]));
        assert_eq!(one(Saved { text: Some("one".into()), lines: 1, rich: true, ..saved("text") }).meta.as_deref(), Some("Formatted"));
        assert_eq!(one(Saved { text: Some("one".into()), lines: 1, ..saved("text") }).meta, None);
        let long = one(Saved { text: Some("€".repeat(5000)), ..saved("code") });
        assert_eq!((long.kind.as_str(), long.text.unwrap().chars().count()), ("code", ITEM_CHARS));

        let image = item(&Saved { thumb: Some(vec![0; 5]), size: Some((640, 400)), ocr: true, icon: Some(vec![0; 2]), ..saved("images") }, 0, &mut Images::new(&fake));
        assert_eq!((image.kind.as_str(), image.image, image.icon, image.meta.as_deref(), image.ocr, image.text), ("image", Some("1005".to_string()), Some("1002".to_string()), Some("640 × 400"), true, None));

        let files = |paths: &[&str]| paths.iter().map(|p| p.to_string()).collect::<Vec<_>>();
        let f = one(Saved { files: files(&[r"C:\a\Report.pdf", r"C:\b\x.png"]), ..saved("files") });
        assert_eq!((f.kind.as_str(), f.files.clone(), f.meta.as_deref()), ("files", Some(vec!["Report.pdf".to_string(), "x.png".into()]), Some("2 files")));
        let cut = one(Saved { files: files(&["a"]), cut: true, ..saved("files") });
        assert_eq!((cut.cut, cut.meta.as_deref()), (true, Some("Cut")));
        let gone = one(Saved { files: files(&["a"]), cut: true, missing: true, ..saved("files") });
        assert_eq!((gone.missing, gone.meta.as_deref()), (true, Some("Missing")));

        let link = Saved { text: Some("https://me:pw@www.figma.com/design/abc".into()), link: LinkMeta { title: Some("Design".into()), site: Some("Figma".into()), image: Some(vec![0; 9]) }, ..saved("links") };
        let l = one(link);
        assert_eq!((l.kind.as_str(), l.url.as_deref(), l.title.as_deref(), l.site.as_deref(), l.image, l.meta.as_deref()), ("link", Some("https://www.figma.com/design/abc"), Some("Design"), Some("Figma"), Some("1009".to_string()), Some("Figma")));
        assert_eq!(one(Saved { text: Some("https://www.figma.com/x".into()), ..saved("links") }).meta.as_deref(), Some("figma.com"));

        let c = one(Saved { text: Some("#0a84ff".into()), ..saved("colors") });
        assert_eq!((c.kind.as_str(), c.color.as_deref(), c.meta.as_deref()), ("color", Some("#0A84FF"), Some("HEX")));
        assert_eq!(one(Saved { text: Some("rgb(48, 209, 88)".into()), ..saved("colors") }).color.as_deref(), Some("#30D158"));
    }

    fn page() -> Page {
        Page {
            items: vec![Saved { text: Some("hi".into()), app: Some("Chrome".into()), icon: Some(vec![1, 2, 3]), ..saved("text") }],
            total: 128,
            history: 128,
            scopes: vec![
                ScopeCount { scope: Scope::History, name: "History".into(), color: None, count: 128 },
                ScopeCount { scope: Scope::Pinned, name: "Pinned".into(), color: None, count: 6 },
                ScopeCount { scope: Scope::Collection(7), name: "Prompts".into(), color: Some("#0A84FF".into()), count: 12 },
            ],
            kinds: vec![(Kind::Text, 40)],
            apps: vec![AppCount { id: "chrome.exe".into(), name: "Chrome".into(), icon: Some(vec![1, 2, 3]), count: 30 }],
        }
    }

    #[test]
    fn views_say_when_empty() {
        let ui = Ui { layout: "strip".into(), ..Default::default() };
        let mut images = Images::new(&fake);
        assert_eq!(view(&Page::default(), &ui, 0, &mut images).empty.as_deref(), Some("Nothing copied yet"));
        let none = Page { history: 3, ..Default::default() };
        assert_eq!(view(&none, &ui, 0, &mut images).empty.as_deref(), Some("No matches"));
        let v = view(&page(), &ui, 0, &mut images);
        assert_eq!(v.empty, None);
        // The app icon and the item's icon are one image, listed once.
        assert_eq!(images.ids, [ImageId(1003)]);
    }

    /// The exact JSON of a view: every contract field, with its name and type.
    #[test]
    fn contract_shape() {
        let ui = Ui { query: Query { text: "fig".into(), scope: Scope::Collection(7), kind: Some(Kind::Text), app: None }, layout: "grid".into(), paused: false, undo: true, stack: Stack { active: true, remaining: 2 } };
        let v = view(&page(), &ui, 120_000, &mut Images::new(&fake));
        let json: Value = serde_json::from_str(&state_message(&v)).unwrap();
        assert_eq!(json["type"], "state");
        let view = &json["view"];
        let keys = |v: &Value| {
            let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
            k.sort();
            k
        };
        assert_eq!(keys(view), ["apps", "collection", "collections", "empty", "filter", "items", "layout", "paused", "query", "stack", "total", "types", "undo"]);
        assert_eq!(view["filter"], serde_json::json!({ "type": "text", "app": null }));
        assert_eq!(view["collection"], "c7");
        assert_eq!(view["layout"], "grid");
        assert_eq!(view["stack"], serde_json::json!({ "active": true, "remaining": 2 }));
        assert_eq!(view["collections"][0], serde_json::json!({ "id": "history", "name": "History", "count": 128, "color": null, "builtin": true }));
        assert_eq!(view["collections"][2], serde_json::json!({ "id": "c7", "name": "Prompts", "count": 12, "color": "#0A84FF", "builtin": false }));
        assert_eq!(view["types"][0], serde_json::json!({ "id": "text", "label": "Text", "count": 40 }));
        assert_eq!(view["apps"][0], serde_json::json!({ "id": "chrome.exe", "name": "Chrome", "icon": "1003", "count": 30 }));
        let item = &view["items"][0];
        assert_eq!(
            keys(item),
            ["age", "app", "collections", "color", "cut", "files", "icon", "id", "image", "kind", "lines", "meta", "missing", "ocr", "pinned", "rich", "site", "text", "title", "url"]
        );
        assert_eq!((item["id"].as_i64(), item["kind"].as_str(), item["text"].as_str(), item["age"].as_str(), item["icon"].as_str()), (Some(7), Some("text"), Some("hi"), Some("2m"), Some("1003")));
        // Round trip.
        let back: View = serde_json::from_value(view.clone()).unwrap();
        assert_eq!(back, v);
    }

    #[test]
    fn messages() {
        let target = Target { hwnd: 1234, focus: 5678, caret: Some([10, 20, 11, 38]) };
        let open: Value = serde_json::from_str(&open_message(&target, &View::default())).unwrap();
        assert_eq!(open["type"], "open");
        assert_eq!(open["target"], serde_json::json!({ "hwnd": 1234, "focus": 5678, "caret": [10, 20, 11, 38] }));
        assert!(open["view"].is_object());
        let open: Value = serde_json::from_str(&open_message(&Target { caret: None, ..target }, &View::default())).unwrap();
        assert_eq!(open["target"]["caret"], Value::Null);
        assert_eq!(serde_json::from_str::<Value>(&preview_message(12, Some("x"), None)).unwrap(), serde_json::json!({ "type": "preview", "id": 12, "text": "x", "image": null }));
        assert_eq!(close_message(), r#"{"type":"close"}"#);
        let files = vec!["C:\\a.txt".to_string()];
        assert_eq!(serde_json::from_str::<Value>(&export_message(3, None, None, Some(&files))).unwrap(), serde_json::json!({ "type": "export", "id": 3, "files": ["C:\\a.txt"] }));
        assert_eq!(serde_json::from_str::<Value>(&export_message(3, Some("t"), Some("9".into()), None)).unwrap(), serde_json::json!({ "type": "export", "id": 3, "text": "t", "png": "9" }));
        assert_eq!(cap_bytes("héllo", 2), "h");
        assert_eq!(cap_bytes("hi", 10), "hi");
    }
}
