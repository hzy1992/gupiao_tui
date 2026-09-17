//! 技术指标计算（纯函数）
//!
//! 所有函数都以 `&Quote`（必要时加上其它参数）为输入，
//! 返回 `f64` 或结构化的 `Indicators`，不修改任何全局状态。

use crate::model::{Depth, Quote};

/// 五项核心指标的集合
///
/// 所有字段都已做"无量纲化"或"归一化"：
/// - `position` ∈ [0, 1]：当前价在当日振幅中的相对位置
/// - `committee_ratio` ∈ [-1, 1]：正数偏多，负数偏空
/// - `volume_ratio` ∈ [0, +∞)：>1 表示放量
/// - `pv_ratio`     ∈ [0, +∞)：价量配合度，越大越强势
/// - `trend_strength` ∈ [-1, 1]：综合趋势强度
#[derive(Debug, Clone, PartialEq)]
pub struct Indicators {
    /// 当前价在当日 [low, high] 中的相对位置，0=最低，1=最高
    pub position: f64,
    /// 委比 = (买盘总量 − 卖盘总量) / (买盘总量 + 卖盘总量)
    /// 范围 [-1, 1]，+1 表示买盘压倒性强势
    pub committee_ratio: f64,
    /// 量比的近似估算（启发式，无历史数据）
    pub volume_ratio: f64,
    /// 价量配合度 = 成交额(元) / 振幅(%)
    /// 单位：(元 / %)，越大越强势
    pub pv_ratio: f64,
    /// 综合趋势强度 ∈ [-1, 1]
    pub trend_strength: f64,
    /// 当日振幅（%），None 表示振幅未知
    pub amplitude: Option<f64>,
}

/// 价格位（支撑 / 压力）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PriceLevel {
    pub price: f64,
    /// 该价位背后的依据（用于向用户解释"为什么是这个数"）
    pub source: PriceLevelSource,
}

/// `PriceLevel` 的依据来源
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriceLevelSource {
    /// 当日最低/最高
    DayLow,
    DayHigh,
    /// 五档买盘（支撑候选）
    BidAsk,
    /// 五档卖盘（压力候选）
    OfferAsk,
    /// 昨收价（重要心理位）
    PrevClose,
    /// 今开价（多空分水岭）
    Open,
    /// 综合算法推导
    Composite,
}

impl PriceLevelSource {
    /// 中文标签（用于显示）
    pub fn label(self) -> &'static str {
        match self {
            PriceLevelSource::DayLow => "日内最低",
            PriceLevelSource::DayHigh => "日内最高",
            PriceLevelSource::BidAsk => "买盘挂单",
            PriceLevelSource::OfferAsk => "卖盘挂单",
            PriceLevelSource::PrevClose => "昨收",
            PriceLevelSource::Open => "今开",
            PriceLevelSource::Composite => "综合推导",
        }
    }
}

impl Indicators {
    /// 从一条行情推导出全部指标
    pub fn from_quote(q: &Quote) -> Self {
        Self {
            position: price_position(q),
            committee_ratio: committee_ratio(&q.depth),
            volume_ratio: volume_ratio(q),
            pv_ratio: pv_ratio(q),
            trend_strength: trend_strength(q),
            amplitude: q.amplitude,
        }
    }

    /// 简短的「一行中文描述」
    pub fn describe(&self) -> String {
        format!(
            "position={:.0} pct, committee={:+.2}, vol_ratio={:.2}, pv={:.0}",
            self.position * 100.0,
            self.committee_ratio,
            self.volume_ratio,
            self.pv_ratio,
        )
    }
}

/// 当前价在当日振幅中的相对位置
///
/// 当 high == low（停牌或一字板）时返回 0.5（中位）
fn price_position(q: &Quote) -> f64 {
    if q.high <= q.low || q.high <= 0.0 || q.low <= 0.0 {
        return 0.5;
    }
    let pos = (q.price - q.low) / (q.high - q.low);
    pos.clamp(0.0, 1.0)
}

