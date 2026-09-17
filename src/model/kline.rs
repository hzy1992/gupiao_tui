//! 历史 K 线数据模型
//!
//! 设计为「纯数据 + 构造工具」，不依赖任何 IO 或全局状态。
//!
//! 当前的 K 线精度是**日 K**，由新浪财经接口提供。
//! 后续如需分钟级或周 K，只需在 Provider 层扩展，模型层不变。

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// 单根 K 线（一天）
///
/// 字段命名严格对齐 Provider 返回的 JSON 键，避免不必要的转换。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Kline {
    /// 交易日期，例如 `2026-09-03`
    pub date: NaiveDate,
    /// 开盘价（元）
    pub open: f64,
    /// 最高价（元）
    pub high: f64,
    /// 最低价（元）
    pub low: f64,
    /// 收盘价（元）
    pub close: f64,
    /// 成交量（**手**；1 手 = 100 股）
    pub volume: i64,
}

impl Kline {
    /// 当日涨跌幅（%），需要前一日的收盘价才能算
    pub fn change_pct(&self, prev_close: f64) -> Option<f64> {
        if prev_close > 0.0 {
            Some((self.close - prev_close) / prev_close * 100.0)
        } else {
            None
        }
    }

    /// 当日振幅（%）
    pub fn amplitude_pct(&self) -> f64 {
        if self.open > 0.0 {
            (self.high - self.low) / self.open * 100.0
        } else {
            0.0
        }
    }

    /// 当日成交额（元）。成交量（手）× 100 × 当日均价。
    ///
    /// 注：新浪接口直接返回手数 × 100 = 股数 × 均价 = 成交额。
    /// 这里用 `(high+low+2*close)/4` 作粗略 VWAP，足够回测用。
    pub fn amount_yuan(&self) -> f64 {
        let avg_price = (self.high + self.low + 2.0 * self.close) / 4.0;
        (self.volume as f64) * 100.0 * avg_price
    }
}

/// 一段时间序列的 K 线，按日期升序排列
///
/// 设计为「不可变快照」，方便在函数间传递和测试。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct KlineSeries {
    pub bars: Vec<Kline>,
}

impl KlineSeries {
    pub fn new(bars: Vec<Kline>) -> Self {
        Self { bars }
    }

    pub fn len(&self) -> usize {
        self.bars.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bars.is_empty()
    }

    /// 在指定日期返回 K 线引用（线性查找；bars 数量很小所以 O(n) 可接受）
    pub fn at(&self, date: NaiveDate) -> Option<&Kline> {
        self.bars.iter().find(|k| k.date == date)
    }

    /// 按索引取（0-based）
    pub fn get(&self, idx: usize) -> Option<&Kline> {
        self.bars.get(idx)
    }

    /// 第一根 / 最后一根
    pub fn first(&self) -> Option<&Kline> {
        self.bars.first()
    }
    pub fn last(&self) -> Option<&Kline> {
        self.bars.last()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn test_change_pct_basic() {
        let k = Kline {
            date: d("2026-09-03"),
            open: 100.0,
            high: 110.0,
            low: 95.0,
            close: 105.0,
            volume: 10000,
        };
        assert!((k.change_pct(100.0).unwrap() - 5.0).abs() < 1e-6);
        assert!(k.change_pct(0.0).is_none());
    }

    #[test]
    fn test_amplitude() {
        let k = Kline {
            date: d("2026-09-03"),
            open: 100.0,
            high: 110.0,
            low: 90.0,
            close: 105.0,
            volume: 0,
        };
        assert!((k.amplitude_pct() - 20.0).abs() < 1e-6);
    }

    #[test]
    fn test_amount_yuan() {
        let k = Kline {
            date: d("2026-09-03"),
            open: 100.0,
            high: 110.0,
            low: 90.0,
            close: 100.0,
            volume: 1000, // 1000 手
        };
        // avg = (110 + 90 + 2*100)/4 = 100
        // amount = 1000 * 100 * 100 = 1e7
        assert!((k.amount_yuan() - 1e7).abs() < 1e-3);
    }

    #[test]
    fn test_series_at_and_get() {
        let series = KlineSeries::new(vec![
            Kline { date: d("2026-09-01"), open: 1.0, high: 1.0, low: 1.0, close: 1.0, volume: 0 },
            Kline { date: d("2026-09-02"), open: 2.0, high: 2.0, low: 2.0, close: 2.0, volume: 0 },
        ]);
        assert_eq!(series.len(), 2);
        assert_eq!(series.at(d("2026-09-02")).unwrap().open, 2.0);
        assert!(series.at(d("2026-09-03")).is_none());
        assert_eq!(series.get(0).unwrap().open, 1.0);
        assert!(series.get(5).is_none());
        assert_eq!(series.first().unwrap().open, 1.0);
        assert_eq!(series.last().unwrap().open, 2.0);
    }
}
