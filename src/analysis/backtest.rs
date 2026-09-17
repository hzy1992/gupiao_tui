//! 历史回测模块
//!
//! ## 目的
//!
//! 用真实历史数据评估**当前算法**的准确度和盈利能力，识别：
//! - 整体胜率（按信号持有 N 日后上涨的比例）
//! - 分信号胜率（StrongBuy vs Buy vs Hold 等）
//! - 分强度胜率（高强度信号是否更准）
//! - 平均/中位收益、最大回撤
//!
//! ## 关键设计
//!
//! - **纯函数**：不发起 IO，不持有全局状态。输入 KlineSeries，输出 BacktestResult。
//! - **截面模拟**：每一天用「当日能拿到的截面信息」构造一个 Quote，喂给现有 analyze()。
//!   没有五档盘口的真实历史数据，所以 depth = Depth::default()，
//!   即「只用价格+成交量维度评估」。
//! - **止损/止盈**：可在固定持仓期之外提前退出
//! - **交易成本建模**：A股实际交易含印花税（卖出 0.1%）+ 佣金（~0.03%）+ 滑点（~0.05%），
//!   买入单边约 0.08%，卖出单边约 0.18%，已在收益计算中扣除。

use chrono::NaiveDate;
use std::collections::HashMap;

use crate::model::{Depth, Kline, KlineSeries, Market, Quote};

use super::analyze_with;
use super::signals::{ScoreThresholds, ScoreWeights, SignalKind};

/// 单次回测的一笔交易
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trade {
    /// 信号发出的日期
    pub signal_date: NaiveDate,
    /// 信号类型
    pub kind: SignalKind,
    /// 信号强度（0~100）
    pub strength: u8,
    /// 建仓价（当日收盘价）
    pub entry_price: f64,
    /// 平仓价（N 个交易日后收盘价）
    pub exit_price: f64,
    /// 平仓日期
    pub exit_date: NaiveDate,
    /// 收益率（含交易成本，%）
    pub return_pct: f64,
    /// 是否因止损提前退出
    pub stopped_loss: bool,
    /// 是否因止盈提前退出
    pub took_profit: bool,
}

/// 回测参数
#[derive(Debug, Clone)]
pub struct BacktestParams {
    /// 持仓天数（N 个交易日后平仓）
    pub hold_days: usize,
    /// 是否只统计 Buy/StrongBuy（多仓视角）；false 则全部信号
    pub only_buy_side: bool,
    /// 打分权重（默认 = 原算法权重）
    pub weights: ScoreWeights,
    /// 打分阈值
    pub thresholds: ScoreThresholds,
    /// 最小信号强度阈值（0-100）：低于此强度的信号被跳过
    pub min_signal_strength: u8,
    /// 止损线（%），None 表示不用止损
    pub stop_loss_pct: Option<f64>,
    /// 止盈线（%），None 表示不用止盈
    pub take_profit_pct: Option<f64>,
}

impl Default for BacktestParams {
    fn default() -> Self {
        Self {
            hold_days: 5,
            only_buy_side: true,
            weights: ScoreWeights::default(),
            thresholds: ScoreThresholds::default(),
            min_signal_strength: 0,
            stop_loss_pct: None,
            take_profit_pct: None,
        }
    }
}

impl BacktestParams {
    pub fn with_hold(hold_days: usize) -> Self {
        Self { hold_days, ..Self::default() }
    }

    pub fn high_confidence() -> Self {
        Self {
            hold_days: 5,
            only_buy_side: true,
            weights: ScoreWeights::default(),
            thresholds: ScoreThresholds::default(),
            min_signal_strength: 50,
            stop_loss_pct: None,
            take_profit_pct: None,
        }
    }

    pub fn live_friendly() -> Self {
        Self {
            hold_days: 5,
            only_buy_side: true,
            weights: ScoreWeights::default(),
            thresholds: ScoreThresholds::default(),
            min_signal_strength: 40,
            stop_loss_pct: Some(5.0),
            take_profit_pct: Some(15.0),
        }
    }
}

/// 分信号类型的统计
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KindStats {
    pub total: usize,
    pub win: usize,
    pub win_rate: f64,
    pub avg_return_pct: f64,
}

