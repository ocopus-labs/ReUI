//! Clipboard history storage: `%APPDATA%\ReUI\clipboard.db`, owned by one thread.
//!
//! Every SQLite call runs on the `clipboard-db` thread ([`Db`]), so a slow disk, a busy
//! lock (2 s timeout) or a big blob never stalls the hub's async thread. Errors are logged
//! and lose that one operation, never the module.
//!
//! Items are deduplicated by a BLAKE3 hash of their kind and content (stable across
//! versions, unlike `DefaultHasher`), so copying something again moves it to the top and
//! keeps the richest formats seen. Caps: 1 000 items and 500 MB of content, oldest
//! unpinned and uncollected first (collections and pins are kept forever). Deletes are
//! soft for a few seconds so they can be undone; everything really deleted is overwritten
//! on disk (`secure_delete`).
//!
//! Search runs on an FTS5 trigram index (`items_fts`), so any substring of three or more
//! characters is an index lookup; one lower-cased document per item holds its text, link
//! title and site, file paths, app and OCR text. Shorter terms scan those documents. The
//! index is written from Rust (Unicode lower-casing) and a trigger drops rows with their item.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc as std_mpsc;
use std::time::Duration;

use reui_protocol::Preview;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Result};

use super::clipboard::classify;
use super::clipboard_os::{Captured, Content};
use super::link::LinkMeta;

pub const MAX_ITEMS: i64 = 1000;
pub const MAX_BYTES: i64 = 500 * 1024 * 1024;
/// `pragma user_version`. 0 is the pre-migration schema, whose columns were added ad hoc;
/// 2 adds collections, the source exe, OCR text and the search index.
pub const SCHEMA_VERSION: i64 = 2;
/// Characters of an item's text that are searched (and read back for a card).
const SEARCH_CHARS: i64 = 20_000;

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

/// What an item costs on disk, for the size cap.
const BYTES: &str = "coalesce(length(cast(text as blob)), 0) + coalesce(length(cast(html as blob)), 0) + coalesce(length(rtf), 0)
    + coalesce(length(png), 0) + coalesce(length(thumb), 0) + coalesce(length(cast(files as blob)), 0)
    + coalesce(length(icon), 0) + coalesce(length(link_image), 0) + coalesce(length(cast(ocr_text as blob)), 0)";

/// Kept forever: pinned, or in a collection.
const KEPT: &str = "(pinned = 1 or id in (select item_id from item_collections))";

const COLUMNS: &str = "id, category, substr(text, 1, 20000), lines, html is not null or rtf is not null, thumb, files, cut, icon,
    width, height, link_title, link_site, link_image, link_fetched, pinned, created_ms, source, coalesce(ocr_text, '') != ''";

/// A type filter. Its name is also the stored `category` and the window's filter id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Text,
    Code,
    Links,
    Images,
    Colors,
    Files,
}

impl Kind {
    pub const ALL: [Kind; 6] = [Kind::Text, Kind::Code, Kind::Links, Kind::Images, Kind::Colors, Kind::Files];

    pub fn name(self) -> &'static str {
        match self {
            Kind::Text => "text",
            Kind::Code => "code",
            Kind::Links => "links",
            Kind::Images => "images",
            Kind::Colors => "colors",
            Kind::Files => "files",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Text => "Text",
            Kind::Code => "Code",
            Kind::Links => "Links",
            Kind::Images => "Images",
            Kind::Colors => "Colors",
            Kind::Files => "Files",
        }
    }

    /// An item's `kind` in the window contract (singular).
    pub fn item_kind(self) -> &'static str {
        match self {
            Kind::Text => "text",
            Kind::Code => "code",
            Kind::Links => "link",
            Kind::Images => "image",
            Kind::Colors => "color",
            Kind::Files => "files",
        }
    }

    pub fn parse(name: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.name() == name)
    }
}

/// Which items a view shows: everything, the pinned ones, or a collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum Scope {
    #[default]
    History,
    Pinned,
    Collection(i64),
}

impl Scope {
    /// `"history"`, `"pinned"`, `"c7"`.
    pub fn parse(id: &str) -> Option<Scope> {
        match id {
            "history" => Some(Scope::History),
            "pinned" => Some(Scope::Pinned),
            _ => id.strip_prefix('c')?.parse().ok().map(Scope::Collection),
        }
    }

