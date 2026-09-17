//! 收藏（自选股）数据模型
//!
//! 表示用户收藏的一只股票。除了股票代码外，还允许附带备注，
//! 方便后续扩展（成本价、目标价、提醒阈值等）。

use serde::{Deserialize, Serialize};

use super::market::Market;

/// 一条收藏记录
///
/// 字段设计原则：
/// - `code` + `market` 共同作为唯一约束（同一市场的同一代码只能收藏一次）
/// - `note` 备用字段，用户可自由填写（如"长线"、"观察"等）
/// - `added_at` 使用 Unix 时间戳，便于排序与跨时区显示
/// - `sort_order` 留给后续的"拖拽排序"功能
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Favorite {
    /// 6 位股票代码
    pub code: String,
    /// 所属市场
    pub market: Market,
    /// 用户自定义备注（可为空）
    #[serde(default)]
    pub note: Option<String>,
    /// 收藏时间（Unix 秒）
    pub added_at: i64,
    /// 自定义排序值，越小越靠前
    #[serde(default)]
    pub sort_order: i64,
}

impl Favorite {
    /// 构造一条新收藏记录，`added_at` 自动取当前时间
    pub fn new(code: String, market: Market, note: Option<String>) -> Self {
        Self {
            code,
            market,
            note,
            added_at: chrono::Utc::now().timestamp(),
            sort_order: 0,
        }
    }

    /// 显示用的简短标识，如 `600519.SH`
    pub fn symbol_str(&self) -> String {
        format!("{}.{}", self.code, self.market)
    }
}