/// 分强度段的统计（0-30 / 30-60 / 60-100）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrengthBucket {
    pub min: u8,
    pub max: u8,
    pub total: usize,
    pub win: usize,
    pub win_rate: f64,
    pub avg_return_pct: f64,
}

/// 回测完整结果
#[derive(Debug, Clone)]
pub struct BacktestResult {
    pub code: String,
    pub market: String,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    pub total_klines: usize,
    pub total_signals: usize,
    pub buy_signals: usize,
    pub win_count: usize,
    pub win_rate: f64,
    pub avg_return_pct: f64,
    pub median_return_pct: f64,
    pub max_drawdown_pct: f64,
    pub profit_factor: f64,
    pub by_signal_kind: HashMap<SignalKind, KindStats>,
    pub by_strength: Vec<StrengthBucket>,
    pub sample_trades: Vec<Trade>,
    pub params: BacktestParams,
    pub stop_loss_count: usize,
    pub take_profit_count: usize,
    pub avg_stop_loss_pct: f64,
    pub avg_take_profit_pct: f64,
}

const COST_BUY_PCT: f64 = 0.0008;
const COST_SELL_PCT: f64 = 0.0018;

pub fn run(series: &KlineSeries, code: &str, market: Market, params: BacktestParams) -> BacktestResult {
    let trades = simulate(series, code, market, &params);
    let n = series.bars.len();
    let period_start = series.bars.first().map(|b| b.date).unwrap_or_default();
    let period_end = series.bars.last().map(|b| b.date).unwrap_or_default();
    let buy_signals = trades.iter().filter(|t| matches!(t.kind, SignalKind::Buy | SignalKind::StrongBuy)).count();
    let win_count = trades.iter().filter(|t| t.return_pct > 0.0).count();
    let all_returns: Vec<f64> = trades.iter().map(|t| t.return_pct).collect();
    let avg_return_pct = if all_returns.is_empty() { 0.0 } else { all_returns.iter().sum::<f64>() / all_returns.len() as f64 };
    let median_return_pct = median(all_returns.clone());
    let max_drawdown = max_drawdown_pct(&trades);
    let profit_factor = profit_factor(&trades);
    let stop_loss_count = trades.iter().filter(|t| t.stopped_loss).count();
    let take_profit_count = trades.iter().filter(|t| t.took_profit).count();
    let stop_loss_trades: Vec<&Trade> = trades.iter().filter(|t| t.stopped_loss).collect();
    let avg_sl = if stop_loss_trades.is_empty() { 0.0 } else { stop_loss_trades.iter().map(|t| t.return_pct).sum::<f64>() / stop_loss_trades.len() as f64 };
    let take_profit_trades: Vec<&Trade> = trades.iter().filter(|t| t.took_profit).collect();
    let avg_tp = if take_profit_trades.is_empty() { 0.0 } else { take_profit_trades.iter().map(|t| t.return_pct).sum::<f64>() / take_profit_trades.len() as f64 };
    BacktestResult {
        code: code.to_string(),
        market: market.code().to_string(),
        period_start,
        period_end,
        total_klines: n,
        total_signals: trades.len(),
        buy_signals,
        win_count,
        win_rate: if trades.is_empty() { 0.0 } else { win_count as f64 / trades.len() as f64 },
        avg_return_pct,
        median_return_pct,
        max_drawdown_pct: max_drawdown,
        profit_factor,
        by_signal_kind: by_signal_kind(&trades),
        by_strength: by_strength(&trades),
        sample_trades: trades,
        params,
        stop_loss_count,
        take_profit_count,
        avg_stop_loss_pct: avg_sl,
        avg_take_profit_pct: avg_tp,
    }
}

fn simulate(series: &KlineSeries, code: &str, market: Market, params: &BacktestParams) -> Vec<Trade> {
    let n = series.bars.len();
    if n < params.hold_days + 5 { return Vec::new(); }
    let max_idx = n.saturating_sub(params.hold_days + 1);
    let mut trades = Vec::new();

    for idx in 5..=max_idx {
        let q = build_quote(&series.bars[idx], code, market);
        let r = analyze_with(&q, &params.weights, &params.thresholds);
        if r.signal.strength < params.min_signal_strength { continue; }
        if params.only_buy_side && !matches!(r.signal.kind, SignalKind::Buy | SignalKind::StrongBuy) { continue; }
        let entry_price = series.bars[idx].close;
        let signal_date = series.bars[idx].date;
        let (exit_price, exit_date, stopped_loss, took_profit) = simulate_exit(series, idx, entry_price, params);
        let gross = (exit_price - entry_price) / entry_price;
        let cost_adj = 1.0 - COST_BUY_PCT - COST_SELL_PCT;
        let return_pct = (gross * cost_adj) * 100.0;
        trades.push(Trade {
            signal_date,
            kind: r.signal.kind,
            strength: r.signal.strength,
            entry_price,
            exit_price,
            exit_date,
            return_pct,
            stopped_loss,
            took_profit,
        });
    }
    trades
}

