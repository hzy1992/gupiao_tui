//! 本地持久化存储层
//!
//! 当前提供：
//! - `db`：SQLite 连接管理、schema 初始化与迁移
//! - `favorite`：收藏（自选股）的数据访问层（DAO）
//! - `kline`：历史 K 线缓存（DAO）
//!
//! ## 数据库位置
//! - Windows: `%APPDATA%\gupiao\gupiao.db`
//! - Linux:   `~/.config/gupiao/gupiao.db`
//! - macOS:   `~/Library/Application Support/gupiao/gupiao.db`
//!
//! ## 扩展性
//! schema 通过 `schema_version` 表管理版本，新增表/字段时只需：
//! 1. 在 `CURRENT_SCHEMA_VERSION` 中递增版本号
//! 2. 在 `db::migrate` 中追加一个分支
//!
//! 后续可平滑添加：自选股分组、价格提醒、用户配置等。

pub mod db;
pub mod favorite;
pub mod kline;
pub mod model;

pub use db::{db_path, default_db_path, Database};
pub use favorite::FavoriteStore;
pub use kline::KlineStore;
pub use model::{ModelRecord, ModelStore};

/// 统一的存储错误类型
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("SQLite 错误: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("无法获取配置目录，请检查 HOME/APPDATA 环境变量")]
    NoConfigDir,

    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
}

pub type StorageResult<T> = std::result::Result<T, StorageError>;
