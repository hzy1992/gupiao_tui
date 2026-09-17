//! 数据模型层
//!
//! 包含所有不依赖外部 IO 的核心数据类型：
//! - Market：市场枚举
//! - Symbol：归一化后的股票标识
//! - Quote：行情
//! - Depth：五档买卖盘
//! - Favorite：收藏（自选股）
//! - NewsItem / Announcement / StockNews：新闻与公告

pub mod favorite;
pub mod kline;
pub mod market;
pub mod ml;
pub mod news;
pub mod quote;

pub use favorite::Favorite;
pub use kline::{Kline, KlineSeries};
pub use market::{Market, Symbol};
pub use ml::{
    predict_proba, DtNode, DtParams, LoadedModel, LrParams, ModelKind, RfParams, TrainConfig,
    TrainReport, TrainedModel,
};
pub use news::{Announcement, NewsItem, StockNews};
pub use quote::{Depth, Quote, Trend};