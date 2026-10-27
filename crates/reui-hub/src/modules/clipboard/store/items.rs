//! Saving and changing items: dedupe by content hash, the item and size caps, pins, soft
//! deletes and undo, edits as new versions, link and OCR results, retention.

use reui_protocol::Preview;
use rusqlite::{params, Connection, OptionalExtension, Result};

use super::query::reindex;
use super::{classify, Captured, Content, Kind, LinkMeta, Store, BYTES};

/// Kept forever: pinned, or in a collection.
const KEPT: &str = "(pinned = 1 or id in (select item_id from item_collections))";

impl Store {
    /// Saves a copy, or moves an identical earlier one to the top (keeping the richest
    /// formats seen). Returns its id, or `None` if it wasn't kept (empty, or over the cap).
    pub fn add(&mut self, captured: Captured, now: i64) -> Result<Option<i64>> {
        let Captured { content, source, exe, icon, size } = captured;
        let hash = content_hash(&content);
        let category = category_of(&content);
        let (kind, text, html, rtf, png, thumb, files, cut) = match content {
            Content::Text { text, html, rtf } => ("text", Some(text), html, rtf, None, None, None, false),
            Content::Image { png, thumb } => ("image", None, None, None, Some(png), Some(thumb), None, false),
            Content::Files { paths, cut } => ("files", None, None, None, None, None, Some(paths.join("\n")), cut),
        };
        if text.as_deref().is_some_and(|t| t.trim().is_empty()) || files.as_deref() == Some("") {
            return Ok(None);
        }
        let len = |b: Option<usize>| b.unwrap_or(0) as i64;
        let bytes = len(text.as_ref().map(String::len)) + len(html.as_ref().map(String::len)) + len(rtf.as_ref().map(Vec::len))
            + len(png.as_ref().map(Vec::len)) + len(thumb.as_ref().map(Vec::len)) + len(files.as_ref().map(String::len)) + len(icon.as_ref().map(Vec::len));
        if bytes > self.max_bytes {
            return Ok(None);
        }
        let lines = text.as_deref().map_or(0, |t| t.trim().lines().count() as i64);
        let (width, height) = size.unzip();
        let tx = self.db.transaction()?;
        let id: i64 = tx.query_row(
            "insert into items (hash, kind, category, text, html, rtf, png, thumb, files, cut, lines, source, exe, icon, width, height, created_ms)
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
             on conflict (hash) do update set created_ms = excluded.created_ms, deleted_ms = null, cut = excluded.cut,
                 html = coalesce(excluded.html, html), rtf = coalesce(excluded.rtf, rtf),
                 source = coalesce(excluded.source, source), exe = coalesce(excluded.exe, exe), icon = coalesce(excluded.icon, icon)
             returning id",
            params![hash, kind, category, text, html, rtf, png, thumb, files, cut, lines, source, exe, icon, width, height, now],
            |r| r.get(0),
        )?;
        tx.execute(&format!("update items set bytes = {BYTES} where id = ?1"), [id])?;
        reindex(&tx, id)?;
        prune(&tx, self.max_items, self.max_bytes)?;
        // Pruning can take the new item itself when pins fill the cap.
        let kept = tx.query_row("select 1 from items where id = ?1", [id], |_| Ok(())).optional()?.is_some();
        tx.commit()?;
        Ok(kept.then_some(id))
    }

    /// The `n`th most recent item (1 = newest), for Ctrl+Alt+1…9.
    pub fn recent(&self, n: u32) -> Result<Option<i64>> {
        self.db
            .query_row("select id from items where deleted_ms is null order by created_ms desc, id desc limit 1 offset ?1", [n.saturating_sub(1)], |r| r.get(0))
            .optional()
    }

    /// Everything needed to put an item back on the clipboard.
    pub fn content(&self, id: i64) -> Result<Option<Content>> {
        self.db
            .query_row("select kind, text, html, rtf, png, thumb, files, cut from items where id = ?1 and deleted_ms is null", [id], |r| {
                let kind: String = r.get(0)?;
                Ok(match kind.as_str() {
                    "image" => Content::Image { png: r.get::<_, Option<Vec<u8>>>(4)?.unwrap_or_default(), thumb: r.get::<_, Option<Vec<u8>>>(5)?.unwrap_or_default() },
                    "files" => Content::Files { paths: r.get::<_, Option<String>>(6)?.unwrap_or_default().lines().map(str::to_owned).collect(), cut: r.get(7)? },
                    _ => Content::Text { text: r.get::<_, Option<String>>(1)?.unwrap_or_default(), html: r.get(2)?, rtf: r.get(3)? },
                })
            })
            .optional()
    }

