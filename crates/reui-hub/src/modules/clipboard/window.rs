//! The clipboard window's messages: queries and layout, paste and copy (one item or
//! several, in a mode), pins, deletes and undo, collections, edits and OCR.

use serde_json::Value;
use tokio::time::Instant;

use super::os::{Command, Content};
use super::preview::prefix;
use super::store::{Kind, Query, Scope};
use super::settings::Settings;
use super::transform::Mode;
use super::{now_ms, ocr, App, Undo, UNDO};

impl App {
    pub(super) async fn on_window(&mut self, message: &str) {
        let Ok(m) = serde_json::from_str::<Value>(message) else {
            eprintln!("clipboard: window sent something that isn't JSON");
            return;
        };
        let now = now_ms();
        match m["type"].as_str().unwrap_or_default() {
            "opened" => {
                self.open = true;
                self.fetch_links();
            }
            "closed" => self.open = false,
            "query" => {
                self.query = Query {
                    text: m["query"].as_str().unwrap_or_default().to_string(),
                    scope: m["collection"].as_str().and_then(Scope::parse).unwrap_or_default(),
                    kind: m["filter"]["type"].as_str().and_then(Kind::parse),
                    app: m["filter"]["app"].as_str().filter(|a| !a.is_empty()).map(String::from),
                };
                self.send_state().await;
            }
            "layout" => {
                if let Some(layout) = m["layout"].as_str().filter(|l| matches!(*l, "strip" | "grid")) {
                    self.settings.layout = layout.to_string();
                    Settings::save_layout(&Settings::path(), layout);
                }
                self.send_state().await;
            }
            kind @ ("paste" | "copy") => {
                // The host hid the window (and, for a paste, gave the target its focus back).
                self.open = false;
                let mode = Mode::parse(m["mode"].as_str().unwrap_or_default());
                let Some((content, plain)) = self.compose(ids(&m), mode).await else { return };
                if kind == "copy" {
                    self.os.send(Command::Copy(content));
                } else if m["focused"].as_bool() == Some(false) {
                    self.os.send(Command::Copy(content));
                    self.press_ctrl_v();
                } else {
                    self.os.send(Command::Paste { content, plain, target: self.target.map(|t| t.hwnd) });
                }
            }
            "stack" => {
                self.open = false;
                self.start_stack(ids(&m)).await;
            }
            kind @ ("pin" | "unpin") => {
                if let Some(id) = m["id"].as_i64() {
                    let pinned = kind == "pin";
                    self.db.call(move |s| s.set_pinned(id, pinned)).await;
                }
                self.send_state().await;
            }
            "delete" => {
                let ids = ids(&m);
                self.finish_undo().await;
                let deleted = self.db.call(move |s| s.delete(&ids, now)).await.unwrap_or_default();
                if !deleted.is_empty() {
                    self.undo = Some((Undo::Deleted(deleted), Instant::now() + UNDO));
                }
                self.send_state().await;
            }
            "undo" => {
                if let Some((undo, _)) = self.undo.take() {
                    self.db
                        .call(move |s| match undo {
                            Undo::Deleted(ids) => s.restore(&ids),
                            Undo::Edited { old, new, fresh } => {
                                s.restore(&[old])?;
                                if fresh {
                                    s.remove(new)?;
                                }
                                Ok(())
                            }
                        })
                        .await;
                }
                self.send_state().await;
            }
            kind @ ("collect" | "uncollect") => {
                let (ids, add) = (ids(&m), kind == "collect");
                match m["collection"].as_str().and_then(Scope::parse) {
                    Some(Scope::Collection(c)) => _ = self.db.call(move |s| s.collect(&ids, c, add)).await,
                    Some(Scope::Pinned) => _ = self.db.call(move |s| ids.iter().try_for_each(|&id| s.set_pinned(id, add))).await,
                    _ => {}
                }
                self.send_state().await;
            }
            "collection.create" => {
                if let Some(name) = name(&m["name"]) {
                    let color = m["color"].as_str().map(String::from);
                    self.db.call(move |s| s.create_collection(&name, color.as_deref())).await;
                }
                self.send_state().await;
            }
            "collection.rename" => {
                if let (Some(id), Some(name)) = (collection_id(&m["id"]), name(&m["name"])) {
                    self.db.call(move |s| s.rename_collection(id, &name)).await;
                }
                self.send_state().await;
            }
            "collection.color" => {
                if let Some(id) = collection_id(&m["id"]) {
                    let color = m["color"].as_str().map(String::from);
                    self.db.call(move |s| s.color_collection(id, color.as_deref())).await;
                }
                self.send_state().await;
            }
            "collection.delete" => {
                if let Some(id) = collection_id(&m["id"]) {
                    self.db.call(move |s| s.delete_collection(id)).await;
                    if self.query.scope == Scope::Collection(id) {
                        self.query.scope = Scope::History;
                    }
                }
                self.send_state().await;
            }
            "collection.reorder" => {
                let order: Vec<i64> = m["ids"].as_array().map(|a| a.iter().filter_map(collection_id).collect()).unwrap_or_default();
                self.db.call(move |s| s.reorder_collections(&order)).await;
                self.send_state().await;
            }
            "edit" => {
                if let (Some(id), Some(text)) = (m["id"].as_i64(), m["text"].as_str().map(String::from)) {
                    self.finish_undo().await;
                    if let Some((new, fresh)) = self.db.call(move |s| s.edit(id, &text, now)).await.flatten() {
                        if new != id {
                            self.undo = Some((Undo::Edited { old: id, new, fresh }, Instant::now() + UNDO));
                        }
                    }
                }
                self.send_state().await;
            }
            "ocr" => {
                let Some(id) = m["id"].as_i64() else { return };
                let mut text = self.db.call(move |s| s.ocr_text(id)).await.flatten();
                if text.is_none() {
                    if let Some(png) = self.db.call(move |s| s.png(id)).await.flatten() {
                        text = ocr::recognize(png).await;
                        if let Some(found) = text.clone() {
                            self.db.call(move |s| s.set_ocr(id, &found)).await;
                        }
                    }
                }
                if let Some(text) = text.filter(|t| !t.trim().is_empty()) {
                    self.os.send(Command::Copy(Content::Text { text, html: None, rtf: None }));
                }
                self.send_state().await;
            }
            "pause" => self.set_paused(true).await,
            "resume" => self.set_paused(false).await,
            other => eprintln!("clipboard: unknown panel message \"{other}\""),
        }
    }