/// 委比 = (买盘总量 − 卖盘总量) / (买盘总量 + 卖盘总量)
///
/// 无买/卖盘时返回 0.0
pub fn committee_ratio(depth: &Depth) -> f64 {
    let bid: i64 = depth.bid_vols.iter().filter_map(|v| *v).sum();
    let ask: i64 = depth.ask_vols.iter().filter_map(|v| *v).sum();
    let total = bid + ask;
    if total == 0 {
        return 0.0;
    }
    let ratio = (bid - ask) as f64 / total as f64;
    ratio.clamp(-1.0, 1.0)
}

/// 量比的近似估算
///
/// 由于没有历史分时数据，采用启发式：
/// 综合"成交额规模"和"换手率"，归一化到 0~5：
/// - 成交额 ≥ 50 亿 → 量比 5
/// - 成交额 10~50 亿 → 线性映射
/// - 成交额 < 10 亿 → 按比例
/// 再叠加换手率加成
fn volume_ratio(q: &Quote) -> f64 {
    if q.amount <= 0.0 {
        return 0.0;
    }
    // 成交额归一化：以 50 亿为"非常活跃"=5.0
    let amount_score = (q.amount / 1e9).clamp(0.0, 5.0);
    // 换手率加成：≥3% 加 0.5
    let tr_bonus = q.turnover_rate.unwrap_or(0.0).max(0.0) / 6.0;
    (amount_score + tr_bonus).clamp(0.0, 10.0)
}

/// 价量配合度 = 成交额(元) / 振幅(%)
fn pv_ratio(q: &Quote) -> f64 {
    let amp = q.amplitude.unwrap_or(1.0).max(0.1);
    if q.amount <= 0.0 {
        return 0.0;
    }
    (q.amount / amp).clamp(0.0, 1e10)
}

/// 综合趋势强度 ∈ [-1, 1]
///
/// 加权：涨跌幅×0.5 + 委比×0.3 + 价量方向×0.2
/// （价量配合对方向敏感：上涨放量=+，下跌放量=-）
fn trend_strength(q: &Quote) -> f64 {
    let pct_score = (q.change_pct / 10.0).clamp(-1.0, 1.0);
    let comm = committee_ratio(&q.depth);
    let pv = pv_ratio(q);
    let pv_magnitude = (pv / 1e8).clamp(0.0, 5.0) / 5.0; // 0~1
    // 方向：price up → +pv_magnitude；down → -pv_magnitude
    let pv_signed = if q.change_pct >= 0.0 {
        pv_magnitude
    } else {
        -pv_magnitude
    };
    let raw = pct_score * 0.5 + comm * 0.3 + pv_signed * 0.2;
    raw.clamp(-1.0, 1.0)
}

/// 由五档买盘价加权得到一个"支撑位候选"
///
/// 加权规则：买1权重大，买5权重小（越接近当前价越有意义）
pub fn bid_support(depth: &Depth) -> Option<PriceLevel> {
    let prices = depth.bid_prices;
    let vols = depth.bid_vols;
    let mut weighted_sum = 0.0;
    let mut weight_total = 0.0;
    for i in 0..5 {
        if let (Some(p), Some(v)) = (prices[i], vols[i]) {
            if v > 0 {
                let w = (5 - i) as f64;
                weighted_sum += p * w * v as f64;
                weight_total += w * v as f64;
            }
        }
    }
    if weight_total <= 0.0 {
        return None;
    }
    Some(PriceLevel {
        price: weighted_sum / weight_total,
        source: PriceLevelSource::BidAsk,
    })
}

