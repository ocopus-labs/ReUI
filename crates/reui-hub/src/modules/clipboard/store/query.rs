//! Reading for the window: a page of items for a query with counts per facet, substring
//! search over the FTS5 trigram index, and the search document each item is indexed by.

use std::collections::HashMap;
use std::path::Path;

use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Result};

use super::{AppCount, Kind, LinkMeta, Page, Query, Saved, Scope, ScopeCount, Store};

/// Characters of an item's text that are searched (and read back for a card).
const SEARCH_CHARS: i64 = 20_000;

const COLUMNS: &str = "id, category, substr(text, 1, 20000), lines, html is not null or rtf is not null, thumb, files, cut, icon,
    width, height, link_title, link_site, link_image, link_fetched, pinned, created_ms, source, coalesce(ocr_text, '') != ''";

impl Store {
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
pub(super) fn reindex(db: &Connection, id: i64) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::super::tests::{captured, from, ids, of_kind, query, text};
    use super::super::Content;
    use super::*;

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
}
