//! Favorites (watchlist) data access layer.
//!
//! Provides CRUD + listing operations. All methods are synchronous because
//! SQLite calls are extremely fast. Business-level validation (e.g. whether
//! a stock code is well-formed) is left to the caller (`Symbol::new`).

use rusqlite::{params, OptionalExtension};

use crate::model::{Favorite, Market};

use super::db::Database;
use super::StorageResult;

/// CRUD wrapper around the favorites table.
pub struct FavoriteStore<'a> {
    db: &'a Database,
}

impl<'a> FavoriteStore<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Insert or update a favorite.
    ///
    /// When a row with the same (code, market) already exists, the note and
    /// timestamps are merged. Returns whether the operation affected a row.
    pub fn add(&self, fav: &Favorite) -> StorageResult<()> {
        self.db.conn().execute(
            "INSERT INTO favorites (code, market, note, added_at, sort_order)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(code, market) DO UPDATE SET
                 note       = excluded.note,
                 added_at   = excluded.added_at,
                 sort_order = excluded.sort_order",
            params![
                fav.code,
                fav.market.code(),
                fav.note,
                fav.added_at,
                fav.sort_order,
            ],
        )?;
        Ok(())
    }

    /// Delete a favorite by (code, market). Returns whether a row was removed.
    pub fn remove(&self, code: &str, market: Market) -> StorageResult<bool> {
        let n = self.db.conn().execute(
            "DELETE FROM favorites WHERE code = ?1 AND market = ?2",
            params![code, market.code()],
        )?;
        Ok(n > 0)
    }

    /// Get a single favorite by (code, market).
    pub fn get(&self, code: &str, market: Market) -> StorageResult<Option<Favorite>> {
        let row = self
            .db
            .conn()
            .query_row(
                "SELECT code, market, note, added_at, sort_order
                 FROM favorites WHERE code = ?1 AND market = ?2",
                params![code, market.code()],
                row_to_favorite,
            )
            .optional()?;
        Ok(row)
    }

    /// Whether (code, market) exists in the favorites.
    pub fn contains(&self, code: &str, market: Market) -> StorageResult<bool> {
        let n: i64 = self.db.conn().query_row(
            "SELECT COUNT(*) FROM favorites WHERE code = ?1 AND market = ?2",
            params![code, market.code()],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    /// List all favorites, ordered by sort_order ASC then added_at ASC.
    pub fn list(&self) -> StorageResult<Vec<Favorite>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT code, market, note, added_at, sort_order
             FROM favorites
             ORDER BY sort_order ASC, added_at ASC",
        )?;
        let rows = stmt.query_map([], row_to_favorite)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Update the note for a single favorite. Returns whether a row was changed.
    pub fn update_note(
        &self,
        code: &str,
        market: Market,
        note: Option<&str>,
    ) -> StorageResult<bool> {
        let n = self.db.conn().execute(
            "UPDATE favorites SET note = ?3 WHERE code = ?1 AND market = ?2",
            params![code, market.code(), note],
        )?;
        Ok(n > 0)
    }

    /// Count of favorites.
    pub fn count(&self) -> StorageResult<i64> {
        let n: i64 = self
            .db
            .conn()
            .query_row("SELECT COUNT(*) FROM favorites", [], |r| r.get(0))?;
        Ok(n)
    }
}

/// Map a single SQL row into a `Favorite`.
fn row_to_favorite(row: &rusqlite::Row<'_>) -> rusqlite::Result<Favorite> {
    let code: String = row.get(0)?;
    let market_str: String = row.get(1)?;
    let note: Option<String> = row.get(2)?;
    let added_at: i64 = row.get(3)?;
    let sort_order: i64 = row.get(4)?;

    let market = Market::from_code(&market_str).unwrap_or(Market::Shenzhen);
    Ok(Favorite {
        code,
        market,
        note,
        added_at,
        sort_order,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Counter to give every test a unique DB path (process id is shared
    /// across all tests in the same test binary).
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn fresh_db() -> Database {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("gupiao_fav_test_{}_{}", std::process::id(), n));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fav.db");
        Database::open_at(&path).unwrap()
    }

    #[test]
    fn test_add_and_get() {
        let db = fresh_db();
        let store = FavoriteStore::new(&db);
        let fav = Favorite::new("600519".into(), Market::Shanghai, Some("long-term".into()));
        store.add(&fav).unwrap();

        let got = store.get("600519", Market::Shanghai).unwrap().unwrap();
        assert_eq!(got.code, "600519");
        assert_eq!(got.market, Market::Shanghai);
        assert_eq!(got.note.as_deref(), Some("long-term"));
    }

    #[test]
    fn test_add_upsert() {
        let db = fresh_db();
        let store = FavoriteStore::new(&db);
        let fav = Favorite::new("000001".into(), Market::Shenzhen, None);
        store.add(&fav).unwrap();
        let mut fav2 = fav.clone();
        fav2.note = Some("updated".into());
        store.add(&fav2).unwrap();
        assert_eq!(store.count().unwrap(), 1);
        assert_eq!(
            store
                .get("000001", Market::Shenzhen)
                .unwrap()
                .unwrap()
                .note
                .as_deref(),
            Some("updated")
        );
    }

    #[test]
    fn test_remove() {
        let db = fresh_db();
        let store = FavoriteStore::new(&db);
        store
            .add(&Favorite::new("300750".into(), Market::Shenzhen, None))
            .unwrap();
        assert!(store.remove("300750", Market::Shenzhen).unwrap());
        assert!(!store.remove("300750", Market::Shenzhen).unwrap());
        assert_eq!(store.count().unwrap(), 0);
    }

    #[test]
    fn test_list_order() {
        let db = fresh_db();
        let store = FavoriteStore::new(&db);
        store
            .add(&Favorite::new("600519".into(), Market::Shanghai, None))
            .unwrap();
        store
            .add(&Favorite::new("000001".into(), Market::Shenzhen, None))
            .unwrap();
        store
            .add(&Favorite::new("300750".into(), Market::Shenzhen, None))
            .unwrap();
        let list = store.list().unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].code, "600519");
    }

    #[test]
    fn test_contains() {
        let db = fresh_db();
        let store = FavoriteStore::new(&db);
        store
            .add(&Favorite::new("688981".into(), Market::Shanghai, None))
            .unwrap();
        assert!(store.contains("688981", Market::Shanghai).unwrap());
        assert!(!store.contains("688981", Market::Shenzhen).unwrap());
    }
}