    pub fn png(&self, id: i64) -> Result<Option<Vec<u8>>> {
        Ok(self.db.query_row("select png from items where id = ?1 and deleted_ms is null", [id], |r| r.get::<_, Option<Vec<u8>>>(0)).optional()?.flatten())
    }

    /// Moves an item to the top (it was just pasted).
    pub fn touch(&self, id: i64, now: i64) -> Result<()> {
        self.db.execute("update items set created_ms = ?2 where id = ?1", params![id, now]).map(drop)
    }

    pub fn set_pinned(&self, id: i64, pinned: bool) -> Result<()> {
        self.db.execute("update items set pinned = ?2 where id = ?1", params![id, pinned]).map(drop)
    }

    /// Hides items until [`Store::restore`] or [`Store::purge_deleted`]. Returns those hidden.
    pub fn delete(&mut self, ids: &[i64], now: i64) -> Result<Vec<i64>> {
        let tx = self.db.transaction()?;
        let mut deleted = Vec::new();
        for &id in ids {
            if tx.execute("update items set deleted_ms = ?2 where id = ?1 and deleted_ms is null", params![id, now])? > 0 {
                deleted.push(id);
            }
        }
        tx.commit()?;
        Ok(deleted)
    }

    pub fn restore(&self, ids: &[i64]) -> Result<()> {
        for &id in ids {
            self.db.execute("update items set deleted_ms = null where id = ?1", [id])?;
        }
        Ok(())
    }

    /// Really removes soft-deleted items (the undo window is over).
    pub fn purge_deleted(&self) -> Result<()> {
        self.db.execute("delete from items where deleted_ms is not null", []).map(drop)
    }

    /// Removes one item for good (undoing an edit that made it).
    pub fn remove(&self, id: i64) -> Result<()> {
        self.db.execute("delete from items where id = ?1", [id]).map(drop)
    }

    /// Clear history: everything but pins and collections. Returns how many went.
    pub fn clear(&self) -> Result<usize> {
        self.db.execute(&format!("delete from items where not {KEPT}"), [])
    }

    /// Retention: unpinned, uncollected items copied before `cutoff_ms`.
    pub fn prune_before(&self, cutoff_ms: i64) -> Result<usize> {
        self.db.execute(&format!("delete from items where not {KEPT} and created_ms < ?1"), [cutoff_ms])
    }

    /// Records a link lookup, failed (`None`) ones too, with when it happened, so a failure
    /// is retried later rather than on every open.
    pub fn set_link(&self, id: i64, meta: Option<&LinkMeta>, now: i64) -> Result<()> {
        let meta = meta.cloned().unwrap_or_default();
        self.db.execute(
            "update items set link_title = ?2, link_site = ?3, link_image = ?4, link_fetched = ?5 where id = ?1",
            params![id, meta.title, meta.site, meta.image, now],
        )?;
        self.db.execute(&format!("update items set bytes = {BYTES} where id = ?1"), [id])?;
        reindex(&self.db, id)
    }

