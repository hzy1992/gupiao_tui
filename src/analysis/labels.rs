//! 监督学习标签生成
//!
//! ## 目标
//!
//! 在 t 日发出信号后，持有 hold_days 个交易日后平仓；
//! 用 exit_close / entry_close - 1 计算未来收益（%），作为回归目标。
//! 同时保留二分类标签（Up/Down）用于模型训练时的分类接口。
//!
//! ## 设计原则
//!
//! - 纯函数：只依赖 model::KlineSeries，无 IO
//! - 跨期对应：用索引而非日历日期算未来收益（避免节假日导致的日期漂移）
//! - 样本边界：idx >= 27 且 idx + hold_days < series.len() 才生成样本
//! - 回归目标 return_pct（%）作为主要标签，保留 Label 枚举用于分类接口兼容
//!
//! ## 与 features 模块的关系
//!
//! build_samples 把 Features::from_klines 和 future_return 组合起来，
//! 是 ML 训练最常用的入口。

use chrono::NaiveDate;

use crate::model::{KlineSeries, Market};

use super::features::Features;

/// 二分类标签（保留用于分类训练接口）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Label {
    Up,
    Down,
}

impl Label {
    /// 从未来收益（%）映射到标签
    ///
    /// threshold_pct 是分界：收益 > 阈值 -> Up，否则 Down。
    /// 默认 0.5% 表示要求涨幅超过 0.5% 才算 Up，
    /// 这样能过滤掉大量噪音，避免模型被小涨小跌的随机波动混淆。
    pub fn from_future_return(ret_pct: f64, threshold_pct: f64) -> Self {
        if ret_pct > threshold_pct {
            Label::Up
        } else {
            Label::Down
        }
    }

    /// 数值化（Up=1.0, Down=0.0），喂给分类模型
    pub fn as_f64(self) -> f64 {
        match self {
            Label::Up => 1.0,
            Label::Down => 0.0,
        }
    }

    /// 从 f64 还原（用于模型输出解析）
    pub fn from_f64(v: f64) -> Self {
        if v >= 0.5 {
            Label::Up
        } else {
            Label::Down
        }
    }
}

/// 单个样本的元数据（用于回看 / 调试）
#[derive(Debug, Clone)]
pub struct Sample {
    /// 特征向量
    pub features: Features,
    /// 二分类标签（用于分类训练）
    pub label: Label,
    /// 回归目标：未来 hold_days 日的收益率（%）
    pub return_pct: f64,
    /// 信号日期
    pub signal_date: NaiveDate,
    /// 平仓日期
    pub exit_date: NaiveDate,
    /// 入场价（当日收盘）
    pub entry_price: f64,
    /// 平仓价（hold_days 日后收盘）
    pub exit_price: f64,
}

impl Sample {
    /// 是否为上涨样本（用于过滤）
    pub fn is_up(&self) -> bool {
        self.label == Label::Up
    }

    /// 收益是否超过阈值（用于置信过滤）
    pub fn return_exceeds(&self, threshold_pct: f64) -> bool {
        self.return_pct > threshold_pct
    }
}

/// 计算 t 日（idx）持有 hold_days 后的相对收益（%）
///
/// 用 idx 当日 close 作为建仓价；idx+hold_days 当日 close 作为平仓价。
/// 返回 None 当未来区间超出序列边界。
pub fn future_return(series: &KlineSeries, idx: usize, hold_days: usize) -> Option<f64> {
    if hold_days == 0 {
        return None;
    }
    let exit_idx = idx.checked_add(hold_days)?;
    if exit_idx >= series.bars.len() {
        return None;
    }
    let entry = series.bars[idx].close;
    let exit = series.bars[exit_idx].close;
    if entry <= 0.0 {
        return None;
    }
    Some((exit - entry) / entry * 100.0)
}

/// 从一段 K 线序列生成所有可用的训练样本
///
/// - 跳过前 27 日（features 需要 28 日历史，KDJ 最严格需要 27 天）
/// - 跳过末尾 hold_days 日（没有未来收益）
/// - turnover_proxy：训练时若 K 线没有流通市值信息，传 None
///
/// 返回的样本按 signal_date 升序。
pub fn build_samples(
    series: &KlineSeries,
    hold_days: usize,
    threshold_pct: f64,
    turnover_proxy: Option<f64>,
) -> Vec<Sample> {
    let n = series.bars.len();
    if n <= 27 + hold_days {
        return Vec::new();
    }
    let last_idx = n - hold_days - 1;

    // 一次性批量提取所有特征（O(n)，比逐个调用快 n 倍）
    let all_features = Features::extract_all_features(series, turnover_proxy);

    let mut out = Vec::new();
    for idx in 27..=last_idx {
        let Some(feat) = all_features[idx] else {
            continue;
        };
        let Some(ret) = future_return(series, idx, hold_days) else {
            continue;
        };
        let label = Label::from_future_return(ret, threshold_pct);
        out.push(Sample {
            features: feat,
            label,
            return_pct: ret,
            signal_date: series.bars[idx].date,
            exit_date: series.bars[idx + hold_days].date,
            entry_price: series.bars[idx].close,
            exit_price: series.bars[idx + hold_days].close,
        });
    }
    out
}