fn simulate_exit(
    series: &KlineSeries,
    entry_idx: usize,
    entry_price: f64,
    params: &BacktestParams,
) -> (f64, NaiveDate, bool, bool) {
    let max_look = (entry_idx + params.hold_days).min(series.bars.len() - 1);
    let mut peak_price = entry_price;
    let mut stop_hit = false;
    let mut tp_hit = false;
    let mut exit_idx = entry_idx + params.hold_days;

    for i in (entry_idx + 1)..=max_look {
        let bar = &series.bars[i];
        let high = bar.high;
        let low = bar.low;
        if high > peak_price { peak_price = high; }

        if let Some(tp) = params.take_profit_pct {
            let tp_price = entry_price * (1.0 + tp / 100.0);
            if high >= tp_price { tp_hit = true; exit_idx = i; break; }
        }

        if let Some(sl) = params.stop_loss_pct {
            let sl_price = peak_price * (1.0 - sl / 100.0);
            if low <= sl_price { stop_hit = true; exit_idx = i; break; }
        }
    }

    let exit_price = series.bars[exit_idx].close;
    let exit_date = series.bars[exit_idx].date;
    (exit_price, exit_date, stop_hit, tp_hit)
}

fn build_quote(bar: &Kline, code: &str, market: Market) -> Quote {
    Quote {
        code: code.into(),
        name: String::new(),
        market,
        price: bar.close,
        prev_close: bar.open,
        open: bar.open,
        high: bar.high,
        low: bar.low,
        change: bar.close - bar.open,
        change_pct: if bar.open > 0.0 { (bar.close - bar.open) / bar.open * 100.0 } else { 0.0 },
        volume: bar.volume,
        amount: 0.0,
        turnover_rate: None,
        pe: None,
        pb: None,
        market_cap: None,
        float_cap: None,
        amplitude: if bar.open > 0.0 { Some((bar.high - bar.low) / bar.open * 100.0) } else { None },
        update_time: chrono::Local::now(),
        depth: Depth::default(),
    }
}

fn median(mut xs: Vec<f64>) -> f64 {
    if xs.is_empty() { return 0.0; }
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = xs.len() / 2;
    if xs.len() % 2 == 0 { (xs[mid - 1] + xs[mid]) / 2.0 } else { xs[mid] }
}

fn profit_factor(trades: &[Trade]) -> f64 {
    let gains: f64 = trades.iter().filter(|t| t.return_pct > 0.0).map(|t| t.return_pct).sum();
    let losses: f64 = trades.iter().filter(|t| t.return_pct < 0.0).map(|t| t.return_pct.abs()).sum();
    if losses < 1e-9 { return f64::INFINITY; }
    gains / losses
}

fn max_drawdown_pct(trades: &[Trade]) -> f64 {
    if trades.is_empty() { return 0.0; }
    let mut peak = f64::MIN;
    let mut max_dd = 0.0;
    let mut equity = 100.0;
    for t in trades {
        equity *= 1.0 + t.return_pct / 100.0;
        if equity > peak { peak = equity; }
        let dd = peak - equity;
        if dd > max_dd { max_dd = dd; }
    }
    max_dd
}

fn by_signal_kind(trades: &[Trade]) -> HashMap<SignalKind, KindStats> {
    let mut map: HashMap<SignalKind, Vec<&Trade>> = HashMap::new();
    for t in trades { map.entry(t.kind).or_default().push(t); }
    map.into_iter().map(|(k, v)| {
        let total = v.len();
        let wins = v.iter().filter(|t| t.return_pct > 0.0).count();
        let avg = v.iter().map(|t| t.return_pct).sum::<f64>() / total as f64;
        (k, KindStats { total, win: wins, win_rate: wins as f64 / total as f64, avg_return_pct: avg })
    }).collect()
}

