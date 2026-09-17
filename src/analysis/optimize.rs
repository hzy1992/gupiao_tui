//! 算法权重优化器
//!
//! 用历史回测数据找一组 `ScoreWeights`，使整体胜率最高。
//!
//! ## 评分指标说明
//!
//! 评估一组参数的质量不仅看胜率，还要看：
//! - **盈亏比 (Profit Factor)**: gross profit / gross loss，越大越好
//! - **期望收益**: 每笔交易的平均收益，考虑了赔钱交易的影响
//! - **最大回撤**: 控制风险
//! - **交易频率**: min_signal_strength 高则频率低，减少磨损

use crate::model::{KlineSeries, Market};
use crate::provider::KlineProvider;
use super::backtest::{run as run_backtest, BacktestParams};
use super::signals::{ScoreThresholds, ScoreWeights};

/// 一组候选配置 + 其在多股票上的综合表现
#[derive(Debug, Clone)]
pub struct CandidateResult {
    pub label: String,
    pub weights: ScoreWeights,
    pub thresholds: ScoreThresholds,
    pub overall_win_rate: f64,
    pub overall_avg_return: f64,
    pub overall_avg_pf: f64,
    /// 综合评分 = 胜率 * 0.4 + min(盈亏比/5, 1) * 0.3 + min(平均收益/5, 1) * 0.3
    pub composite_score: f64,
    /// 每只股票的多周期平均胜率
    pub per_stock_wr: Vec<(String, f64)>,
    /// 总交易次数（多股票多周期求和）
    pub total_trades: usize,
    /// 平均持仓天数
    pub avg_hold_days: usize,
}

/// 在一组股票数据上评估一组权重配置
pub fn evaluate(
    datasets: &[(String, Market, KlineSeries)],
    weights: &ScoreWeights,
    thresholds: &ScoreThresholds,
    holds: &[usize],
    params: &BacktestParams,
) -> CandidateResult {
    let mut per_stock_wr = Vec::new();
    let mut wr_sum = 0.0;
    let mut ret_sum = 0.0;
    let mut pf_sum = 0.0;
    let mut count = 0;
    let mut total_trades = 0;

    for (code, market, series) in datasets {
        let mut h_wr = 0.0;
        let mut h_ret = 0.0;
        let mut h_pf = 0.0;
        let mut h_n = 0;
        for &h in holds {
            if series.len() <= h {
                continue;
            }
            let mut p = params.clone();
            p.hold_days = h;
            p.weights = *weights;
            p.thresholds = *thresholds;
            let r = run_backtest(series, code, *market, p);
            h_wr += r.win_rate;
            h_ret += r.avg_return_pct;
            h_pf += if r.profit_factor.is_finite() { r.profit_factor } else { 5.0 };
            h_n += 1;
            total_trades += r.total_signals;
        }
        if h_n > 0 {
            let n = h_n as f64;
            per_stock_wr.push((code.clone(), h_wr / n));
            wr_sum += h_wr / n;
            ret_sum += h_ret / n;
            pf_sum += h_pf / n;
            count += 1;
        }
    }

    let overall_wr = if count > 0 { wr_sum / count as f64 } else { 0.0 };
    let overall_ret = if count > 0 { ret_sum / count as f64 } else { 0.0 };
    let overall_pf = if count > 0 { pf_sum / count as f64 } else { 0.0 };

    // 综合评分：平衡胜率、盈亏比、收益
    let pf_score = overall_pf.min(5.0) / 5.0;
    let ret_score = (overall_ret / 5.0).max(0.0).min(1.0);
    let composite = overall_wr * 0.4 + pf_score * 0.3 + ret_score * 0.3;

    CandidateResult {
        label: String::new(),
        weights: *weights,
        thresholds: *thresholds,
        overall_win_rate: overall_wr,
        overall_avg_return: overall_ret,
        overall_avg_pf: overall_pf,
        composite_score: composite,
        per_stock_wr,
        total_trades,
        avg_hold_days: holds.iter().sum::<usize>() / holds.len().max(1),
    }
}

