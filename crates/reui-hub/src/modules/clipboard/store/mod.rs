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
//!
//! Files: `schema.rs` (migrations), `items.rs` (saving and changing items), `query.rs`
//! (the window's page and search), `collections.rs` and `db.rs` (the DB thread).

mod collections;
mod db;
mod items;
mod query;
mod schema;

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, Result};

use super::preview::classify;
use super::link::LinkMeta;
use super::os::{Captured, Content};

pub use self::db::Db;
use self::schema::migrate;

pub const MAX_ITEMS: i64 = 1000;
pub const MAX_BYTES: i64 = 500 * 1024 * 1024;

/// What an item costs on disk, for the size cap.
const BYTES: &str = "coalesce(length(cast(text as blob)), 0) + coalesce(length(cast(html as blob)), 0) + coalesce(length(rtf), 0)
    + coalesce(length(png), 0) + coalesce(length(thumb), 0) + coalesce(length(cast(files as blob)), 0)
    + coalesce(length(icon), 0) + coalesce(length(link_image), 0) + coalesce(length(cast(ocr_text as blob)), 0)";

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
    /// The start of the text (see `COLUMNS` in `query.rs`).
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
        let dir = crate::paths::config();
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
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn text(t: &str) -> Captured {
        Captured { content: Content::Text { text: t.into(), html: None, rtf: None }, source: Some("Notepad".into()), exe: Some("notepad.exe".into()), icon: None, size: None }
    }

    pub(super) fn from(t: &str, app: &str) -> Captured {
        Captured { content: Content::Text { text: t.into(), html: None, rtf: None }, source: Some(app.into()), exe: Some(format!("{}.exe", app.to_lowercase())), icon: Some(app.as_bytes().to_vec()), size: None }
    }

    pub(super) fn captured(content: Content) -> Captured {
        Captured { content, source: None, exe: None, icon: None, size: None }
    }

    pub(super) fn query(text: &str, scope: Scope) -> Query {
        Query { text: text.into(), scope, ..Default::default() }
    }

    pub(super) fn ids(store: &Store, text: &str, scope: Scope) -> Vec<i64> {
        store.page(&query(text, scope), 60).unwrap().items.iter().map(|s| s.id).collect()
    }

    pub(super) fn of_kind(store: &Store, kind: Kind) -> Vec<i64> {
        store.page(&Query { kind: Some(kind), ..Default::default() }, 60).unwrap().items.iter().map(|s| s.id).collect()
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
}