    pub fn id(self) -> String {
        match self {
            Scope::History => "history".into(),
            Scope::Pinned => "pinned".into(),
            Scope::Collection(id) => format!("c{id}"),
        }
    }
}

/// What the window asks for.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Query {
    pub text: String,
    pub scope: Scope,
    pub kind: Option<Kind>,
    /// App id: the source exe ("chrome.exe"), or its display name for items saved before v2.
    pub app: Option<String>,
}

/// One saved item, as read back for display.
#[derive(Debug, Clone, Default)]
pub struct Saved {
    pub id: i64,
    /// A [`Kind`]'s name ("links", "code").
    pub category: String,
    /// The start of the text (see [`COLUMNS`]).
    pub text: Option<String>,
    /// Lines in the whole (trimmed) text.
    pub lines: u32,
    /// Has HTML or RTF besides the text.
    pub rich: bool,
    pub thumb: Option<Vec<u8>>,
    pub files: Vec<String>,
    pub cut: bool,
    /// None of the files exist any more.
    pub missing: bool,
    pub icon: Option<Vec<u8>>,
    pub size: Option<(u32, u32)>,
    pub link: LinkMeta,
    /// When the link was last looked up (ms).
    pub link_fetched: Option<i64>,
    pub pinned: bool,
    pub created_ms: i64,
    /// The source app's display name.
    pub app: Option<String>,
    /// An image with recognised text.
    pub ocr: bool,
    /// Collection ids it's in.
    pub collections: Vec<i64>,
}

/// A collection tab: built in (History, Pinned) or the user's own.
#[derive(Debug, Clone, PartialEq)]
pub struct ScopeCount {
    pub scope: Scope,
    pub name: String,
    pub color: Option<String>,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppCount {
    pub id: String,
    pub name: String,
    /// The newest icon seen for the app.
    pub icon: Option<Vec<u8>>,
    pub count: u32,
}

/// What the window shows for a query. Each facet's counts apply the query and the other
/// facets but not itself (the Links count is what you'd get by picking Links).
#[derive(Debug, Default)]
pub struct Page {
    /// Best match first, then newest.
    pub items: Vec<Saved>,
    /// Matches before the cap.
    pub total: u32,
    /// Items in the whole history (no query or filters).
    pub history: u32,
    pub scopes: Vec<ScopeCount>,
    /// Types with matches, in [`Kind::ALL`] order; the selected one always.
    pub kinds: Vec<(Kind, u32)>,
    /// Apps with matches, most first; the selected one always.
    pub apps: Vec<AppCount>,
}

pub struct Store {
    db: Connection,
    pub max_items: i64,
    pub max_bytes: i64,
}

impl Store {
    pub fn open() -> Result<Self> {
        let dir = std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("ReUI")).unwrap_or_else(|| PathBuf::from("."));
        _ = std::fs::create_dir_all(&dir);
        let path = dir.join("clipboard.db");
        match Self::open_at(&path) {
            Err(e) if matches!(e.sqlite_error_code(), Some(rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase)) => {
                eprintln!("clipboard: history is damaged ({e}); starting a new one, the old file is kept as clipboard.db.corrupt");
                _ = std::fs::rename(&path, dir.join("clipboard.db.corrupt"));
                for suffix in ["-wal", "-shm"] {
                    _ = std::fs::remove_file(dir.join(format!("clipboard.db{suffix}")));
                }
                Self::open_at(&path)
            }
            result => result,
        }
    }

    pub fn open_at(path: &Path) -> Result<Self> {
        Self::with(Connection::open(path)?)
    }

    #[cfg(test)]
    pub fn in_memory() -> Result<Self> {
        Self::with(Connection::open_in_memory()?)
    }

    /// Sets up and migrates an open database.
    pub fn with(mut db: Connection) -> Result<Self> {
        db.busy_timeout(Duration::from_secs(2))?;
        db.execute_batch("pragma journal_mode = wal; pragma secure_delete = on;")?;
        migrate(&mut db)?;
        // Soft deletes left by a crash inside the undo window.
        db.execute("delete from items where deleted_ms is not null", [])?;
        Ok(Self { db, max_items: MAX_ITEMS, max_bytes: MAX_BYTES })
    }

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

