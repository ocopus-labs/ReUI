//! The database schema and its migrations (`pragma user_version`): v0 → v1 rebuilds and
//! rehashes the items table, v1 → v2 adds collections, the source exe, OCR text and search.

use rusqlite::{params, Connection, Result};

use super::items::{hash, text_category};
use super::query::reindex;
use super::{Kind, BYTES};

/// `pragma user_version`. 0 is the pre-migration schema, whose columns were added ad hoc;
/// 2 adds collections, the source exe, OCR text and the search index.
const SCHEMA_VERSION: i64 = 2;

const ITEMS_V1: &str = "
    id integer primary key,
    hash integer not null unique,
    kind text not null,
    category text not null,
    text text,
    html text,
    rtf blob,
    png blob,
    thumb blob,
    files text,
    cut integer not null default 0,
    lines integer not null default 0,
    source text,
    icon blob,
    width integer,
    height integer,
    link_title text,
    link_site text,
    link_image blob,
    link_fetched integer,
    pinned integer not null default 0,
    deleted_ms integer,
    bytes integer not null default 0,
    created_ms integer not null";

pub(super) fn migrate(db: &mut Connection) -> Result<()> {
    let version: i64 = db.query_row("pragma user_version", [], |r| r.get(0))?;
    if version >= SCHEMA_VERSION {
        return Ok(());
    }
    let tx = db.transaction()?;
    if version < 1 {
        to_v1(&tx)?;
    }
    if version < 2 {
        to_v2(&tx)?;
    }
    tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    tx.commit()
}

/// v0 → v1: rebuilds the table, rehashing every row with BLAKE3 (newest wins if two
/// collapse into one) and filling the new columns.
fn to_v1(db: &Connection) -> Result<()> {
    db.execute_batch(&format!("create table items_v1 ({ITEMS_V1});"))?;
    let exists = db.query_row("select count(*) from sqlite_master where type = 'table' and name = 'items'", [], |r| r.get::<_, i64>(0))? > 0;
    if exists {
        // v0 grew these columns one release at a time; add any an old file lacks so one select reads them all.
        let columns: Vec<String> = db.prepare("select name from pragma_table_info('items')")?.query_map([], |r| r.get(0))?.collect::<Result<_>>()?;
        for (name, kind) in [("icon", "blob"), ("width", "integer"), ("height", "integer"), ("link_title", "text"), ("link_site", "text"), ("link_image", "blob"), ("link_fetched", "integer")] {
            if !columns.iter().any(|c| c == name) {
                db.execute_batch(&format!("alter table items add column {name} {kind}"))?;
            }
        }
        let mut read = db.prepare(
            "select kind, text, png, thumb, files, source, icon, width, height, link_title, link_site, link_image, link_fetched, created_ms
             from items order by created_ms desc, id desc",
        )?;
        let mut write = db.prepare(
            "insert or ignore into items_v1 (hash, kind, category, text, png, thumb, files, lines, source, icon, width, height, link_title, link_site, link_image, link_fetched, created_ms)
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        )?;
        let mut rows = read.query([])?;
        while let Some(r) = rows.next()? {
            let kind: String = r.get(0)?;
            let text: Option<String> = r.get(1)?;
            let png: Option<Vec<u8>> = r.get(2)?;
            let files: Option<String> = r.get(4)?;
            let (hash, category, lines) = match kind.as_str() {
                "image" => (hash("image", png.as_deref().unwrap_or_default()), Kind::Images.name(), 0),
                "files" => (hash("files", files.as_deref().unwrap_or_default().as_bytes()), Kind::Files.name(), 0),
                _ => {
                    let t = text.as_deref().unwrap_or_default();
                    (hash("text", t.as_bytes()), text_category(t), t.trim().lines().count() as i64)
                }
            };
            write.execute(params![
                hash,
                kind,
                category,
                text,
                png,
                r.get::<_, Option<Vec<u8>>>(3)?,
                files,
                lines,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<Vec<u8>>>(6)?,
                r.get::<_, Option<i64>>(7)?,
                r.get::<_, Option<i64>>(8)?,
                r.get::<_, Option<String>>(9)?,
                r.get::<_, Option<String>>(10)?,
                r.get::<_, Option<Vec<u8>>>(11)?,
                r.get::<_, Option<i64>>(12)?,
                r.get::<_, i64>(13)?,
            ])?;
        }
        drop(rows);
        drop(read);
        db.execute_batch("drop table items;")?;
    }
    // `bytes` is filled by v2, once every column it counts exists.
    db.execute_batch("alter table items_v1 rename to items; create index items_recent on items (created_ms desc);")
}