/// 贡献度分析：分别关掉每个维度
pub fn ablation(
    datasets: &[(String, Market, KlineSeries)],
    holds: &[usize],
    params: &BacktestParams,
) -> Vec<CandidateResult> {
    let mut results = Vec::new();

    let baseline = evaluate(datasets, &ScoreWeights::default(), &ScoreThresholds::default(), holds, params);
    let mut baseline_labeled = baseline.clone();
    baseline_labeled.label = "[基线] 默认权重".to_string();
    let baseline_wr = baseline_labeled.overall_win_rate;
    let baseline_pf = baseline_labeled.overall_avg_pf;
    results.push(baseline_labeled);

    let zero_out: Vec<(&str, ScoreWeights)> = vec![
        ("[剔除] 涨跌幅", ScoreWeights { pct: 0.0, ..ScoreWeights::default() }),
        ("[剔除] 价格位置", ScoreWeights { position: 0.0, ..ScoreWeights::default() }),
        ("[剔除] 委比", ScoreWeights { committee: 0.0, ..ScoreWeights::default() }),
        ("[剔除] 量能", ScoreWeights { volume: 0.0, ..ScoreWeights::default() }),
        ("[剔除] 价量配合", ScoreWeights { pv: 0.0, ..ScoreWeights::default() }),
        ("[剔除] 趋势强度", ScoreWeights { trend: 0.0, ..ScoreWeights::default() }),
    ];
    for (label, w) in zero_out {
        let mut r = evaluate(datasets, &w, &ScoreThresholds::default(), holds, params);
        let wr_delta = (r.overall_win_rate - baseline_wr) * 100.0;
        let pf_delta = r.overall_avg_pf - baseline_pf;
        r.label = format!("{} (WRΔ={:+.2}%, PFΔ={:+.2})", label, wr_delta, pf_delta);
        results.push(r);
    }

    results
}