    /// The window's content for a query: up to `limit` items, with counts per facet.
    pub fn page(&self, query: &Query, limit: usize) -> Result<Page> {
        let matches = self.matches(&query.text)?;
        let metas: Vec<Meta> = self
            .db
            .prepare_cached("select id, category, coalesce(exe, source), source, pinned, created_ms from items where deleted_ms is null")?
            .query_map([], |r| Ok(Meta { id: r.get(0)?, kind: Kind::parse(&r.get::<_, String>(1)?), app: r.get(2)?, app_name: r.get(3)?, pinned: r.get(4)?, created: r.get(5)? }))?
            .collect::<Result<_>>()?;
        let members = self.memberships()?;
        let collections = self.collections()?;
        let no_members = Vec::new();

        let mut scope_counts: HashMap<Scope, u32> = HashMap::new();
        let mut kind_counts: HashMap<Kind, u32> = HashMap::new();
        // App id → (count, name of its newest item, newest time).
        let mut app_counts: HashMap<&str, (u32, Option<&str>, i64)> = HashMap::new();
        let mut hits: Vec<(u8, i64, i64)> = Vec::new();
        for meta in &metas {
            let tier = match &matches {
                None => 0,
                Some(m) => match m.get(&meta.id) {
                    Some(&tier) => tier,
                    None => continue,
                },
            };
            let mine = members.get(&meta.id).unwrap_or(&no_members);
            let in_scope = match query.scope {
                Scope::History => true,
                Scope::Pinned => meta.pinned,
                Scope::Collection(c) => mine.contains(&c),
            };
            let kind_ok = query.kind.is_none() || meta.kind == query.kind;
            let app_ok = query.app.is_none() || meta.app.as_deref() == query.app.as_deref();
            if kind_ok && app_ok {
                *scope_counts.entry(Scope::History).or_default() += 1;
                if meta.pinned {
                    *scope_counts.entry(Scope::Pinned).or_default() += 1;
                }
                for &c in mine {
                    *scope_counts.entry(Scope::Collection(c)).or_default() += 1;
                }
            }
            if in_scope && app_ok {
                if let Some(kind) = meta.kind {
                    *kind_counts.entry(kind).or_default() += 1;
                }
            }
            if in_scope && kind_ok {
                if let Some(app) = meta.app.as_deref() {
                    let entry = app_counts.entry(app).or_insert((0, None, i64::MIN));
                    entry.0 += 1;
                    if meta.created > entry.2 {
                        entry.1 = meta.app_name.as_deref();
                        entry.2 = meta.created;
                    }
                }
            }
            if in_scope && kind_ok && app_ok {
                hits.push((tier, meta.created, meta.id));
            }
        }
        hits.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then(b.2.cmp(&a.2)));
        let total = hits.len() as u32;
        let ids: Vec<i64> = hits.iter().take(limit).map(|h| h.2).collect();
        let mut items = self.saved(&ids)?;
        for item in &mut items {
            item.collections = members.get(&item.id).cloned().unwrap_or_default();
        }