/// 多只股票合并到一个训练集
pub fn merge_samples(
    datasets: &[(String, Market, KlineSeries)],
    hold_days: usize,
    threshold_pct: f64,
) -> Vec<Sample> {
    let mut out = Vec::new();
    for (code, _market, series) in datasets {
        eprintln!("[DEBUG] merge_samples: processing {} ({} bars)", code, series.bars.len());
        out.extend(build_samples(series, hold_days, threshold_pct, None));
        eprintln!("[DEBUG] merge_samples: {} done, total={}", code, out.len());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Kline;
    use chrono::NaiveDate;

    fn d_fn(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn uptrend() -> KlineSeries {
        let mut bars = Vec::new();
        let mut p = 100.0;
        for i in 0..35 {
            let date = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()
                + chrono::Duration::days(i as i64);
            p *= 1.01;
            bars.push(Kline {
                date,
                open: p - 0.1,
                high: p + 0.5,
                low: p - 0.5,
                close: p,
                volume: 1_000_000,
            });
        }
        KlineSeries::new(bars)
    }

    #[test]
    fn test_label_from_threshold() {
        // threshold=0.5: 涨 1% > 0.5 -> Up；涨 0.5% == 0.5 -> Down（严格 >）
        assert_eq!(Label::from_future_return(1.0, 0.5), Label::Up);
        assert_eq!(Label::from_future_return(0.5, 0.5), Label::Down);
        assert_eq!(Label::from_future_return(-3.0, 0.5), Label::Down);
        assert_eq!(Label::from_future_return(0.001, 0.0), Label::Up);
    }

    #[test]
    fn test_label_f64_roundtrip() {
        assert_eq!(Label::from_f64(Label::Up.as_f64()), Label::Up);
        assert_eq!(Label::from_f64(Label::Down.as_f64()), Label::Down);
        assert_eq!(Label::from_f64(0.5), Label::Up);
        assert_eq!(Label::from_f64(0.4999), Label::Down);
    }

    #[test]
    fn test_future_return_basic() {
        let s = uptrend();
        let r = future_return(&s, 0, 5).unwrap();
        let expected = (1.01f64.powi(5) - 1.0) * 100.0;
        assert!((r - expected).abs() < 1e-4);
    }

    #[test]
    fn test_future_return_out_of_bounds() {
        let s = uptrend(); // 35 根
        assert!(future_return(&s, 30, 5).is_none());
        assert!(future_return(&s, 29, 5).is_some());
    }

    #[test]
    fn test_build_samples_uptrend_mostly_up() {
        let s = uptrend();
        // threshold=0.5：上涨 1%/天，5天 ≈ 5.1%，>> 0.5%，所以全部 Up
        let samples = build_samples(&s, 5, 0.5, None);
        // idx ∈ [27, 29]，共 3 个样本
        assert!(samples.len() >= 2);
        for sample in &samples {
            assert_eq!(sample.label, Label::Up, "ret={}", sample.return_pct);
            assert!(sample.return_pct > 0.0);
        }
    }

    #[test]
    fn test_build_samples_threshold_filters() {
        let s = uptrend();
        // 极高阈值：任何收益都达不到
        let samples = build_samples(&s, 5, 100.0, None);
        assert!(!samples.is_empty());
        assert!(samples.iter().all(|s| s.label == Label::Down));
    }

    #[test]
    fn test_build_samples_too_short() {
        let bars: Vec<Kline> = (0..30)
            .map(|i| {
                let date = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()
                    + chrono::Duration::days(i as i64);
                Kline {
                    date,
                    open: 100.0,
                    high: 100.5,
                    low: 99.5,
                    close: 100.0 + i as f64,
                    volume: 1_000_000,
                }
            })
            .collect();
        let s = KlineSeries::new(bars);
        assert!(build_samples(&s, 5, 0.5, None).is_empty());
    }

    #[test]
    fn test_sample_return_exceeds() {
        let s = uptrend();
        let samples = build_samples(&s, 5, 1.0, None);
        if !samples.is_empty() {
            assert!(samples[0].return_exceeds(0.5));
            assert!(!samples[0].return_exceeds(100.0));
        }
    }

    #[test]
    fn test_merge_samples_combines() {
        let s1 = uptrend();
        let mut s2 = uptrend();
        for (i, bar) in s2.bars.iter_mut().enumerate() {
            bar.date = d_fn("2025-01-01") + chrono::Duration::days(i as i64);
        }
        let datasets = vec![
            ("S1".to_string(), Market::Shanghai, s1),
            ("S2".to_string(), Market::Shenzhen, s2),
        ];
        let merged = merge_samples(&datasets, 5, 0.5);
        assert!(merged.len() >= 4);
    }
}
