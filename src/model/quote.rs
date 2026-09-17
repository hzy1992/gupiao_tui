//! 行情数据模型
//!
//! `Quote` 是行情的统一结构，所有数据源（腾讯/东财/新浪等）
//! 最终都会归一化到这个类型，由 `Display` 层消费。

use chrono::{DateTime, Local, TimeZone};
use serde::{Deserialize, Serialize};

use super::market::Market;

/// 五档行情（买卖盘）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Depth {
    /// 卖盘价格（卖1~卖5）
    pub ask_prices: [Option<f64>; 5],
    /// 卖盘量（手）
    pub ask_vols: [Option<i64>; 5],
    /// 买盘价格（买1~买5）
    pub bid_prices: [Option<f64>; 5],
    /// 买盘量（手）
    pub bid_vols: [Option<i64>; 5],
}

impl Depth {
    /// 是否为空（买卖盘完全无数据）
    pub fn is_empty(&self) -> bool {
        self.ask_prices.iter().all(|p| p.is_none()) && self.bid_prices.iter().all(|p| p.is_none())
    }
}

/// 单只股票的实时行情
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Quote {
    /// 股票代码（不含市场前缀）
    pub code: String,
    /// 股票名称
    pub name: String,
    /// 所属市场
    pub market: Market,

    /// 最新价（元）
    pub price: f64,
    /// 昨收（元）
    pub prev_close: f64,
    /// 今开（元）
    pub open: f64,
    /// 最高（元）
    pub high: f64,
    /// 最低（元）
    pub low: f64,

    /// 涨跌额（元）= price - prev_close
    pub change: f64,
    /// 涨跌幅（%）= change / prev_close * 100
    pub change_pct: f64,

    /// 成交量（手）
    pub volume: i64,
    /// 成交额（元）
    pub amount: f64,

    /// 换手率（%）
    pub turnover_rate: Option<f64>,
    /// 市盈率（动）
    pub pe: Option<f64>,
    /// 市净率
    pub pb: Option<f64>,
    /// 总市值（元）
    pub market_cap: Option<f64>,
    /// 流通市值（元）
    pub float_cap: Option<f64>,
    /// 振幅（%）
    pub amplitude: Option<f64>,

    /// 行情更新时间
    pub update_time: DateTime<Local>,

    /// 五档行情
    #[serde(default)]
    pub depth: Depth,
}

impl Quote {
    /// 涨跌方向
    pub fn trend(&self) -> Trend {
        if self.change > 0.0 {
            Trend::Up
        } else if self.change < 0.0 {
            Trend::Down
        } else {
            Trend::Flat
        }
    }

    /// 由腾讯接口原始 14 位时间戳构造 `DateTime<Local>`
    pub fn parse_tencent_time(raw: &str) -> Option<DateTime<Local>> {
        if raw.len() < 14 {
            return None;
        }
        // 注意 chrono 0.4.45 签名：year 是 i32，其余是 u32
        let y: i32 = raw[0..4].parse().ok()?;
        let mo: u32 = raw[4..6].parse().ok()?;
        let d: u32 = raw[6..8].parse().ok()?;
        let h: u32 = raw[8..10].parse().ok()?;
        let mi: u32 = raw[10..12].parse().ok()?;
        let s: u32 = raw[12..14].parse().ok()?;
        Local.with_ymd_and_hms(y, mo, d, h, mi, s).single()
    }
}

/// 涨跌方向
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trend {
    Up,
    Down,
    Flat,
}