        let count = |scope: Scope| scope_counts.get(&scope).copied().unwrap_or(0);
        let mut scopes = vec![
            ScopeCount { scope: Scope::History, name: "History".into(), color: None, count: count(Scope::History) },
            ScopeCount { scope: Scope::Pinned, name: "Pinned".into(), color: None, count: count(Scope::Pinned) },
        ];
        scopes.extend(collections.into_iter().map(|c| ScopeCount { scope: Scope::Collection(c.id), count: count(Scope::Collection(c.id)), name: c.name, color: c.color }));
        let kinds = Kind::ALL
            .into_iter()
            .map(|k| (k, kind_counts.get(&k).copied().unwrap_or(0)))
            .filter(|&(k, n)| n > 0 || query.kind == Some(k))
            .collect();
        let mut apps: Vec<AppCount> = app_counts
            .into_iter()
            .map(|(id, (count, name, _))| AppCount { id: id.to_string(), name: name.unwrap_or(id).to_string(), icon: None, count })
            .collect();
        if let Some(app) = query.app.as_deref().filter(|a| !apps.iter().any(|x| x.id == *a)) {
            apps.push(AppCount { id: app.to_string(), name: app.to_string(), icon: None, count: 0 });
        }
        apps.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        let mut icon = self.db.prepare_cached(
            "select icon from items where coalesce(exe, source) = ?1 and icon is not null and deleted_ms is null order by created_ms desc limit 1",
        )?;
        for app in &mut apps {
            app.icon = icon.query_row([&app.id], |r| r.get(0)).optional()?;
        }
        Ok(Page { items, total, history: metas.len() as u32, scopes, kinds, apps })
    }

    /// Item id → match tier (0: the whole query at a word start, 1: inside a word, 2: its
    /// words apart) for a search; `None` for no search (everything matches).
    fn matches(&self, query: &str) -> Result<Option<HashMap<i64, u8>>> {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return Ok(None);
        }
        let mut terms: Vec<&str> = query.split_whitespace().collect();
        terms.dedup();
        // Trigrams index substrings of three characters or more; shorter terms only filter.
        let phrases: Vec<String> = terms.iter().filter(|t| t.chars().count() >= 3).map(|t| format!("\"{}\"", t.replace('"', "\"\""))).collect();
        let mut conditions: Vec<String> = Vec::new();
        let mut values: Vec<String> = vec![query.clone(), phrases.join(" ")];
        if !phrases.is_empty() {
            conditions.push("items_fts match ?2".into());
        }
        for term in &terms {
            values.push(term.to_string());
            conditions.push(format!("instr(doc, ?{}) > 0", values.len()));
        }
        let sql = format!(
            r#"select rowid, case
                 when p > 0 and (p = 1 or substr(doc, p - 1, 1) in (' ', char(10), char(9), '/', '\', '.', '-', '_', '(', '[', '{{', '"', '''', ':', '#', '@', ',', ';', '<', '=')
                      or instr(doc, ' ' || ?1) > 0 or instr(doc, char(10) || ?1) > 0) then 0
                 when p > 0 then 1 else 2 end
               from (select rowid, doc, instr(doc, ?1) as p from items_fts where {})"#,
            conditions.join(" and ")
        );
        let mut statement = self.db.prepare_cached(&sql)?;
        let rows = statement.query_map(params_from_iter(values.iter()), |r| Ok((r.get::<_, i64>(0)?, r.get::<_, u8>(1)?)))?;
        Ok(Some(rows.collect::<Result<_>>()?))
    }

    /// Items by id, in that order (missing ones skipped).
    fn saved(&self, ids: &[i64]) -> Result<Vec<Saved>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let list = ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
        let mut by_id: HashMap<i64, Saved> =
            self.db.prepare(&format!("select {COLUMNS} from items where id in ({list})"))?.query_map([], read_saved)?.map(|s| s.map(|s| (s.id, s))).collect::<Result<_>>()?;
        Ok(ids.iter().filter_map(|id| by_id.remove(id)).collect())
    }

    fn memberships(&self) -> Result<HashMap<i64, Vec<i64>>> {
        let mut members: HashMap<i64, Vec<i64>> = HashMap::new();
        let mut statement = self.db.prepare_cached("select item_id, collection_id from item_collections")?;
        for row in statement.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
            let (item, collection) = row?;
            members.entry(item).or_default().push(collection);
        }
        Ok(members)
    }

    /// The newest item (the copy toast shows it).
    pub fn latest(&self) -> Result<Option<Saved>> {
        self.db
            .prepare_cached(&format!("select {COLUMNS} from items where deleted_ms is null order by created_ms desc, id desc limit 1"))?
            .query_row([], read_saved)
            .optional()
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

    pub fn collections(&self) -> Result<Vec<Collection>> {
        self.db
            .prepare_cached("select id, name, color from collections order by position, id")?
            .query_map([], |r| Ok(Collection { id: r.get(0)?, name: r.get(1)?, color: r.get(2)? }))?
            .collect()
    }

    /// A new collection at the end. Returns its id.
    pub fn create_collection(&self, name: &str, color: Option<&str>) -> Result<i64> {
        self.db.query_row(
            "insert into collections (name, color, position) values (?1, ?2, (select coalesce(max(position), 0) + 1 from collections)) returning id",
            params![name, color],
            |r| r.get(0),
        )
    }

    pub fn rename_collection(&self, id: i64, name: &str) -> Result<()> {
        self.db.execute("update collections set name = ?2 where id = ?1", params![id, name]).map(drop)
    }

    pub fn color_collection(&self, id: i64, color: Option<&str>) -> Result<()> {
        self.db.execute("update collections set color = ?2 where id = ?1", params![id, color]).map(drop)
    }

    /// The items stay in History.
    pub fn delete_collection(&mut self, id: i64) -> Result<()> {
        let tx = self.db.transaction()?;
        tx.execute("delete from item_collections where collection_id = ?1", [id])?;
        tx.execute("delete from collections where id = ?1", [id])?;
        tx.commit()
    }

    /// Collections in this order; any not listed keep their place after them.
    pub fn reorder_collections(&mut self, ids: &[i64]) -> Result<()> {
        let tx = self.db.transaction()?;
        tx.execute("update collections set position = position + ?1", [ids.len() as i64 + 1])?;
        for (i, id) in ids.iter().enumerate() {
            tx.execute("update collections set position = ?2 where id = ?1", params![id, i as i64])?;
        }
        tx.commit()
    }

    /// Adds items to (or with `add` false, removes them from) a collection.
    pub fn collect(&mut self, ids: &[i64], collection: i64, add: bool) -> Result<()> {
        let tx = self.db.transaction()?;
        let exists = tx.query_row("select 1 from collections where id = ?1", [collection], |_| Ok(())).optional()?.is_some();
        for &id in ids {
            if !add {
                tx.execute("delete from item_collections where item_id = ?1 and collection_id = ?2", params![id, collection])?;
            } else if exists {
                tx.execute("insert or ignore into item_collections (item_id, collection_id) select id, ?2 from items where id = ?1", params![id, collection])?;
            }
        }
        tx.commit()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Collection {
    pub id: i64,
    pub name: String,
    pub color: Option<String>,
}

/// The few columns facets need, for every item.
struct Meta {
    id: i64,
    kind: Option<Kind>,
    app: Option<String>,
    app_name: Option<String>,
    pinned: bool,
    created: i64,
}

/// Rewrites an item's search document (or drops it, if the item is gone).
fn reindex(db: &Connection, id: i64) -> Result<()> {
    let doc = db
        .query_row(
            "select substr(text, 1, ?2), link_title, link_site, files, source, exe, ocr_text from items where id = ?1",
            params![id, SEARCH_CHARS],
            |r| {
                let parts: Vec<Option<String>> = (0..7).map(|i| r.get(i)).collect::<Result<_>>()?;
                Ok(parts.into_iter().flatten().filter(|p| !p.is_empty()).collect::<Vec<_>>().join("\n").to_lowercase())
            },
        )
        .optional()?;
    db.execute("delete from items_fts where rowid = ?1", [id])?;
    if let Some(doc) = doc {
        db.execute("insert into items_fts (rowid, doc) values (?1, ?2)", params![id, doc])?;
    }
    Ok(())
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

fn read_saved(r: &rusqlite::Row) -> Result<Saved> {
    let (width, height): (Option<u32>, Option<u32>) = (r.get(9)?, r.get(10)?);
    let files: Vec<String> = r.get::<_, Option<String>>(6)?.map(|f| f.lines().map(str::to_owned).collect()).unwrap_or_default();
    // A cheap stat per path; this runs on the DB thread, so a slow share stalls nothing.
    let missing = !files.is_empty() && !files.iter().any(|f| Path::new(f).exists());
    Ok(Saved {
        id: r.get(0)?,
        category: r.get(1)?,
        text: r.get(2)?,
        lines: r.get(3)?,
        rich: r.get(4)?,
        thumb: r.get(5)?,
        files,
        cut: r.get(7)?,
        missing,
        icon: r.get(8)?,
        size: width.zip(height),
        link: LinkMeta { title: r.get(11)?, site: r.get(12)?, image: r.get(13)? },
        link_fetched: r.get(14)?,
        pinned: r.get(15)?,
        created_ms: r.get(16)?,
        app: r.get(17)?,
        ocr: r.get(18)?,
        collections: Vec::new(),
    })
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

fn hash(kind: &str, bytes: &[u8]) -> i64 {
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

fn text_category(text: &str) -> &'static str {
    match classify(text) {
        Preview::Link { .. } => Kind::Links.name(),
        Preview::Color(_) => Kind::Colors.name(),
        Preview::Code(_) => Kind::Code.name(),
        _ => Kind::Text.name(),
    }
}

fn migrate(db: &mut Connection) -> Result<()> {
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

type Job = Box<dyn FnOnce(&mut Store) + Send>;

/// Handle to the DB thread. Calls queue in order; the thread ends when every handle is dropped.
#[derive(Clone)]
pub struct Db {
    jobs: std_mpsc::Sender<Job>,
}

impl Db {
    /// Opens the store on its own thread; `None` if it can't be opened.
    pub async fn start(open: impl FnOnce() -> Result<Store> + Send + 'static) -> Option<Db> {
        let (jobs, queue) = std_mpsc::channel::<Job>();
        let (ready_tx, ready) = tokio::sync::oneshot::channel();
        std::thread::Builder::new()
            .name("clipboard-db".into())
            .spawn(move || {
                let mut store = match open() {
                    Ok(store) => store,
                    Err(e) => {
                        eprintln!("clipboard: can't open history: {e}");
                        _ = ready_tx.send(false);
                        return;
                    }
                };
                _ = ready_tx.send(true);
                // A panic in one operation must not end history: this thread is the only
                // way to the database, and losing it would silently stop every save.
                for job in queue {
                    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job(&mut store))).is_err() {
                        eprintln!("clipboard: a history operation failed unexpectedly; skipped it");
                    }
                }
            })
            .ok()?;
        ready.await.unwrap_or(false).then_some(Db { jobs })
    }

    /// Runs `f` on the DB thread. Errors are logged there and come back as `None`.
    pub async fn call<T: Send + 'static>(&self, f: impl FnOnce(&mut Store) -> Result<T> + Send + 'static) -> Option<T> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let job: Job = Box::new(move |store| _ = tx.send(f(store).map_err(|e| eprintln!("clipboard: {e}")).ok()));
        self.jobs.send(job).ok()?;
        rx.await.ok().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(t: &str) -> Captured {
        Captured { content: Content::Text { text: t.into(), html: None, rtf: None }, source: Some("Notepad".into()), exe: Some("notepad.exe".into()), icon: None, size: None }
    }

    fn from(t: &str, app: &str) -> Captured {
        Captured { content: Content::Text { text: t.into(), html: None, rtf: None }, source: Some(app.into()), exe: Some(format!("{}.exe", app.to_lowercase())), icon: Some(app.as_bytes().to_vec()), size: None }
    }

    fn captured(content: Content) -> Captured {
        Captured { content, source: None, exe: None, icon: None, size: None }
    }

    fn query(text: &str, scope: Scope) -> Query {
        Query { text: text.into(), scope, ..Default::default() }
    }

    fn ids(store: &Store, text: &str, scope: Scope) -> Vec<i64> {
        store.page(&query(text, scope), 60).unwrap().items.iter().map(|s| s.id).collect()
    }

    fn of_kind(store: &Store, kind: Kind) -> Vec<i64> {
        store.page(&Query { kind: Some(kind), ..Default::default() }, 60).unwrap().items.iter().map(|s| s.id).collect()
    }

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
    fn search_and_type_filters() {
        let mut s = Store::in_memory().unwrap();
        let link = s.add(text("https://www.figma.com/design/abc"), 1).unwrap().unwrap();
        let code = s.add(text("fn main() {\n    run();\n}"), 2).unwrap().unwrap();
        let color = s.add(text("#0A84FF"), 3).unwrap().unwrap();
        let files = s.add(captured(Content::Files { paths: vec![r"C:\nowhere\Report_2026.pdf".into()], cut: true }), 4).unwrap().unwrap();
        let image = s.add(captured(Content::Image { png: vec![1, 2, 3], thumb: vec![1] }), 5).unwrap().unwrap();
        let plain = s.add(text("100% done_now"), 6).unwrap().unwrap();
        s.set_link(link, Some(&LinkMeta { title: Some("Design file".into()), site: Some("Figma".into()), image: None }), 7).unwrap();
        s.set_ocr(image, "Quarterly Revenue chart").unwrap();
        s.set_pinned(code, true).unwrap();

        assert_eq!(ids(&s, "", Scope::History), [plain, image, files, color, code, link]);
        assert_eq!(ids(&s, "FIGMA", Scope::History), [link]);
        assert_eq!(ids(&s, "design FILE", Scope::History), [link]);
        assert_eq!(ids(&s, "report_2026", Scope::History), [files]);
        assert_eq!(ids(&s, "notepad", Scope::History), [plain, color, code, link]);
        // Text inside a screenshot.
        assert_eq!(ids(&s, "revenue", Scope::History), [image]);
        // Wildcards are literal; short terms work too.
        assert_eq!(ids(&s, "100%", Scope::History), [plain]);
        assert_eq!(ids(&s, "e_n", Scope::History), [plain]);
        assert_eq!(ids(&s, "%", Scope::History), [plain]);
        assert_eq!(ids(&s, "\"", Scope::History), Vec::<i64>::new());
        // Every word must match, anywhere.
        assert_eq!(ids(&s, "figma abc", Scope::History), [link]);
        assert!(ids(&s, "figma zzz", Scope::History).is_empty());

        assert_eq!(of_kind(&s, Kind::Links), [link]);
        assert_eq!(of_kind(&s, Kind::Code), [code]);
        assert_eq!(of_kind(&s, Kind::Colors), [color]);
        assert_eq!(of_kind(&s, Kind::Files), [files]);
        assert_eq!(of_kind(&s, Kind::Images), [image]);
        assert_eq!(of_kind(&s, Kind::Text), [plain]);
        assert_eq!(ids(&s, "", Scope::Pinned), [code]);

        let f = s.page(&Query { kind: Some(Kind::Files), ..Default::default() }, 60).unwrap().items.remove(0);
        assert!(f.cut && f.missing);
        let i = s.page(&Query { kind: Some(Kind::Images), ..Default::default() }, 60).unwrap().items.remove(0);
        assert!(i.ocr);
        assert_eq!(s.page(&Query::default(), 2).unwrap().items.len(), 2);
    }

    #[test]
    fn ranking_puts_word_starts_first_then_newest() {
        let mut s = Store::in_memory().unwrap();
        let inside = s.add(text("the spreadsheet"), 1).unwrap().unwrap();
        let older_start = s.add(text("sheet music"), 2).unwrap().unwrap();
        let apart = s.add(text("sheep ate it"), 3).unwrap().unwrap();
        let newer_start = s.add(text("my sheet/notes"), 4).unwrap().unwrap();
        let path = s.add(text(r"C:\docs\sheets.txt"), 5).unwrap().unwrap();
        assert_eq!(ids(&s, "sheet", Scope::History), [path, newer_start, older_start, inside]);
        // Two words: the phrase first, words apart after.
        let phrase = s.add(text("ate sheep"), 6).unwrap().unwrap();
        assert_eq!(ids(&s, "sheep ate", Scope::History), [apart, phrase]);
        assert_eq!(ids(&s, "ate sheep", Scope::History), [phrase, apart]);
    }

    #[test]
    fn counts_per_collection_type_and_app() {
        let mut s = Store::in_memory().unwrap();
        let a = s.add(from("alpha note", "Chrome"), 1).unwrap().unwrap();
        let b = s.add(from("https://alpha.dev", "Chrome"), 2).unwrap().unwrap();
        let c = s.add(from("beta note", "Slack"), 3).unwrap().unwrap();
        let d = s.add(from("alpha again", "Slack"), 4).unwrap().unwrap();
        s.set_pinned(a, true).unwrap();
        let prompts = s.create_collection("Prompts", Some("#0A84FF")).unwrap();
        let empty = s.create_collection("Empty", None).unwrap();
        s.collect(&[a, b, c, 12345], prompts, true).unwrap();

        let page = s.page(&query("alpha", Scope::History), 60).unwrap();
        assert_eq!(page.items.iter().map(|i| i.id).collect::<Vec<_>>(), [d, b, a]);
        assert_eq!((page.total, page.history), (3, 4));
        let scopes: Vec<(Scope, &str, u32)> = page.scopes.iter().map(|x| (x.scope, x.name.as_str(), x.count)).collect();
        assert_eq!(scopes, [(Scope::History, "History", 3), (Scope::Pinned, "Pinned", 1), (Scope::Collection(prompts), "Prompts", 2), (Scope::Collection(empty), "Empty", 0)]);
        assert_eq!(page.scopes[2].color.as_deref(), Some("#0A84FF"));
        assert_eq!(page.kinds, [(Kind::Text, 2), (Kind::Links, 1)]);
        let apps: Vec<(&str, &str, u32, bool)> = page.apps.iter().map(|x| (x.id.as_str(), x.name.as_str(), x.count, x.icon.is_some())).collect();
        assert_eq!(apps, [("chrome.exe", "Chrome", 2, true), ("slack.exe", "Slack", 1, true)]);
        assert_eq!(page.items[1].collections, [prompts]);

        // In a collection, filtered by app: facets apply everything but themselves.
        let q = Query { text: "alpha".into(), scope: Scope::Collection(prompts), kind: None, app: Some("chrome.exe".into()) };
        let page = s.page(&q, 60).unwrap();
        assert_eq!(page.items.iter().map(|i| i.id).collect::<Vec<_>>(), [b, a]);
        assert_eq!(page.scopes.iter().map(|x| x.count).collect::<Vec<_>>(), [2, 1, 2, 0]);
        assert_eq!(page.apps.iter().map(|x| (x.id.as_str(), x.count)).collect::<Vec<_>>(), [("chrome.exe", 2)]);
        // A selected type with no matches stays listed.
        let q = Query { kind: Some(Kind::Files), ..q };
        let page = s.page(&q, 60).unwrap();
        assert_eq!((page.total, page.kinds.clone()), (0, vec![(Kind::Text, 1), (Kind::Links, 1), (Kind::Files, 0)]));

        s.collect(&[b], prompts, false).unwrap();
        assert_eq!(ids(&s, "", Scope::Collection(prompts)), [c, a]);
        s.delete_collection(prompts).unwrap();
        assert_eq!(ids(&s, "", Scope::History).len(), 4);
        assert_eq!(s.collections().unwrap().len(), 1);
    }

    #[test]
    fn collections_are_managed() {
        let mut s = Store::in_memory().unwrap();
        let a = s.create_collection("A", None).unwrap();
        let b = s.create_collection("B", Some("#FF0000")).unwrap();
        let c = s.create_collection("C", None).unwrap();
        s.rename_collection(a, "Prompts").unwrap();
        s.color_collection(b, None).unwrap();
        s.reorder_collections(&[c, a]).unwrap();
        let names: Vec<(i64, String, Option<String>)> = s.collections().unwrap().into_iter().map(|x| (x.id, x.name, x.color)).collect();
        assert_eq!(names, [(c, "C".into(), None), (a, "Prompts".into(), None), (b, "B".into(), None)]);
        assert_eq!(Scope::parse("c12"), Some(Scope::Collection(12)));
        assert_eq!(Scope::parse("pinned"), Some(Scope::Pinned));
        assert_eq!(Scope::parse("cx"), None);
        assert_eq!(Scope::Collection(7).id(), "c7");
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
    fn search_over_a_full_history_is_fast() {
        let mut s = Store::in_memory().unwrap();
        for i in 0..1000 {
            s.add(text(&format!("item {i} with some ordinary text https://example.com/{i}\n{}", "lorem ipsum ".repeat(40))), i).unwrap();
        }
        for q in ["ipsum 99", "item 99 with", "zz", "lorem"] {
            let start = std::time::Instant::now();
            s.page(&query(q, Scope::History), 60).unwrap();
            // Generous for debug builds; release is far below the 16 ms budget.
            assert!(start.elapsed() < Duration::from_millis(150), "{q}: {:?}", start.elapsed());
        }
        assert_eq!(s.page(&query("ipsum 99", Scope::History), 60).unwrap().total, 19);
        assert_eq!(s.page(&query("item 99 with", Scope::History), 60).unwrap().items[0].text.as_deref().map(|t| t.starts_with("item 99 ")), Some(true));
    }

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

    #[test]
    fn link_lookups_are_recorded() {
        let mut s = Store::in_memory().unwrap();
        let id = s.add(text("https://example.com"), 1).unwrap().unwrap();
        assert_eq!(s.latest().unwrap().unwrap().link_fetched, None);
        s.set_link(id, None, 42).unwrap();
        let item = s.latest().unwrap().unwrap();
        assert_eq!((item.link_fetched, item.link), (Some(42), LinkMeta::default()));
    }

    #[test]
    fn a_file_database_round_trips() {
        let path = std::env::temp_dir().join(format!("reui-clipboard-test-{}.db", std::process::id()));
        {
            let mut s = Store::open_at(&path).unwrap();
            s.add(text("kept"), 1).unwrap();
        }
        assert_eq!(Store::open_at(&path).unwrap().page(&query("kept", Scope::History), 60).unwrap().items.len(), 1);
        for suffix in ["", "-wal", "-shm"] {
            _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }

    #[tokio::test]
    async fn the_db_thread_runs_calls() {
        let db = Db::start(Store::in_memory).await.unwrap();
        let id = db.call(|s| s.add(text("x"), 1)).await.flatten();
        assert!(id.is_some());
        assert_eq!(db.call(|s| s.page(&Query::default(), 60)).await.unwrap().items.len(), 1);
        assert!(Db::start(|| Err(rusqlite::Error::InvalidQuery)).await.is_none());
    }
}
