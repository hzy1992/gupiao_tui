//! 数据源层
//!
//! 通过 QuoteProvider trait 抽象所有行情数据源。
//! 当前实现：
//! - 	encent：腾讯财经（qt.gtimg.cn），默认数据源
//!
//! 后续可扩展：
//! - astmoney：东方财富（push2.eastmoney.com）
//! - sina：新浪财经
//!

use async_trait::async_trait;

use crate::error::Result;
use crate::model::{KlineSeries, Quote, Symbol};

mod news;
mod sina;
mod tencent;

pub use news::EastMoneyNewsProvider;
pub use sina::SinaKlineProvider;
pub use tencent::TencentProvider;

/// 行情数据源抽象
///
/// 实现者负责：给定一个 Symbol，从对应的 HTTP 接口拉取数据，
/// 并归一化为内部 Quote 结构。
#[async_trait]
pub trait QuoteProvider: Send + Sync {
    /// 数据源名称（用于显示和日志）
    fn name(&self) -> &'static str;

    /// 获取指定股票的实时行情
    async fn fetch(&self, symbol: &Symbol) -> Result<Quote>;
}

/// 历史 K 线数据源抽象
///
/// 与 QuoteProvider 解耦，因为：
/// - 数据源不同（腾讯实时 / 新浪历史）
/// - 失败容忍度不同（实时拉不到可以重试，历史拉不到可以读缓存）
#[async_trait]
pub trait KlineProvider: Send + Sync {
    /// 数据源名称（用于显示和日志）
    fn name(&self) -> &'static str;

    /// 拉取最近 days 个交易日的日 K 线
    async fn fetch(&self, symbol: &Symbol, days: usize) -> Result<KlineSeries>;
}