/// v1 → v2: collections, the source exe (the app filter's id), OCR text, and the search
/// index, built for every existing item.
fn to_v2(db: &Connection) -> Result<()> {
    db.execute_batch(&format!(
        "alter table items add column exe text;
         alter table items add column ocr_text text;
         create table collections (id integer primary key, name text not null, color text, position integer not null default 0);
         create table item_collections (item_id integer not null, collection_id integer not null, primary key (item_id, collection_id)) without rowid;
         create index item_collections_by_collection on item_collections (collection_id);
         create virtual table items_fts using fts5(doc, tokenize = 'trigram');
         create trigger items_gone after delete on items begin
             delete from items_fts where rowid = old.id;
             delete from item_collections where item_id = old.id;
         end;
         update items set bytes = {BYTES};"
    ))?;
    let ids: Vec<i64> = db.prepare("select id from items")?.query_map([], |r| r.get(0))?.collect::<Result<_>>()?;
    for id in ids {
        reindex(db, id)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::tests::{ids, of_kind, text};
    use super::super::{Content, Query, Scope, Store};
    use super::*;

    /// A history written by the pre-migration hub: base table plus some of the ad-hoc columns.
    #[test]
    fn version_0_databases_upgrade() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "create table items (id integer primary key, hash integer not null unique, kind text not null, text text, png blob,
                 thumb blob, files text, source text, created_ms integer not null);
             create index items_recent on items (created_ms desc);
             alter table items add column icon blob;
             alter table items add column width integer;
             alter table items add column height integer;
             insert into items (hash, kind, text, source, created_ms) values (11, 'text', 'https://example.com/a', 'Chrome', 100);
             insert into items (hash, kind, text, created_ms) values (12, 'text', '#fff', 200);
             insert into items (hash, kind, png, thumb, width, height, created_ms) values (13, 'image', x'0102', x'01', 2, 1, 300);
             insert into items (hash, kind, files, created_ms) values (14, 'files', 'C:\\a.txt', 400);",
        )
        .unwrap();
        let mut s = Store::with(db).unwrap();
        let version: i64 = s.db.query_row("pragma user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        let page = s.page(&Query::default(), 60).unwrap();
        assert_eq!(page.items.len(), 4);
        assert_eq!(page.items[1].size, Some((2, 1)));
        assert_eq!(of_kind(&s, Kind::Links).len(), 1);
        assert_eq!(of_kind(&s, Kind::Colors).len(), 1);
        assert_eq!(ids(&s, "chrome", Scope::History).len(), 1);
        let bytes: i64 = s.db.query_row("select sum(bytes) from items", [], |r| r.get(0)).unwrap();
        assert!(bytes > 0);
        // Rehashed: copying the same text again dedupes against the migrated row.
        let link = of_kind(&s, Kind::Links)[0];
        assert_eq!(s.add(text("https://example.com/a"), 500).unwrap(), Some(link));
        assert_eq!(s.page(&Query::default(), 60).unwrap().items.len(), 4);
        // Reopening doesn't migrate again.
        let db = std::mem::replace(&mut s.db, Connection::open_in_memory().unwrap());
        assert_eq!(Store::with(db).unwrap().page(&Query::default(), 60).unwrap().items.len(), 4);
    }

    /// A history written by the v1 hub (`user_version` 1): gains collections, OCR and the index.
    #[test]
    fn version_1_databases_upgrade() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(&format!(
            "create table items ({ITEMS_V1});
             create index items_recent on items (created_ms desc);
             insert into items (hash, kind, category, text, html, source, pinned, created_ms) values (21, 'text', 'text', 'Hello World', '<b>Hello</b>', 'Word', 1, 100);
             insert into items (hash, kind, category, text, link_title, link_site, source, created_ms) values (22, 'text', 'links', 'https://figma.com/x', 'Design file', 'Figma', 'Chrome', 200);
             insert into items (hash, kind, category, png, thumb, width, height, created_ms) values (23, 'image', 'images', x'0102', x'01', 2, 1, 300);
             pragma user_version = 1;"
        ))
        .unwrap();
        let mut s = Store::with(db).unwrap();
        let version: i64 = s.db.query_row("pragma user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(version, 2);
        assert_eq!(ids(&s, "", Scope::History).len(), 3);
        assert_eq!(ids(&s, "", Scope::Pinned).len(), 1);
        // The index covers old rows: text, link title, app.
        assert_eq!(ids(&s, "hello", Scope::History).len(), 1);
        assert_eq!(ids(&s, "design", Scope::History).len(), 1);
        assert_eq!(ids(&s, "word", Scope::History).len(), 1);
        // Old rows have no exe: the app filter uses their display name.
        let page = s.page(&Query::default(), 60).unwrap();
        assert!(page.apps.iter().any(|a| a.id == "Chrome" && a.name == "Chrome"));
        assert_eq!(s.ocr_pending(10).unwrap().len(), 1);
        let c = s.create_collection("New", None).unwrap();
        let first = page.items[0].id;
        s.collect(&[first], c, true).unwrap();
        assert_eq!(ids(&s, "", Scope::Collection(c)), [first]);
        assert!(matches!(s.content(ids(&s, "hello", Scope::History)[0]).unwrap(), Some(Content::Text { html: Some(_), .. })));
    }
}