    /// Replaces a text item's content: the new text is saved as its own item (taking the
    /// old one's place, pin and collections) and the old one is soft-deleted, so undo is a
    /// restore. Returns the new id and whether it's a new row (not an existing copy).
    pub fn edit(&mut self, id: i64, text: &str, now: i64) -> Result<Option<(i64, bool)>> {
        if text.trim().is_empty() {
            return Ok(None);
        }
        let old = self
            .db
            .query_row("select kind, source, exe, icon, pinned from items where id = ?1 and deleted_ms is null", [id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, Option<Vec<u8>>>(3)?, r.get::<_, bool>(4)?))
            })
            .optional()?;
        let Some((kind, source, exe, icon, pinned)) = old.filter(|o| o.0 == "text") else { return Ok(None) };
        let hash = hash("text", text.as_bytes());
        let tx = self.db.transaction()?;
        let existing: Option<i64> = tx.query_row("select id from items where hash = ?1", [hash], |r| r.get(0)).optional()?;
        if existing == Some(id) {
            return Ok(Some((id, false)));
        }
        let new = match existing {
            Some(other) => {
                tx.execute("update items set created_ms = ?2, deleted_ms = null where id = ?1", params![other, now])?;
                other
            }
            None => tx.query_row(
                "insert into items (hash, kind, category, text, lines, source, exe, icon, created_ms) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) returning id",
                params![hash, kind, text_category(text), text, text.trim().lines().count() as i64, source, exe, icon, now],
                |r| r.get(0),
            )?,
        };
        tx.execute("insert or ignore into item_collections (item_id, collection_id) select ?1, collection_id from item_collections where item_id = ?2", params![new, id])?;
        if pinned {
            tx.execute("update items set pinned = 1 where id = ?1", [new])?;
        }
        tx.execute("update items set deleted_ms = ?2 where id = ?1", params![id, now])?;
        tx.execute(&format!("update items set bytes = {BYTES} where id = ?1"), [new])?;
        reindex(&tx, new)?;
        tx.commit()?;
        Ok(Some((new, existing.is_none())))
    }

    /// Stores what OCR found in an image ('' for nothing, so it isn't tried again).
    pub fn set_ocr(&self, id: i64, text: &str) -> Result<()> {
        self.db.execute("update items set ocr_text = ?2 where id = ?1", params![id, text])?;
        self.db.execute(&format!("update items set bytes = {BYTES} where id = ?1"), [id])?;
        reindex(&self.db, id)
    }

    /// The recognised text of an image; `None` if OCR hasn't run on it yet.
    pub fn ocr_text(&self, id: i64) -> Result<Option<String>> {
        Ok(self.db.query_row("select ocr_text from items where id = ?1", [id], |r| r.get::<_, Option<String>>(0)).optional()?.flatten())
    }

    /// Images OCR hasn't looked at yet, newest first.
    pub fn ocr_pending(&self, limit: usize) -> Result<Vec<i64>> {
        self.db
            .prepare_cached("select id from items where kind = 'image' and ocr_text is null and deleted_ms is null order by created_ms desc limit ?1")?
            .query_map([limit as i64], |r| r.get(0))?
            .collect()
    }
}

/// Oldest unpinned, uncollected items go first, until both caps hold.
fn prune(db: &Connection, max_items: i64, max_bytes: i64) -> Result<()> {
    let count: i64 = db.query_row("select count(*) from items", [], |r| r.get(0))?;
    if count > max_items {
        db.execute(&format!("delete from items where id in (select id from items where not {KEPT} order by created_ms asc, id asc limit ?1)"), [count - max_items])?;
    }
    let total: i64 = db.query_row("select coalesce(sum(bytes), 0) from items", [], |r| r.get(0))?;
    let mut over = total - max_bytes;
    if over <= 0 {
        return Ok(());
    }
    let mut oldest = db.prepare(&format!("select id, bytes from items where not {KEPT} order by created_ms asc, id asc"))?;
    let mut doomed = Vec::new();
    for row in oldest.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
        let (id, bytes) = row?;
        doomed.push(id);
        over -= bytes;
        if over <= 0 {
            break;
        }
    }
    for id in doomed {
        db.execute("delete from items where id = ?1", [id])?;
    }
    Ok(())
}

/// Stable across builds and versions: BLAKE3 of the kind and the content bytes. Rich
/// formats aren't included, so the same text with or without HTML is one item (E-C15).
pub fn content_hash(content: &Content) -> i64 {
    match content {
        Content::Text { text, .. } => hash("text", text.as_bytes()),
        Content::Image { png, .. } => hash("image", png),
        Content::Files { paths, .. } => hash("files", paths.join("\n").as_bytes()),
    }
}

pub(super) fn hash(kind: &str, bytes: &[u8]) -> i64 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(kind.as_bytes());
    hasher.update(&[0]);
    hasher.update(bytes);
    i64::from_le_bytes(hasher.finalize().as_bytes()[..8].try_into().unwrap())
}

fn category_of(content: &Content) -> &'static str {
    match content {
        Content::Text { text, .. } => text_category(text),
        Content::Image { .. } => Kind::Images.name(),
        Content::Files { .. } => Kind::Files.name(),
    }
}

