//! SQLite 数据库管理
//!
//! 职责：
//! - 决定数据库文件路径（跨平台）
//! - 创建/打开数据库连接
//! - 初始化 schema（首次运行）
//! - 升级迁移（schema_version 表驱动）

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};

use super::{StorageError, StorageResult};

/// 当前 schema 版本号
///
/// 每次新增/修改表结构时递增，并相应地在 `migrate()` 中追加分支。
pub const CURRENT_SCHEMA_VERSION: i32 = 3;

/// 返回默认的数据库文件路径
///
/// - Windows: `%APPDATA%\gupiao\gupiao.db`
/// - Linux:   `~/.config/gupiao/gupiao.db`
/// - macOS:   `~/Library/Application Support/gupiao/gupiao.db`
pub fn default_db_path() -> StorageResult<PathBuf> {
    let base = dirs::config_dir()
        .or_else(dirs::data_dir)
        .ok_or(StorageError::NoConfigDir)?;
    Ok(base.join("gupiao").join("gupiao.db"))
}

/// 同 `default_db_path`，但允许显式覆盖（测试用）
pub fn db_path() -> StorageResult<PathBuf> {
    if let Ok(p) = std::env::var("GUPIAO_DB") {
        return Ok(PathBuf::from(p));
    }
    default_db_path()
}

/// 已打开的数据库句柄
pub struct Database {
    conn: Connection,
    path: PathBuf,
}

impl Database {
    /// 打开（必要时创建）数据库文件并确保 schema 是最新的
    pub fn open() -> StorageResult<Self> {
        let path = db_path()?;
        Self::open_at(&path)
    }

    /// 在指定路径打开（测试 / 用户自定义路径）
    pub fn open_at(path: &Path) -> StorageResult<Self> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let conn = Connection::open(path)?;
        // WAL 模式对 CLI 工具非必需，但能提升后续并发扩展（如后台任务）的兼容性
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        let mut db = Self {
            conn,
            path: path.to_path_buf(),
        };
        db.init_schema()?;
        Ok(db)
    }

    /// 数据库文件路径（用于显示给用户）
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 内部访问连接（crate 内使用）
    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }

    /// 初始化或迁移 schema
    fn init_schema(&mut self) -> StorageResult<()> {
        // 先建一个轻量的版本表（即便后续整库迁移也保留它）
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER PRIMARY KEY
            )",
            [],
        )?;

        let current: i32 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        migrate(&mut self.conn, current)?;
        Ok(())
    }

    /// 清空所有自选股（便捷函数）
    ///
    /// 当前 favorites 是唯一的业务表，直接 DELETE FROM 即可。
    /// 后续若加入多张业务表，此函数应改为"按表名循环清空"或提供单独的事务化 API。
    pub fn clear_favorites(&self) -> StorageResult<usize> {
        let n = self.conn.execute("DELETE FROM favorites", [])?;
        Ok(n)
    }
}

/// 在事务内按版本号顺序执行迁移
fn migrate(conn: &mut Connection, from: i32) -> StorageResult<()> {
    if from < 1 {
        let tx = conn.transaction()?;
        tx.execute(
            "CREATE TABLE IF NOT EXISTS favorites (
                code         TEXT    NOT NULL,
                market       TEXT    NOT NULL,
                note         TEXT,
                added_at     INTEGER NOT NULL,
                sort_order   INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (code, market)
            )",
            [],
        )?;
        tx.execute(
            "CREATE INDEX IF NOT EXISTS idx_favorites_added_at
                ON favorites (added_at)",
            [],
        )?;
        tx.execute(
            "INSERT INTO schema_version (version) VALUES (?1)",
            params![1],
        )?;
        tx.commit()?;
    }

    // v2：新增 daily_klines 表（历史 K 线缓存）
    if from < 2 {
        let tx = conn.transaction()?;
        tx.execute(
            "CREATE TABLE IF NOT EXISTS daily_klines (
                code         TEXT    NOT NULL,
                market       TEXT    NOT NULL,
                date         TEXT    NOT NULL,   -- YYYY-MM-DD
                open         REAL    NOT NULL,
                high         REAL    NOT NULL,
                low          REAL    NOT NULL,
                close        REAL    NOT NULL,
                volume       INTEGER NOT NULL,
                fetched_at   INTEGER NOT NULL,   -- Unix 秒
                PRIMARY KEY (code, market, date)
            )",
            [],
        )?;
        tx.execute(
            "CREATE INDEX IF NOT EXISTS idx_daily_klines_date
                ON daily_klines (date)",
            [],
        )?;
        tx.execute(
            "INSERT INTO schema_version (version) VALUES (?1)",
            params![2],
        )?;
        tx.commit()?;
    }

    // v3：新增 ml_models 表（机器学习模型持久化）
    if from < 3 {
        let tx = conn.transaction()?;
        tx.execute(
            "CREATE TABLE IF NOT EXISTS ml_models (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                kind            TEXT    NOT NULL,
                hold_days       INTEGER NOT NULL,
                threshold_pct   REAL    NOT NULL,
                is_default      INTEGER NOT NULL DEFAULT 0,
                params_json     TEXT    NOT NULL,
                report_json     TEXT    NOT NULL,
                trained_at      INTEGER NOT NULL,
                train_samples   INTEGER NOT NULL,
                cv_mean         REAL    NOT NULL,
                cv_min          REAL    NOT NULL,
                cv_max          REAL    NOT NULL
            )",
            [],
        )?;
        tx.execute(
            "CREATE INDEX IF NOT EXISTS idx_ml_models_default
                ON ml_models (is_default)",
            [],
        )?;
        tx.execute(
            "INSERT INTO schema_version (version) VALUES (?1)",
            params![3],
        )?;
        tx.commit()?;
    }

    // 后续版本在此追加：if from < 4 { ... }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn test_open_creates_schema() {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("gupiao_db_test_{}_{}", std::process::id(), n));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.db");

        let db = Database::open_at(&path).unwrap();
        let v: i32 = db
            .conn()
            .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, CURRENT_SCHEMA_VERSION);
    }
}