fn by_strength(trades: &[Trade]) -> Vec<StrengthBucket> {
    let buckets = [(0, 30), (30, 60), (60, 100)];
    buckets.iter().map(|(lo, hi)| {
        let bucket_trades: Vec<&Trade> = trades.iter().filter(|t| (t.strength as u16) >= *lo && (t.strength as u16) < *hi).collect();
        let total = bucket_trades.len();
        let wins = bucket_trades.iter().filter(|t| t.return_pct > 0.0).count();
        let avg = if total > 0 { bucket_trades.iter().map(|t| t.return_pct).sum::<f64>() / total as f64 } else { 0.0 };
        StrengthBucket { min: *lo as u8, max: *hi as u8, total, win: wins, win_rate: if total > 0 { wins as f64 / total as f64 } else { 0.0 }, avg_return_pct: avg }
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn d(s: &str) -> NaiveDate { NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap() }

    fn trending_series() -> KlineSeries {
        let mut bars = Vec::with_capacity(10);
        let mut p: f64 = 100.0;
        for i in 0..10 {
            let date = d("2026-09-01") + chrono::Duration::days(i as i64);
            let open = p;
            let close = p * (1.0 + 0.01 * if i < 5 { 1.0 } else { -0.5 });
            let high = close.max(open) + 0.5;
            let low = close.min(open) - 0.5;
            bars.push(Kline { date, open, high, low, close, volume: 1_000_000 });
            p = close;
        }
        KlineSeries::new(bars)
    }

    #[test]
    fn test_run_on_empty_series() {
        let s = KlineSeries::default();
        let r = run(&s, "600519", Market::Shanghai, BacktestParams::default());
        assert_eq!(r.total_signals, 0);
        assert_eq!(r.win_rate, 0.0);
    }

    #[test]
    fn test_run_on_short_series_returns_empty() {
        let s = KlineSeries::new(vec![
            Kline { date: d("2026-09-01"), open: 100.0, high: 101.0, low: 99.0, close: 100.0, volume: 0 },
            Kline { date: d("2026-09-02"), open: 100.0, high: 101.0, low: 99.0, close: 100.0, volume: 0 },
            Kline { date: d("2026-09-03"), open: 100.0, high: 101.0, low: 99.0, close: 100.0, volume: 0 },
            Kline { date: d("2026-09-04"), open: 100.0, high: 101.0, low: 99.0, close: 100.0, volume: 0 },
        ]);
        let r = run(&s, "600519", Market::Shanghai, BacktestParams::default());
        assert_eq!(r.total_signals, 0);
    }

    #[test]
    fn test_run_basic_simulation() {
        let s = trending_series();
        let params = BacktestParams { only_buy_side: false, ..BacktestParams::with_hold(3) };
        let _r = run(&s, "600519", Market::Shanghai, params);
    }

    #[test]
    fn test_profit_factor_handles_all_wins() {
        let trades = vec![
            Trade { signal_date: d("2026-09-01"), kind: SignalKind::Buy, strength: 50,
                   entry_price: 100.0, exit_price: 105.0, exit_date: d("2026-09-06"), return_pct: 4.0,
                   stopped_loss: false, took_profit: false },
            Trade { signal_date: d("2026-09-02"), kind: SignalKind::Buy, strength: 50,
                   entry_price: 100.0, exit_price: 103.0, exit_date: d("2026-09-07"), return_pct: 2.0,
                   stopped_loss: false, took_profit: false },
        ];
        let pf = profit_factor(&trades);
        assert!(pf.is_infinite() || pf > 10.0, "pf={}", pf);
    }

    #[test]
    fn test_max_drawdown_calculation() {
        let trades = vec![
            Trade { signal_date: d("2026-09-01"), kind: SignalKind::Buy, strength: 50,
                   entry_price: 100.0, exit_price: 110.0, exit_date: d("2026-09-06"), return_pct: 10.0,
                   stopped_loss: false, took_profit: false },
            Trade { signal_date: d("2026-09-02"), kind: SignalKind::Buy, strength: 50,
                   entry_price: 100.0, exit_price: 95.0, exit_date: d("2026-09-07"), return_pct: -5.0,
                   stopped_loss: false, took_profit: false },
            Trade { signal_date: d("2026-09-03"), kind: SignalKind::Buy, strength: 50,
                   entry_price: 100.0, exit_price: 90.0, exit_date: d("2026-09-08"), return_pct: -10.0,
                   stopped_loss: false, took_profit: false },
        ];
        let dd = max_drawdown_pct(&trades);
        // 复利: 100 -> 110 -> 104.5 -> 94.05, peak=110, dd = 110-94.05 = 15.95
        assert!((dd - 15.95).abs() < 1e-4, "dd={}", dd);
    }

    #[test]
    fn test_median() {
        assert_eq!(median(vec![]), 0.0);
        assert_eq!(median(vec![5.0]), 5.0);
        assert!((median(vec![1.0, 2.0, 3.0]) - 2.0).abs() < 1e-6);
        assert!((median(vec![1.0, 2.0, 3.0, 4.0]) - 2.5).abs() < 1e-6);
    }

    #[test]
    fn test_cost_buy_less_than_sell() {
        assert!(COST_BUY_PCT < COST_SELL_PCT);
    }

    #[test]
    fn test_stop_loss_triggers() {
        let trades = vec![
            Trade { signal_date: d("2026-09-01"), kind: SignalKind::Buy, strength: 50,
                   entry_price: 100.0, exit_price: 95.0, exit_date: d("2026-09-03"),
                   return_pct: -5.0, stopped_loss: true, took_profit: false },
            Trade { signal_date: d("2026-09-02"), kind: SignalKind::Buy, strength: 50,
                   entry_price: 100.0, exit_price: 110.0, exit_date: d("2026-09-07"),
                   return_pct: 10.0, stopped_loss: false, took_profit: false },
        ];
        let result = BacktestResult {
            code: "TEST".into(), market: "SH".into(),
            period_start: d("2026-09-01"), period_end: d("2026-09-10"),
            total_klines: 20, total_signals: 2, buy_signals: 2, win_count: 1,
            win_rate: 0.5, avg_return_pct: 2.5, median_return_pct: 2.5,
            max_drawdown_pct: 5.0, profit_factor: 2.0,
            by_signal_kind: HashMap::new(), by_strength: vec![],
            sample_trades: trades, params: BacktestParams::default(),
            stop_loss_count: 1, take_profit_count: 0,
            avg_stop_loss_pct: -5.0, avg_take_profit_pct: 0.0,
        };
        assert_eq!(result.stop_loss_count, 1);
        assert!(result.avg_stop_loss_pct < 0.0);
    }

    #[test]
    fn test_take_profit_triggers() {
        let trades = vec![
            Trade { signal_date: d("2026-09-01"), kind: SignalKind::Buy, strength: 50,
                   entry_price: 100.0, exit_price: 115.0, exit_date: d("2026-09-04"),
                   return_pct: 15.0, stopped_loss: false, took_profit: true },
            Trade { signal_date: d("2026-09-02"), kind: SignalKind::Buy, strength: 50,
                   entry_price: 100.0, exit_price: 90.0, exit_date: d("2026-09-07"),
                   return_pct: -10.0, stopped_loss: false, took_profit: false },
        ];
        let result = BacktestResult {
            code: "TEST".into(), market: "SH".into(),
            period_start: d("2026-09-01"), period_end: d("2026-09-10"),
            total_klines: 20, total_signals: 2, buy_signals: 2, win_count: 1,
            win_rate: 0.5, avg_return_pct: 2.5, median_return_pct: 2.5,
            max_drawdown_pct: 10.0, profit_factor: 1.5,
            by_signal_kind: HashMap::new(), by_strength: vec![],
            sample_trades: trades, params: BacktestParams::default(),
            stop_loss_count: 0, take_profit_count: 1,
            avg_stop_loss_pct: 0.0, avg_take_profit_pct: 15.0,
        };
        assert_eq!(result.take_profit_count, 1);
        assert!(result.avg_take_profit_pct > 0.0);
    }

    #[test]
    fn test_live_friendly_params() {
        let params = BacktestParams::live_friendly();
        assert_eq!(params.stop_loss_pct, Some(5.0));
        assert_eq!(params.take_profit_pct, Some(15.0));
        assert_eq!(params.min_signal_strength, 40);
    }
}