pub(super) fn text_category(text: &str) -> &'static str {
    match classify(text) {
        Preview::Link { .. } => Kind::Links.name(),
        Preview::Color(_) => Kind::Colors.name(),
        Preview::Code(_) => Kind::Code.name(),
        _ => Kind::Text.name(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{captured, ids, text};
    use super::super::{Query, Scope};
    use super::*;

    #[test]
    fn hashes_are_stable_and_per_kind() {
        // Pinned value: changing the hash would break dedupe of every existing history.
        assert_eq!(content_hash(&Content::Text { text: "hello".into(), html: None, rtf: None }), hash("text", b"hello"));
        assert_eq!(hash("text", b"hello"), hash("text", b"hello"));
        assert_ne!(hash("text", b"hello"), hash("files", b"hello"));
        let rich = Content::Text { text: "hello".into(), html: Some("<b>hello</b>".into()), rtf: None };
        assert_eq!(content_hash(&rich), hash("text", b"hello"));
    }

    #[test]
    fn copying_again_moves_to_top_and_keeps_rich_formats() {
        let mut s = Store::in_memory().unwrap();
        let a = s.add(captured(Content::Text { text: "hello".into(), html: Some("<b>hello</b>".into()), rtf: None }), 1).unwrap().unwrap();
        let b = s.add(text("world"), 2).unwrap().unwrap();
        assert_eq!(ids(&s, "", Scope::History), [b, a]);
        assert_eq!(s.add(text("hello"), 3).unwrap(), Some(a));
        assert_eq!(ids(&s, "", Scope::History), [a, b]);
        assert!(matches!(s.content(a).unwrap(), Some(Content::Text { html: Some(_), .. })));
        let top = s.page(&Query::default(), 60).unwrap().items.remove(0);
        assert!(top.rich);
        // The second copy brought its app along.
        assert_eq!(top.app.as_deref(), Some("Notepad"));
        assert_eq!(s.add(text("   \n"), 4).unwrap(), None);
    }

    #[test]
    fn caps_drop_oldest_unkept_and_never_misreport() {
        let mut s = Store::in_memory().unwrap();
        s.max_items = 3;
        let first = s.add(text("one"), 1).unwrap().unwrap();
        s.set_pinned(first, true).unwrap();
        let second = s.add(text("two"), 2).unwrap().unwrap();
        let c = s.create_collection("Keep", None).unwrap();
        s.collect(&[second], c, true).unwrap();
        let third = s.add(text("three"), 3).unwrap().unwrap();
        for (i, t) in ["four", "five"].into_iter().enumerate() {
            s.add(text(t), 4 + i as i64).unwrap();
        }
        let left = ids(&s, "", Scope::History);
        assert_eq!(left.len(), 3);
        assert!(left.contains(&first) && left.contains(&second) && !left.contains(&third));

        let mut s = Store::in_memory().unwrap();
        s.max_bytes = 10;
        assert_eq!(s.add(text("this is far too long"), 1).unwrap(), None);
        let a = s.add(text("123456"), 2).unwrap().unwrap();
        s.add(text("abcdef"), 3).unwrap().unwrap();
        assert_eq!(ids(&s, "", Scope::History).len(), 1);
        assert!(!ids(&s, "", Scope::History).contains(&a));
        // Pins fill the cap: the new item itself is what goes, and add says so.
        let only = ids(&s, "", Scope::History)[0];
        s.set_pinned(only, true).unwrap();
        assert_eq!(s.add(text("xyz1234"), 4).unwrap(), None);
        assert_eq!(ids(&s, "", Scope::History), [only]);
    }

    #[test]
    fn pins_and_collections_survive_clear_and_retention() {
        let mut s = Store::in_memory().unwrap();
        let old = s.add(text("old"), 1_000).unwrap().unwrap();
        let pinned = s.add(text("pinned"), 1_000).unwrap().unwrap();
        let collected = s.add(text("collected"), 1_000).unwrap().unwrap();
        let new = s.add(text("new"), 9_000).unwrap().unwrap();
        s.set_pinned(pinned, true).unwrap();
        let c = s.create_collection("Prompts", Some("#0A84FF")).unwrap();
        s.collect(&[collected], c, true).unwrap();
        assert_eq!(s.prune_before(5_000).unwrap(), 1);
        assert!(!ids(&s, "", Scope::History).contains(&old));
        assert_eq!(s.clear().unwrap(), 1);
        assert_eq!(ids(&s, "", Scope::History), [collected, pinned]);
        assert!(!ids(&s, "", Scope::History).contains(&new));
        assert_eq!(ids(&s, "", Scope::Pinned), [pinned]);
        assert_eq!(ids(&s, "", Scope::Collection(c)), [collected]);
    }

    #[test]
    fn deletes_can_be_undone_until_purged() {
        let mut s = Store::in_memory().unwrap();
        let a = s.add(text("a"), 1).unwrap().unwrap();
        let b = s.add(text("b"), 2).unwrap().unwrap();
        assert_eq!(s.delete(&[a, 999], 3).unwrap(), [a]);
        assert_eq!(ids(&s, "", Scope::History), [b]);
        assert!(s.content(a).unwrap().is_none());
        s.restore(&[a]).unwrap();
        assert_eq!(ids(&s, "", Scope::History), [b, a]);
        s.delete(&[a, b], 4).unwrap();
        assert!(ids(&s, "", Scope::History).is_empty());
        s.purge_deleted().unwrap();
        s.restore(&[a]).unwrap();
        assert!(ids(&s, "", Scope::History).is_empty());
        // Gone from the index too.
        let fts: i64 = s.db.query_row("select count(*) from items_fts", [], |r| r.get(0)).unwrap();
        assert_eq!(fts, 0);
        // Copying a soft-deleted item again brings it back.
        let c = s.add(text("c"), 5).unwrap().unwrap();
        s.delete(&[c], 6).unwrap();
        assert_eq!(s.add(text("c"), 7).unwrap(), Some(c));
        assert_eq!(ids(&s, "", Scope::History), [c]);
    }

    #[test]
    fn edits_are_new_versions_that_can_be_undone() {
        let mut s = Store::in_memory().unwrap();
        let other = s.add(text("other"), 1).unwrap().unwrap();
        let old = s.add(text("draft"), 2).unwrap().unwrap();
        s.set_pinned(old, true).unwrap();
        let c = s.create_collection("P", None).unwrap();
        s.collect(&[old], c, true).unwrap();
        let (new, fresh) = s.edit(old, "final", 3).unwrap().unwrap();
        assert!(fresh && new != old);
        assert_eq!(ids(&s, "", Scope::History), [new, other]);
        assert_eq!(ids(&s, "", Scope::Pinned), [new]);
        assert_eq!(ids(&s, "", Scope::Collection(c)), [new]);
        assert_eq!(ids(&s, "final", Scope::History), [new]);
        // Undo: the old version back, the new one gone.
        s.restore(&[old]).unwrap();
        s.remove(new).unwrap();
        assert_eq!(ids(&s, "", Scope::History), [old, other]);
        // Editing into an existing text reuses it.
        assert_eq!(s.edit(old, "other", 4).unwrap(), Some((other, false)));
        assert_eq!(s.edit(other, "other", 5).unwrap(), Some((other, false)));
        assert_eq!(s.edit(other, "  ", 5).unwrap(), None);
        let image = s.add(captured(Content::Image { png: vec![1], thumb: vec![1] }), 6).unwrap().unwrap();
        assert_eq!(s.edit(image, "x", 7).unwrap(), None);
    }

    #[test]
    fn recent_items_count_from_the_newest() {
        let mut s = Store::in_memory().unwrap();
        let a = s.add(text("a"), 1).unwrap().unwrap();
        let b = s.add(text("b"), 2).unwrap().unwrap();
        let c = s.add(text("c"), 3).unwrap().unwrap();
        s.set_pinned(a, true).unwrap();
        assert_eq!([s.recent(1).unwrap(), s.recent(2).unwrap(), s.recent(3).unwrap(), s.recent(4).unwrap()], [Some(c), Some(b), Some(a), None]);
        // Pasting moves it to the top; deleted items don't count.
        s.touch(a, 4).unwrap();
        s.delete(&[c], 5).unwrap();
        assert_eq!([s.recent(1).unwrap(), s.recent(2).unwrap(), s.recent(3).unwrap()], [Some(a), Some(b), None]);
    }

    #[test]
    fn ocr_bookkeeping() {
        let mut s = Store::in_memory().unwrap();
        let a = s.add(captured(Content::Image { png: vec![1], thumb: vec![1] }), 1).unwrap().unwrap();
        let b = s.add(captured(Content::Image { png: vec![2], thumb: vec![2] }), 2).unwrap().unwrap();
        s.add(text("t"), 3).unwrap();
        assert_eq!(s.ocr_pending(10).unwrap(), [b, a]);
        assert_eq!(s.ocr_text(a).unwrap(), None);
        s.set_ocr(b, "").unwrap();
        assert_eq!(s.ocr_pending(10).unwrap(), [a]);
        assert_eq!(s.ocr_text(b).unwrap().as_deref(), Some(""));
        assert_eq!(s.png(a).unwrap(), Some(vec![1]));
    }

    #[test]
    fn link_lookups_are_recorded() {
        let mut s = Store::in_memory().unwrap();
        let id = s.add(text("https://example.com"), 1).unwrap().unwrap();
        assert_eq!(s.latest().unwrap().unwrap().link_fetched, None);
        s.set_link(id, None, 42).unwrap();
        let item = s.latest().unwrap().unwrap();
        assert_eq!((item.link_fetched, item.link), (Some(42), LinkMeta::default()));
    }
}
