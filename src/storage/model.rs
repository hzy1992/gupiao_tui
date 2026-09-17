//! ML 模型持久化（DAO）
//!
//! 存储在 SQLite `ml_models` 表（v3 schema），主键自增。
//! 同一时间最多一个 `is_default = 1` 的模型——`set_default` 自动取消其它。

use rusqlite::{params, OptionalExtension};

use crate::model::ml::{LoadedModel, ModelKind, TrainReport, TrainedModel};

use super::db::Database;
use super::StorageResult;

/// 模型数据库行
#[derive(Debug, Clone)]
pub struct ModelRecord {
    pub id: i64,
    pub kind: ModelKind,
    pub hold_days: usize,
    pub threshold_pct: f64,
    pub is_default: bool,
    pub model: TrainedModel,
    pub report: TrainReport,
}

/// 模型 DAO
pub struct ModelStore<'a> {
    db: &'a Database,
}

impl<'a> ModelStore<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// 保存一个新训练的模型（自动分配 id，params/report 序列化为 JSON）
    ///
    /// 如果 `is_default = true`，会先把其它模型置为非默认。
    pub fn save(&self, model: &TrainedModel, report: &TrainReport, is_default: bool) -> StorageResult<i64> {
        let params_json = serde_json::to_string(model)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        let report_json = serde_json::to_string(report)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

        let tx = self.db.conn().unchecked_transaction()?;
        let id: i64 = {
            let mut stmt = tx.prepare(
                "INSERT INTO ml_models
                    (kind, hold_days, threshold_pct, is_default,
                     params_json, report_json, trained_at,
                     train_samples, cv_mean, cv_min, cv_max)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            )?;
            stmt.execute(params![
                kind_to_str(report.kind),
                report.hold_days as i64,
                report.threshold_pct,
                if is_default { 1 } else { 0 },
                params_json,
                report_json,
                report.trained_at,
                report.train_samples as i64,
                report.cv_mean,
                report.cv_min,
                report.cv_max,
            ])?;
            tx.last_insert_rowid()
        };
        if is_default {
            tx.execute(
                "UPDATE ml_models SET is_default = 0 WHERE id != ?1",
                params![id],
            )?;
        }
        tx.commit()?;
        Ok(id)
    }

    /// 列出所有模型（按 trained_at 倒序）
    pub fn list(&self) -> StorageResult<Vec<ModelRecord>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT id, kind, hold_days, threshold_pct, is_default,
                    params_json, report_json
             FROM ml_models
             ORDER BY trained_at DESC",
        )?;
        let rows = stmt
            .query_map([], row_to_record)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// 读取完整模型（params + report 反序列化）
    pub fn load(&self, id: i64) -> StorageResult<Option<LoadedModel>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT id, kind, hold_days, threshold_pct, is_default,
                    params_json, report_json
             FROM ml_models
             WHERE id = ?1",
        )?;
        let rec = stmt
            .query_row(params![id], row_to_record)
            .optional()?;
        Ok(rec.map(|r| LoadedModel {
            id: r.id,
            kind: r.kind,
            hold_days: r.hold_days,
            threshold_pct: r.threshold_pct,
            model: r.model,
            report: r.report,
        }))
    }

    /// 加载默认模型（is_default = 1），无默认返回 None
    pub fn load_default(&self) -> StorageResult<Option<LoadedModel>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT id, kind, hold_days, threshold_pct, is_default,
                    params_json, report_json
             FROM ml_models
             WHERE is_default = 1
             ORDER BY trained_at DESC
             LIMIT 1",
        )?;
        let rec = stmt
            .query_row([], row_to_record)
            .optional()?;
        Ok(rec.map(|r| LoadedModel {
            id: r.id,
            kind: r.kind,
            hold_days: r.hold_days,
            threshold_pct: r.threshold_pct,
            model: r.model,
            report: r.report,
        }))
    }

    /// 设为默认（其它自动取消）
    pub fn set_default(&self, id: i64) -> StorageResult<()> {
        let tx = self.db.conn().unchecked_transaction()?;
        tx.execute("UPDATE ml_models SET is_default = 0", [])?;
        tx.execute(
            "UPDATE ml_models SET is_default = 1 WHERE id = ?1",
            params![id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// 删除指定模型
    pub fn delete(&self, id: i64) -> StorageResult<usize> {
        let n = self
            .db
            .conn()
            .execute("DELETE FROM ml_models WHERE id = ?1", params![id])?;
        Ok(n)
    }

    /// 统计模型数量
    pub fn count(&self) -> StorageResult<usize> {
        let n: i64 = self
            .db
            .conn()
            .query_row("SELECT COUNT(*) FROM ml_models", [], |r| r.get(0))?;
        Ok(n as usize)
    }
}

fn kind_to_str(k: ModelKind) -> &'static str {
    match k {
        ModelKind::LogisticRegression => "lr",
        ModelKind::DecisionTree => "dt",
        ModelKind::RandomForest => "rf",
    }
}

fn str_to_kind(s: &str) -> rusqlite::Result<ModelKind> {
    match s {
        "lr" => Ok(ModelKind::LogisticRegression),
        "dt" => Ok(ModelKind::DecisionTree),
        "rf" => Ok(ModelKind::RandomForest),
        other => Err(rusqlite::Error::FromSqlConversionFailure(
            1,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown model kind: {}", other),
            )),
        )),
    }
}

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<ModelRecord> {
    let id: i64 = row.get(0)?;
    let kind_str: String = row.get(1)?;
    let kind = str_to_kind(&kind_str)?;
    let hold_days: i64 = row.get(2)?;
    let threshold_pct: f64 = row.get(3)?;
    let is_default: i64 = row.get(4)?;
    let params_json: String = row.get(5)?;
    let report_json: String = row.get(6)?;

    let model: TrainedModel = serde_json::from_str(&params_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let report: TrainReport = serde_json::from_str(&report_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, Box::new(e))
    })?;

    Ok(ModelRecord {
        id,
        kind,
        hold_days: hold_days as usize,
        threshold_pct,
        is_default: is_default != 0,
        model,
        report,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ml::{DtNode, DtParams, LrParams, ModelKind, RfParams, TrainReport, TrainedModel};
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn fresh_db() -> Database {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "gupiao_model_test_{}_{}",
            std::process::id(),
            n
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.db");
        Database::open_at(&path).unwrap()
    }

    fn mk_report(kind: ModelKind) -> TrainReport {
        TrainReport {
            kind,
            hold_days: 5,
            threshold_pct: 0.5,
            trained_at: 1700000000,
            train_samples: 200,
            n_features: 13,
            cv_accuracies: vec![0.55, 0.60, 0.58, 0.62],
            cv_mean: 0.5875,
            cv_min: 0.55,
            cv_max: 0.62,
            train_accuracy: 0.7,
            train_codes: vec!["600519".to_string()],
            feature_importance: None,
        }
    }

    fn mk_lr() -> TrainedModel {
        TrainedModel::Lr(LrParams {
            coefficients: vec![0.1; 13],
            intercept: 0.05,
        })
    }

    fn mk_dt() -> TrainedModel {
        TrainedModel::Dt(DtParams {
            root: DtNode::Leaf { prob: 0.7, n: 10 },
            n_classes: 2,
        })
    }

    fn mk_rf() -> TrainedModel {
        TrainedModel::Rf(RfParams {
            trees: vec![DtParams {
                root: DtNode::Leaf { prob: 0.6, n: 5 },
                n_classes: 2,
            }],
            n_classes: 2,
        })
    }

    #[test]
    fn test_save_and_load_lr() {
        let db = fresh_db();
        let store = ModelStore::new(&db);
        let id = store.save(&mk_lr(), &mk_report(ModelKind::LogisticRegression), true).unwrap();
        assert!(id > 0);
        let loaded = store.load(id).unwrap().unwrap();
        assert_eq!(loaded.kind, ModelKind::LogisticRegression);
        assert_eq!(loaded.hold_days, 5);
        match loaded.model {
            TrainedModel::Lr(p) => {
                assert_eq!(p.coefficients.len(), 13);
                assert!((p.intercept - 0.05).abs() < 1e-9);
            }
            _ => panic!("expected Lr"),
        }
    }

    #[test]
    fn test_save_dt_and_rf() {
        let db = fresh_db();
        let store = ModelStore::new(&db);
        let id_dt = store.save(&mk_dt(), &mk_report(ModelKind::DecisionTree), false).unwrap();
        let id_rf = store.save(&mk_rf(), &mk_report(ModelKind::RandomForest), false).unwrap();
        assert_ne!(id_dt, id_rf);
        let rf = store.load(id_rf).unwrap().unwrap();
        match rf.model {
            TrainedModel::Rf(r) => assert_eq!(r.trees.len(), 1),
            _ => panic!("expected Rf"),
        }
    }

    #[test]
    fn test_list_orders_by_trained_at_desc() {
        let db = fresh_db();
        let store = ModelStore::new(&db);
        let mut r = mk_report(ModelKind::LogisticRegression);
        r.trained_at = 100;
        store.save(&mk_lr(), &r, false).unwrap();
        let mut r = mk_report(ModelKind::DecisionTree);
        r.trained_at = 300;
        store.save(&mk_dt(), &r, false).unwrap();
        let mut r = mk_report(ModelKind::RandomForest);
        r.trained_at = 200;
        store.save(&mk_rf(), &r, false).unwrap();
        let list = store.list().unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].kind, ModelKind::DecisionTree);
        assert_eq!(list[2].kind, ModelKind::LogisticRegression);
    }

    #[test]
    fn test_is_default_uniqueness() {
        let db = fresh_db();
        let store = ModelStore::new(&db);
        store.save(&mk_lr(), &mk_report(ModelKind::LogisticRegression), true).unwrap();
        let id2 = store.save(&mk_dt(), &mk_report(ModelKind::DecisionTree), true).unwrap();
        let list = store.list().unwrap();
        let defaults: Vec<_> = list.iter().filter(|m| m.is_default).collect();
        assert_eq!(defaults.len(), 1);
        assert_eq!(defaults[0].id, id2);
    }

    #[test]
    fn test_load_default_and_delete() {
        let db = fresh_db();
        let store = ModelStore::new(&db);
        assert!(store.load_default().unwrap().is_none());
        let id = store.save(&mk_rf(), &mk_report(ModelKind::RandomForest), true).unwrap();
        assert_eq!(store.load_default().unwrap().unwrap().id, id);
        assert_eq!(store.count().unwrap(), 1);
        assert_eq!(store.delete(id).unwrap(), 1);
        assert_eq!(store.count().unwrap(), 0);
        assert!(store.load(id).unwrap().is_none());
    }

    #[test]
    fn test_count_empty() {
        let db = fresh_db();
        let store = ModelStore::new(&db);
        assert_eq!(store.count().unwrap(), 0);
        assert!(store.list().unwrap().is_empty());
    }
}