/// 由五档卖盘价加权得到一个"压力位候选"
pub fn ask_resistance(depth: &Depth) -> Option<PriceLevel> {
    let prices = depth.ask_prices;
    let vols = depth.ask_vols;
    let mut weighted_sum = 0.0;
    let mut weight_total = 0.0;
    for i in 0..5 {
        if let (Some(p), Some(v)) = (prices[i], vols[i]) {
            if v > 0 {
                let w = (5 - i) as f64;
                weighted_sum += p * w * v as f64;
                weight_total += w * v as f64;
            }
        }
    }
    if weight_total <= 0.0 {
        return None;
    }
    Some(PriceLevel {
        price: weighted_sum / weight_total,
        source: PriceLevelSource::OfferAsk,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Depth, Market, Quote};
    use chrono::Local;

    fn mk_quote(price: f64, high: f64, low: f64, open: f64, prev_close: f64) -> Quote {
        Quote {
            code: "600519".into(),
            name: "贵州茅台".into(),
            market: Market::Shanghai,
            price,
            prev_close,
            open,
            high,
            low,
            change: price - prev_close,
            change_pct: (price - prev_close) / prev_close * 100.0,
            volume: 10000,
            amount: 1e9,
            turnover_rate: Some(0.5),
            pe: Some(20.0),
            pb: Some(5.0),
            market_cap: Some(16451.20),
            float_cap: Some(16451.20),
            amplitude: Some(2.5),
            update_time: Local::now(),
            depth: Depth::default(),
        }
    }

    #[test]
    fn test_price_position_mid() {
        let q = mk_quote(100.0, 110.0, 90.0, 95.0, 100.0);
        assert!((price_position(&q) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_price_position_top() {
        let q = mk_quote(110.0, 110.0, 90.0, 95.0, 100.0);
        assert!((price_position(&q) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_price_position_zero_amplitude() {
        // 一字板：high == low -> 返回 0.5
        let q = mk_quote(100.0, 100.0, 100.0, 100.0, 100.0);
        assert!((price_position(&q) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_committee_ratio_balanced() {
        let mut d = Depth::default();
        d.bid_vols = [Some(100); 5];
        d.ask_vols = [Some(100); 5];
        assert_eq!(committee_ratio(&d), 0.0);
    }

    #[test]
    fn test_committee_ratio_bullish() {
        let mut d = Depth::default();
        d.bid_vols = [Some(200); 5];
        d.ask_vols = [Some(100); 5];
        // total = 1500, diff = 500
        assert!((committee_ratio(&d) - 1.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn test_committee_ratio_empty() {
        let d = Depth::default();
        assert_eq!(committee_ratio(&d), 0.0);
    }

    #[test]
    fn test_trend_strength_clamped() {
        let q = mk_quote(200.0, 210.0, 90.0, 95.0, 100.0);
        // 涨跌幅 100% -> 截断
        let s = trend_strength(&q);
        assert!(s > 0.0);
        assert!(s <= 1.0);
    }

    #[test]
    fn test_indicators_from_quote() {
        let q = mk_quote(100.0, 110.0, 90.0, 95.0, 100.0);
        let ind = Indicators::from_quote(&q);
        assert!((ind.position - 0.5).abs() < 1e-6);
        assert_eq!(ind.committee_ratio, 0.0);
        assert!(ind.pv_ratio > 0.0);
    }

    #[test]
    fn test_bid_support_weighted() {
        let mut d = Depth::default();
        d.bid_prices = [Some(10.0), Some(9.0), Some(8.0), Some(7.0), Some(6.0)];
        d.bid_vols = [Some(100), Some(1), Some(1), Some(1), Some(1)];
        let s = bid_support(&d).unwrap();
        assert!(s.price > 9.0);
        assert_eq!(s.source, PriceLevelSource::BidAsk);
    }

    #[test]
    fn test_bid_support_empty() {
        let d = Depth::default();
        assert!(bid_support(&d).is_none());
    }

    #[test]
    fn test_price_level_source_label() {
        assert_eq!(PriceLevelSource::DayLow.label(), "日内最低");
        assert_eq!(PriceLevelSource::Composite.label(), "综合推导");
    }
}
