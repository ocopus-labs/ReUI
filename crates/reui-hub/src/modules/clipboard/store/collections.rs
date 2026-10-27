//! Collections: the user's named, coloured, ordered groups of items (kept forever).

use rusqlite::{params, OptionalExtension, Result};

use super::Store;

impl Store {
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

#[cfg(test)]
mod tests {
    use super::super::Scope;
    use super::*;

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
}