/// 网格搜索：3^6 = 729 种组合，按综合评分降序
pub fn grid_search(
    datasets: &[(String, Market, KlineSeries)],
    holds: &[usize],
    params: &BacktestParams,
) -> Vec<CandidateResult> {
    let vals = [0.0_f64, 25.0, 50.0];
    let mut all_results: Vec<CandidateResult> = Vec::new();

    for &pct in &vals {
        for &pos in &vals {
            for &com in &vals {
                for &vol in &vals {
                    for &pv in &vals {
                        for &tr in &vals {
                            let w = ScoreWeights { pct, position: pos, committee: com, volume: vol, pv, trend: tr };
                            let r = evaluate(datasets, &w, &ScoreThresholds::default(), holds, params);
                            all_results.push(r);
                        }
                    }
                }
            }
        }
    }

    all_results.sort_by(|a, b| {
        b.composite_score
            .partial_cmp(&a.composite_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    all_results
}

/// 网格搜索 + 止损止盈参数（3^6 × 2^3 = 5832 种组合，按综合评分降序取前 N）
pub fn grid_search_with_sltp(
    datasets: &[(String, Market, KlineSeries)],
    holds: &[usize],
    top_n: usize,
) -> Vec<CandidateResult> {
    let vals = [0.0_f64, 25.0, 50.0];
    let sl_vals = [None, Some(3.0), Some(5.0)];
    let tp_vals = [None, Some(10.0), Some(15.0)];
    let str_vals = [0u8, 30, 50];
    let mut all_results: Vec<CandidateResult> = Vec::new();

    for &pct in &vals {
        for &pos in &vals {
            for &com in &vals {
                for &vol in &vals {
                    for &pv in &vals {
                        for &tr in &vals {
                            for sl in &sl_vals {
                                for tp in &tp_vals {
                                    for &minsig in &str_vals {
                                        let base_params = BacktestParams {
                                            hold_days: 5,
                                            only_buy_side: true,
                                            weights: ScoreWeights { pct, position: pos, committee: com, volume: vol, pv, trend: tr },
                                            thresholds: ScoreThresholds::default(),
                                            min_signal_strength: minsig,
                                            stop_loss_pct: *sl,
                                            take_profit_pct: *tp,
                                        };
                                        let r = evaluate(datasets, &ScoreWeights { pct, position: pos, committee: com, volume: vol, pv, trend: tr },
                                                         &ScoreThresholds::default(), holds, &base_params);
                                        all_results.push(r);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    all_results.sort_by(|a, b| {
        b.composite_score
            .partial_cmp(&a.composite_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    all_results.truncate(top_n);
    all_results
}

/// 把评估结果格式化成控制台表格
pub fn format_results(results: &[CandidateResult], top_n: usize) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    writeln!(s, "  {:<36} {:>8} {:>10} {:>8} {:>10}", "配置", "胜率", "平均收益", "PF", "综合分").unwrap();
    writeln!(s, "  {}", "─".repeat(80)).unwrap();
    for r in results.iter().take(top_n) {
        writeln!(
            s,
            "  {:<36} {:>7.2}% {:>+9.3}% {:>8.2} {:>10.3}",
            r.label,
            r.overall_win_rate * 100.0,
            r.overall_avg_return,
            r.overall_avg_pf,
            r.composite_score,
        ).unwrap();
    }
    s
}

/// 拉取并缓存一只股票的 K 线（main.rs 复用）
pub async fn ensure_klines(
    store: &crate::storage::KlineStore<'_>,
    provider: &crate::provider::SinaKlineProvider,
    symbol: &crate::model::Symbol,
    days: usize,
    force_refresh: bool,
) -> anyhow::Result<KlineSeries> {
    let cached = store.count(&symbol.code, symbol.market)?;
    let need_fetch = force_refresh || cached < days;
    if need_fetch {
        let s = provider.fetch(symbol, days).await?;
        store.upsert_series(&symbol.code, symbol.market, &s)?;
        return Ok(s);
    }
    Ok(store.load_range(&symbol.code, symbol.market, None, None)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Kline;
    use chrono::NaiveDate;

    fn mk_data() -> Vec<(String, Market, KlineSeries)> {
        let mut bars = Vec::new();
        for i in 0..50 {
            let date = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()
                + chrono::Duration::days(i as i64);
            let close = 100.0 + (i as f64) * 0.5;
            bars.push(Kline {
                date, open: close - 0.3, high: close + 0.5, low: close - 0.5, close,
                volume: 1_000_000 + i as i64 * 1000,
            });
        }
        vec![("TEST".to_string(), Market::Shanghai, KlineSeries::new(bars))]
    }

    fn default_params() -> BacktestParams {
        BacktestParams {
            hold_days: 5,
            only_buy_side: true,
            weights: ScoreWeights::default(),
            thresholds: ScoreThresholds::default(),
            min_signal_strength: 0,
            stop_loss_pct: None,
            take_profit_pct: None,
        }
    }

    #[test]
    fn test_evaluate_basic() {
        let data = mk_data();
        let r = evaluate(&data, &ScoreWeights::default(), &ScoreThresholds::default(), &[5], &default_params());
        assert_eq!(r.per_stock_wr.len(), 1);
    }

    #[test]
    fn test_ablation_seven() {
        let data = mk_data();
        let r = ablation(&data, &[5], &default_params());
        assert_eq!(r.len(), 7);
        assert!(r[0].label.contains("基线"));
    }

    #[test]
    fn test_grid_search_729() {
        let data = mk_data();
        let r = grid_search(&data, &[5], &default_params());
        assert_eq!(r.len(), 729);
        assert!(r[0].composite_score >= r.last().map(|x| x.composite_score).unwrap_or(0.0));
    }

    #[test]
    fn test_optimal_weights_constant() {
        let data = mk_data();
        let r = evaluate(&data, &ScoreWeights::optimal(), &ScoreThresholds::default(), &[5], &default_params());
        assert!(r.overall_win_rate >= 0.0);
    }

    #[test]
    fn test_composite_score_calculation() {
        // 综合评分 = wr*0.4 + min(pf/5,1)*0.3 + min(ret/5,1)*0.3
        // 纯随机模型: wr=0.5, pf=1, ret=0 -> score = 0.2
        // 好模型: wr=0.7, pf=2, ret=3 -> score = 0.28 + 0.12 + 0.18 = 0.58
        let data = mk_data();
        let r = evaluate(&data, &ScoreWeights::default(), &ScoreThresholds::default(), &[5], &default_params());
        assert!(r.composite_score >= 0.0 && r.composite_score <= 1.0);
    }

    #[test]
    fn test_grid_search_with_sltp_limits_results() {
        let data = mk_data();
        let r = grid_search_with_sltp(&data, &[5], 20);
        assert!(r.len() <= 20);
        assert!(!r.is_empty());
        // Top result should have highest composite score
        for i in 0..r.len() - 1 {
            assert!(r[i].composite_score >= r[i + 1].composite_score);
        }
    }
}