    /// What pasting `ids` puts on the clipboard (see [`compose`]); the items move to the top.
    async fn compose(&self, ids: Vec<i64>, mode: Mode) -> Option<(Content, bool)> {
        let now = now_ms();
        let contents = self
            .db
            .call(move |s| {
                let mut contents = Vec::new();
                for &id in &ids {
                    if let Some(content) = s.content(id)? {
                        s.touch(id, now)?;
                        contents.push(content);
                    }
                }
                Ok(contents)
            })
            .await?;
        compose(contents, mode)
    }
}

/// What pasting several items means: one item as it is (or as plain text in a `mode`);
/// several are their text joined by newlines, transformed per item; with no text among
/// them, the first one alone. `bool`: paste as plain text.
fn compose(contents: Vec<Content>, mode: Mode) -> Option<(Content, bool)> {
    let plain = |text: String| Content::Text { text, html: None, rtf: None };
    if contents.len() == 1 {
        let content = contents.into_iter().next()?;
        return Some(match content {
            Content::Text { text, html, .. } if mode != Mode::Normal => (plain(mode.apply(&text, html.as_deref())), true),
            content => (content, false),
        });
    }
    let texts: Vec<String> = contents
        .iter()
        .filter_map(|c| match c {
            Content::Text { text, html, .. } => Some(mode.apply(text, html.as_deref())),
            _ => None,
        })
        .collect();
    if texts.is_empty() {
        return contents.into_iter().next().map(|c| (c, false));
    }
    Some((plain(texts.join("\n")), true))
}

/// Item ids in a window message (`"ids": [12, 13]`).
fn ids(message: &Value) -> Vec<i64> {
    message["ids"].as_array().map(|a| a.iter().filter_map(Value::as_i64).collect()).unwrap_or_default()
}

/// A user collection's id: `"c7"` (as in views) or `7`.
fn collection_id(value: &Value) -> Option<i64> {
    match value {
        Value::String(s) => match Scope::parse(s)? {
            Scope::Collection(id) => Some(id),
            _ => None,
        },
        _ => value.as_i64(),
    }
}

/// A collection name: trimmed, not empty, at most 64 characters.
fn name(value: &Value) -> Option<String> {
    let name = value.as_str()?.trim();
    (!name.is_empty()).then(|| prefix(name, 64).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(t: &str, html: Option<&str>) -> Content {
        Content::Text { text: t.into(), html: html.map(String::from), rtf: None }
    }

    #[test]
    fn pasting_one_or_several() {
        let rich = text("Hello", Some("<!--StartFragment--><b>Hello</b><!--EndFragment-->"));
        // One item, as is: rich formats kept.
        assert_eq!(compose(vec![rich.clone()], Mode::Normal), Some((rich.clone(), false)));
        // In a mode: plain text, transformed.
        assert_eq!(compose(vec![rich.clone()], Mode::Upper), Some((text("HELLO", None), true)));
        assert_eq!(compose(vec![rich.clone()], Mode::Markdown), Some((text("**Hello**", None), true)));
        // Several: joined by newlines; non-text items left out.
        let image = Content::Image { png: vec![1], thumb: vec![1] };
        let files = Content::Files { paths: vec!["a".into()], cut: false };
        assert_eq!(compose(vec![text("a ", None), image.clone(), text(" b", None)], Mode::Trim), Some((text("a\nb", None), true)));
        assert_eq!(compose(vec![text("a", None), text("b", None)], Mode::Normal), Some((text("a\nb", None), true)));
        // No text among them: the first item alone, as is.
        assert_eq!(compose(vec![image.clone(), files.clone()], Mode::Normal), Some((image.clone(), false)));
        assert_eq!(compose(vec![files.clone()], Mode::Upper), Some((files, false)));
        assert_eq!(compose(Vec::new(), Mode::Normal), None);
    }

    #[test]
    fn window_message_fields() {
        let m: Value = serde_json::from_str(r#"{"type":"collect","ids":[3,"x",4],"collection":"c7","id":"c9","name":"  Prompts  "}"#).unwrap();
        assert_eq!(ids(&m), [3, 4]);
        assert_eq!(collection_id(&m["collection"]), Some(7));
        assert_eq!(collection_id(&m["id"]), Some(9));
        assert_eq!(collection_id(&Value::from(5)), Some(5));
        assert_eq!(collection_id(&Value::from("pinned")), None);
        assert_eq!(name(&m["name"]).as_deref(), Some("Prompts"));
        assert_eq!(name(&Value::from("   ")), None);
        assert_eq!(name(&Value::Null), None);
    }
}
