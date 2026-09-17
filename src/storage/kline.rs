//! 历史 K 线缓存（DAO）
//!
//! 作用：
//! - 缓存新浪拉到的日 K 线，避免重复请求
//! - 提供按 (code, market, 日期范围) 的查询接口
//! - 写入用 `INSERT OR REPLACE`，同一根 K 线被多次拉取会被覆盖

use chrono::NaiveDate;
use rusqlite::{params, OptionalExtension};

use crate::model::{Kline, KlineSeries, Market};

use super::db::Database;
use super::StorageResult;

/// K 线缓存访问层
pub struct KlineStore<'a> {
    db: &'a Database,
}

impl<'a> KlineStore<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// 写入一组 K 线（覆盖已有数据）
    pub fn upsert_series(
        &self,
        code: &str,
        market: Market,
        series: &KlineSeries,
    ) -> StorageResult<usize> {
        let now = chrono::Local::now().timestamp();
        let tx = self.db.conn().unchecked_transaction()?;
        let mut count = 0;
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO daily_klines
                    (code, market, date, open, high, low, close, volume, fetched_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?;
            for k in &series.bars {
                stmt.execute(params![
                    code,
                    market.code(),
                    k.date.format("%Y-%m-%d").to_string(),
                    k.open,
                    k.high,
                    k.low,
                    k.close,
                    k.volume,
                    now,
                ])?;
                count += 1;
            }
        }
        tx.commit()?;
        Ok(count)
    }

    /// 读取指定范围的 K 线，按日期升序
    pub fn load_range(
        &self,
        code: &str,
        market: Market,
        from: Option<NaiveDate>,
        to: Option<NaiveDate>,
    ) -> StorageResult<KlineSeries> {
        let mut sql = String::from(
            "SELECT date, open, high, low, close, volume
             FROM daily_klines
             WHERE code = ?1 AND market = ?2",
        );
        if from.is_some() {
            sql.push_str(" AND date >= ?3");
        }
        if to.is_some() {
            sql.push_str(if from.is_some() { " AND date <= ?4" } else { " AND date <= ?3" });
        }
        sql.push_str(" ORDER BY date ASC");

        let mut stmt = self.db.conn().prepare(&sql)?;

        let row_map = |row: &rusqlite::Row<'_>| -> rusqlite::Result<Kline> {
            let date_str: String = row.get(0)?;
            let date = NaiveDate::parse_from_str(&date_str, "%Y-%m-%d").map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;
            Ok(Kline {
                date,
                open: row.get(1)?,
                high: row.get(2)?,
                low: row.get(3)?,
                close: row.get(4)?,
                volume: row.get(5)?,
            })
        };

        let f_str = from.map(|d| d.format("%Y-%m-%d").to_string());
        let t_str = to.map(|d| d.format("%Y-%m-%d").to_string());

        let rows: Vec<Kline> = match (f_str, t_str) {
            (Some(f), Some(t)) => stmt
                .query_map(params![code, market.code(), f, t], row_map)?
                .collect::<rusqlite::Result<Vec<_>>>()?,
            (Some(f), None) => stmt
                .query_map(params![code, market.code(), f], row_map)?
                .collect::<rusqlite::Result<Vec<_>>>()?,
            (None, Some(t)) => stmt
                .query_map(params![code, market.code(), t], row_map)?
                .collect::<rusqlite::Result<Vec<_>>>()?,
            (None, None) => stmt
                .query_map(params![code, market.code()], row_map)?
                .collect::<rusqlite::Result<Vec<_>>>()?,
        };

        Ok(KlineSeries::new(rows))
    }

    /// 缓存里有多少根
    pub fn count(&self, code: &str, market: Market) -> StorageResult<usize> {
        let n: i64 = self
            .db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM daily_klines WHERE code = ?1 AND market = ?2",
                params![code, market.code()],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0);
        Ok(n as usize)
    }

    /// 删除某只股票的全部缓存
    pub fn clear(&self, code: &str, market: Market) -> StorageResult<usize> {
        let n = self.db.conn().execute(
            "DELETE FROM daily_klines WHERE code = ?1 AND market = ?2",
            params![code, market.code()],
        )?;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::db::Database;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn fresh_db() -> Database {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("gupiao_kline_test_{}_{}", std::process::id(), n));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("kline.db");
        Database::open_at(&path).unwrap()
    }

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn sample_series() -> KlineSeries {
        KlineSeries::new(vec![
            Kline { date: d("2026-09-01"), open: 10.0, high: 11.0, low: 9.5, close: 10.5, volume: 1000 },
            Kline { date: d("2026-09-02"), open: 10.5, high: 12.0, low: 10.0, close: 11.5, volume: 2000 },
            Kline { date: d("2026-09-03"), open: 11.5, high: 12.5, low: 11.0, close: 12.0, volume: 3000 },
        ])
    }

    #[test]
    fn test_upsert_and_load_all() {
        let db = fresh_db();
        let store = KlineStore::new(&db);
        store
            .upsert_series("600519", Market::Shanghai, &sample_series())
            .unwrap();
        let s = store.load_range("600519", Market::Shanghai, None, None).unwrap();
        assert_eq!(s.len(), 3);
        assert_eq!(s.bars[0].date, d("2026-09-01"));
        assert_eq!(s.bars[2].volume, 3000);
    }

    #[test]
    fn test_upsert_overwrites_existing() {
        let db = fresh_db();
        let store = KlineStore::new(&db);
        let mut series = sample_series();
        store
            .upsert_series("600519", Market::Shanghai, &series)
            .unwrap();
        // 修改第二根的 close，再 upsert
        series.bars[1].close = 99.0;
        store
            .upsert_series("600519", Market::Shanghai, &series)
            .unwrap();
        let s = store.load_range("600519", Market::Shanghai, None, None).unwrap();
        assert_eq!(s.len(), 3); // 不会因重复插入而变多
        assert!((s.bars[1].close - 99.0).abs() < 1e-6);
    }

    #[test]
    fn test_load_range_with_bounds() {
        let db = fresh_db();
        let store = KlineStore::new(&db);
        store
            .upsert_series("000001", Market::Shenzhen, &sample_series())
            .unwrap();

        // 只取 09-02
        let s = store
            .load_range("000001", Market::Shenzhen, Some(d("2026-09-02")), Some(d("2026-09-02")))
            .unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s.bars[0].date, d("2026-09-02"));

        // from 不带 to
        let s = store
            .load_range("000001", Market::Shenzhen, Some(d("2026-09-02")), None)
            .unwrap();
        assert_eq!(s.len(), 2);

        // to 不带 from
        let s = store
            .load_range("000001", Market::Shenzhen, None, Some(d("2026-09-02")))
            .unwrap();
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn test_count_and_clear() {
        let db = fresh_db();
        let store = KlineStore::new(&db);
        assert_eq!(store.count("600519", Market::Shanghai).unwrap(), 0);
        store
            .upsert_series("600519", Market::Shanghai, &sample_series())
            .unwrap();
        assert_eq!(store.count("600519", Market::Shanghai).unwrap(), 3);
        let n = store.clear("600519", Market::Shanghai).unwrap();
        assert_eq!(n, 3);
        assert_eq!(store.count("600519", Market::Shanghai).unwrap(), 0);
    }

    #[test]
    fn test_different_stocks_isolated() {
        let db = fresh_db();
        let store = KlineStore::new(&db);
        store
            .upsert_series("600519", Market::Shanghai, &sample_series())
            .unwrap();
        store
            .upsert_series("000001", Market::Shenzhen, &sample_series())
            .unwrap();
        assert_eq!(store.count("600519", Market::Shanghai).unwrap(), 3);
        assert_eq!(store.count("000001", Market::Shenzhen).unwrap(), 3);
        // 删除其中一只不影响另一只
        store.clear("600519", Market::Shanghai).unwrap();
        assert_eq!(store.count("000001", Market::Shenzhen).unwrap(), 3);
    }
}
