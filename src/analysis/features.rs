//! 历史 K 线特征工程
//!
//! ## 目标
//!
//! 把一段历史 K 线序列（`KlineSeries`）转换为固定维度的特征向量，
//! 供后续机器学习模型（LR / 决策树 / RF）训练和推理使用。
//!
//! ## 依赖
//!
//! 本模块纯函数：只依赖 model 层（`Kline` / `KlineSeries`），不发起 IO，
//! 不修改全局状态。输入 K 线序列 + 当前索引，输出固定 20 维特征。
//!
//! ## 特征清单
//!
//! | 序号 | 名称 | 含义 | 范围 |
//! |------|------|------|------|
//! |  0 | ma5_ratio | close / MA5 - 1 | 实数 |
//! |  1 | ma10_ratio | close / MA10 - 1 | 实数 |
//! |  2 | ma20_ratio | close / MA20 - 1 | 实数 |
//! |  3 | ma5_ma10_diff | (MA5 - MA10) / MA10 | 实数 |
//! |  4 | ma5_ma20_diff | (MA5 - MA20) / MA20 | 实数 |
//! |  5 | rsi14 | 14 日 RSI | [0, 100] |
//! |  6 | macd_hist | MACD - Signal | 实数 |
//! |  7 | bb_position | (close - lower) / (upper - lower) | [0, 1] |
//! |  8 | vol_ratio_20 | 当日量 / 20 日均量 | [0, +inf) |
//! |  9 | vol_ratio_5 | 当日量 / 5 日均量（短期放量检测）| [0, +inf) |
//! | 10 | volatility_20 | 20 日对数收益标准差（年化）| 实数 |
//! | 11 | momentum_5 | 5 日累计收益（%）| 实数 |
//! | 12 | momentum_20 | 20 日累计收益（%）| 实数 |
//! | 13 | price_position | (close - 20_low) / (20_high - 20_low) | [0, 1] |
//! | 14 | turnover_proxy | 成交额 / 自由流通市值（缺失时填 0）| 实数 |
//! | 15 | kdj_k | KDJ 的 K 值 | [0, 100] |
//! | 16 | kdj_d | KDJ 的 D 值 | [0, 100] |
//! | 17 | kdj_j | KDJ 的 J 值（放大波动）| 实数 |
//! | 18 | atr_ratio | ATR(14) / close（归一化波动率）| 实数 |
//! | 19 | ma_cross | MA5 > MA20 ? +1 : (MA5 < MA20 ? -1 : 0) | {-1,0,1} |

use crate::model::KlineSeries;

/// 特征维度数（与上表严格对应）
pub const NUM_FEATURES: usize = 20;

/// 特征名常量数组（顺序与 `Features::values` 严格对应）
pub const FEATURE_NAMES: [&str; NUM_FEATURES] = [
    "ma5_ratio",
    "ma10_ratio",
    "ma20_ratio",
    "ma5_ma10_diff",
    "ma5_ma20_diff",
    "rsi14",
    "macd_hist",
    "bb_position",
    "vol_ratio_20",
    "vol_ratio_5",
    "volatility_20",
    "momentum_5",
    "momentum_20",
    "price_position",
    "turnover_proxy",
    "kdj_k",
    "kdj_d",
    "kdj_j",
    "atr_ratio",
    "ma_cross",
];

/// 一条样本的 20 维特征向量
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Features {
    pub values: [f64; NUM_FEATURES],
}

impl Features {
    pub fn zeros() -> Self {
        Self { values: [0.0; NUM_FEATURES] }
    }

    pub fn to_vec(&self) -> Vec<f64> {
        self.values.to_vec()
    }

    /// 从 K 线序列的 idx 提取 20 维特征
    /// 需要至少 28 个交易日历史（KDJ 最严格需要 27 天）
    pub fn from_klines(
        series: &KlineSeries,
        idx: usize,
        turnover_proxy: Option<f64>,
    ) -> Option<Self> {
        if idx < 27 || idx >= series.bars.len() {
            return None;
        }

        let close_t = series.bars[idx].close;
        let bars = &series.bars;

        let ma5 = ma(bars, idx, 5)?;
        let ma10 = ma(bars, idx, 10)?;
        let ma20 = ma(bars, idx, 20)?;

        let ma5_ratio = ((close_t / ma5) - 1.0).clamp(-0.5, 0.5);
        let ma10_ratio = ((close_t / ma10) - 1.0).clamp(-0.5, 0.5);
        let ma20_ratio = ((close_t / ma20) - 1.0).clamp(-0.5, 0.5);
        let ma5_ma10_diff = if ma10.abs() > 1e-9 {
            ((ma5 - ma10) / ma10).clamp(-0.5, 0.5)
        } else {
            0.0
        };
        let ma5_ma20_diff = if ma20.abs() > 1e-9 {
            ((ma5 - ma20) / ma20).clamp(-0.5, 0.5)
        } else {
            0.0
        };

        let ma_cross = if ma5 > ma20 * 1.005 {
            1.0
        } else if ma5 < ma20 * 0.995 {
            -1.0
        } else {
            0.0
        };

        let rsi14 = rsi(bars, idx, 14).unwrap_or(50.0);
        let (macd_dif, macd_dea) = macd(bars, idx);
        let macd_hist = macd_dif - macd_dea;

        let (bb_upper, bb_lower, _bb_mid) = bollinger(bars, idx, 20, 2.0);
        let bb_position = if (bb_upper - bb_lower).abs() < 1e-9 {
            0.5
        } else {
            ((close_t - bb_lower) / (bb_upper - bb_lower)).clamp(0.0, 1.0)
        };

        let vol_ratio_20 = volume_ratio(bars, idx, 20).unwrap_or(1.0);
        let vol_ratio_5 = volume_ratio(bars, idx, 5).unwrap_or(1.0);
        let volatility_20 = stddev_log_return(bars, idx, 20).unwrap_or(0.0);
        let momentum_5 = cumulative_return(bars, idx, 5).unwrap_or(0.0);
        let momentum_20 = cumulative_return(bars, idx, 20).unwrap_or(0.0);

        let (hi20, lo20) = high_low_n(bars, idx, 20).unwrap_or((close_t, close_t));
        let price_position = if (hi20 - lo20).abs() < 1e-9 {
            0.5
        } else {
            ((close_t - lo20) / (hi20 - lo20)).clamp(0.0, 1.0)
        };

        // KDJ 预计算（一次性 O(n)，然后 O(1) 查找）
        let (kdj_kv, kdj_dv, kdj_jv) = {
            let (k_all, d_all, j_all) = precompute_kdj(bars, 9);
            kdj_at(idx, &k_all, &d_all, &j_all)
        };
        let atr = atr(bars, idx, 14).unwrap_or(0.0);
        let atr_ratio = if close_t > 1e-9 { atr / close_t } else { 0.0 };

        let turnover_proxy = turnover_proxy.unwrap_or(0.0).clamp(0.0, 1.0);

        Some(Features {
            values: [
                ma5_ratio,
                ma10_ratio,
                ma20_ratio,
                ma5_ma10_diff,
                ma5_ma20_diff,
                rsi14,
                macd_hist,
                bb_position,
                vol_ratio_20,
                vol_ratio_5,
                volatility_20,
                momentum_5,
                momentum_20,
                price_position,
                turnover_proxy,
                kdj_kv,
                kdj_dv,
                kdj_jv,
                atr_ratio,
                ma_cross,
            ],
        })
    }

    /// 从 K 线系列批量提取所有特征（一次性 O(n) 预计算所有指标）。
    /// 返回 Vec，索引对应 bars 索引；无效位置（历史不足或参数无效）为 None。
    /// 这是训练流程最高效的入口，比反复调用 from_klines 快 n 倍。
    pub fn extract_all_features(
        series: &KlineSeries,
        turnover_proxy: Option<f64>,
    ) -> Vec<Option<Features>> {
        let bars = &series.bars;
        let n = bars.len();
        let mut out = vec![None; n];
        if n < 28 {
            return out;
        }

        // 预计算 KDJ、ATR、DIF（一次性）
        let (kdj_k, kdj_d, kdj_j) = precompute_kdj(bars, 9);
        let atr_arr = compute_atr_array(bars, 14);
        let dif_vals: Vec<f64> = (0..n).map(|i| ema(bars, i, 12) - ema(bars, i, 26)).collect();
        let mut dea_vals = vec![0.0_f64; n];
        for i in 8..n {
            let start = if i < 9 { 0 } else { i + 1 - 9 };
            let slice = &dif_vals[start..=i];
            dea_vals[i] = slice.iter().sum::<f64>() / slice.len() as f64;
        }

        let tp = turnover_proxy.unwrap_or(0.0).clamp(0.0, 1.0);

        for idx in 27..n {
            let close_t = bars[idx].close;
            let ma5 = match ma(bars, idx, 5) {
                Some(v) => v,
                None => continue,
            };
            let ma10 = match ma(bars, idx, 10) {
                Some(v) => v,
                None => continue,
            };
            let ma20 = match ma(bars, idx, 20) {
                Some(v) => v,
                None => continue,
            };

            let ma5_ratio = ((close_t / ma5) - 1.0).clamp(-0.5, 0.5);
            let ma10_ratio = ((close_t / ma10) - 1.0).clamp(-0.5, 0.5);
            let ma20_ratio = ((close_t / ma20) - 1.0).clamp(-0.5, 0.5);
            let ma5_ma10_diff = if ma10.abs() > 1e-9 {
                ((ma5 - ma10) / ma10).clamp(-0.5, 0.5)
            } else { 0.0 };
            let ma5_ma20_diff = if ma20.abs() > 1e-9 {
                ((ma5 - ma20) / ma20).clamp(-0.5, 0.5)
            } else { 0.0 };

            let ma_cross = if ma5 > ma20 * 1.005 {
                1.0
            } else if ma5 < ma20 * 0.995 {
                -1.0
            } else {
                0.0
            };

            let rsi14 = rsi(bars, idx, 14).unwrap_or(50.0);
            let macd_dif = dif_vals[idx];
            let macd_dea = dea_vals[idx];
            let macd_hist = macd_dif - macd_dea;

            let (bb_upper, bb_lower, _) = bollinger(bars, idx, 20, 2.0);
            let bb_position = if (bb_upper - bb_lower).abs() < 1e-9 {
                0.5
            } else {
                ((close_t - bb_lower) / (bb_upper - bb_lower)).clamp(0.0, 1.0)
            };

            let vol_ratio_20 = volume_ratio(bars, idx, 20).unwrap_or(1.0);
            let vol_ratio_5 = volume_ratio(bars, idx, 5).unwrap_or(1.0);
            let volatility_20 = stddev_log_return(bars, idx, 20).unwrap_or(0.0);
            let momentum_5 = cumulative_return(bars, idx, 5).unwrap_or(0.0);
            let momentum_20 = cumulative_return(bars, idx, 20).unwrap_or(0.0);

            let (hi20, lo20) = high_low_n(bars, idx, 20).unwrap_or((close_t, close_t));
            let price_position = if (hi20 - lo20).abs() < 1e-9 {
                0.5
            } else {
                ((close_t - lo20) / (hi20 - lo20)).clamp(0.0, 1.0)
            };

            let (kdj_kv, kdj_dv, kdj_jv) = kdj_at(idx, &kdj_k, &kdj_d, &kdj_j);
            let atr_ratio = atr_arr[idx].unwrap_or(0.0) / close_t.max(1e-9);

            out[idx] = Some(Features {
                values: [
                    ma5_ratio, ma10_ratio, ma20_ratio,
                    ma5_ma10_diff, ma5_ma20_diff,
                    rsi14, macd_hist, bb_position,
                    vol_ratio_20, vol_ratio_5,
                    volatility_20, momentum_5, momentum_20,
                    price_position, tp,
                    kdj_kv, kdj_dv, kdj_jv,
                    atr_ratio, ma_cross,
                ],
            });
        }

        out
    }

}

// ==================== 私有指标函数 ====================

/// 简单移动平均：包含 idx 当日，往前 n 根（含 idx）的算术平均
fn ma(bars: &[crate::model::Kline], idx: usize, n: usize) -> Option<f64> {
    if idx + 1 < n { return None; }
    let sum: f64 = bars[idx + 1 - n..=idx].iter().map(|k| k.close).sum();
    Some(sum / n as f64)
}

/// 高低点窗口（包含 idx 当日）
fn high_low_n(bars: &[crate::model::Kline], idx: usize, n: usize) -> Option<(f64, f64)> {
    if idx + 1 < n { return None; }
    let win = &bars[idx + 1 - n..=idx];
    let hi = win.iter().map(|k| k.high).fold(f64::NEG_INFINITY, f64::max);
    let lo = win.iter().map(|k| k.low).fold(f64::INFINITY, f64::min);
    Some((hi, lo))
}

/// 量比 = 当日量 / 过去 n 日均量（含当日）
fn volume_ratio(bars: &[crate::model::Kline], idx: usize, n: usize) -> Option<f64> {
    if idx + 1 < n { return None; }
    let win = &bars[idx + 1 - n..=idx];
    let avg: f64 = win.iter().map(|k| k.volume as f64).sum::<f64>() / n as f64;
    let today_vol = bars[idx].volume as f64;
    if avg <= 0.0 { return Some(0.0); }
    Some(today_vol / avg)
}

/// 累计收益（%）：idx 当日 vs idx-n 当日收盘
fn cumulative_return(bars: &[crate::model::Kline], idx: usize, n: usize) -> Option<f64> {
    if idx < n { return None; }
    let prev = bars[idx - n].close;
    let cur = bars[idx].close;
    if prev <= 0.0 { return None; }
    Some((cur - prev) / prev * 100.0)
}

/// n 日对数收益标准差（年化：乘 sqrt(252)）
fn stddev_log_return(bars: &[crate::model::Kline], idx: usize, n: usize) -> Option<f64> {
    if idx < n || n < 2 { return None; }
    let win = &bars[idx + 1 - n..=idx];
    let rets: Vec<f64> = win.windows(2).filter_map(|w| {
        let prev = w[0].close;
        let cur = w[1].close;
        if prev > 0.0 && cur > 0.0 { Some((cur / prev).ln()) } else { None }
    }).collect();
    if rets.is_empty() { return None; }
    let mean = rets.iter().sum::<f64>() / rets.len() as f64;
    let var = rets.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / rets.len() as f64;
    Some(var.sqrt() * (252.0_f64).sqrt())
}

/// 经典 RSI（n=14）
fn rsi(bars: &[crate::model::Kline], idx: usize, n: usize) -> Option<f64> {
    if idx < n { return None; }
    let win = &bars[idx + 1 - n..=idx];
    let mut avg_gain = 0.0;
    let mut avg_loss = 0.0;
    for w in win.windows(2) {
        let chg = w[1].close - w[0].close;
        if chg > 0.0 { avg_gain += chg; } else { avg_loss += -chg; }
    }
    let count = (n - 1) as f64;
    if count <= 0.0 { return Some(50.0); }
    avg_gain /= count;
    avg_loss /= count;
    if avg_gain < 1e-12 && avg_loss < 1e-12 { return Some(50.0); }
    if avg_loss < 1e-12 { return Some(100.0); }
    let rs = avg_gain / avg_loss;
    Some(100.0 - 100.0 / (1.0 + rs))
}

/// MACD（12, 26, 9）返回 (DIF, DEA)
fn macd(bars: &[crate::model::Kline], idx: usize) -> (f64, f64) {
    if idx < 25 { return (0.0, 0.0); }
    let dif = ema(bars, idx, 12) - ema(bars, idx, 26);
    if idx < 33 { return (dif, 0.0); }
    let mut dif_vals = Vec::with_capacity(9);
    for i in (idx - 8)..=idx {
        dif_vals.push(ema(bars, i, 12) - ema(bars, i, 26));
    }
    let dea = dif_vals.iter().sum::<f64>() / 9.0;
    (dif, dea)
}

/// EMA：首期 SMA 初始化，之后 alpha = 2/(N+1) 递推
fn ema(bars: &[crate::model::Kline], idx: usize, n: usize) -> f64 {
    if idx + 1 < n { return 0.0; }
    let alpha = 2.0 / (n as f64 + 1.0);
    let start = idx + 1 - n;
    let mut prev: f64 = bars[start..=idx].iter().map(|k| k.close).sum::<f64>() / n as f64;
    for i in (start + 1)..=idx {
        prev = alpha * bars[i].close + (1.0 - alpha) * prev;
    }
    prev
}

/// 布林带：n 日 SMA +/- k * 标准差
fn bollinger(bars: &[crate::model::Kline], idx: usize, n: usize, k: f64) -> (f64, f64, f64) {
    if idx + 1 < n { return (0.0, 0.0, 0.0); }
    let win = &bars[idx + 1 - n..=idx];
    let mid: f64 = win.iter().map(|x| x.close).sum::<f64>() / n as f64;
    let var: f64 = win.iter().map(|x| (x.close - mid).powi(2)).sum::<f64>() / n as f64;
    let sd = var.sqrt();
    (mid + k * sd, mid - k * sd, mid)
}
/// KDJ 预计算：一次性 O(n) 迭代遍历整个系列。
/// n = RSV 窗口（标准 9）。返回 (k_vec, d_vec, j_vec)，对齐 bars 索引。
/// idx < n 时 K=D=J=50.0。
fn precompute_kdj(bars: &[crate::model::Kline], n: usize) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let len = bars.len();
    let mut k_vec = vec![50.0; len];
    let mut d_vec = vec![50.0; len];
    let mut j_vec = vec![50.0; len];
    if len <= n {
        return (k_vec, d_vec, j_vec);
    }
    // 第一个有效 idx = n：用窗口 bars[1..=n] 计算 RSV，K/D 初值 50
    let first_rsv = {
        let win = &bars[1..=n];
        let close = bars[n].close;
        let hi = win.iter().map(|k| k.high).fold(f64::NEG_INFINITY, f64::max);
        let lo = win.iter().map(|k| k.low).fold(f64::INFINITY, f64::min);
        if (hi - lo).abs() < 1e-9 {
            50.0
        } else {
            ((close - lo) / (hi - lo) * 100.0).clamp(0.0, 100.0)
        }
    };
    let k_n = (50.0 * 2.0 / 3.0 + first_rsv / 3.0).clamp(0.0, 100.0);
    let d_n = (50.0 * 2.0 / 3.0 + k_n / 3.0).clamp(0.0, 100.0);
    k_vec[n] = k_n;
    d_vec[n] = d_n;
    j_vec[n] = (3.0 * k_n - 2.0 * d_n).clamp(0.0, 100.0);
    // 迭代后续 idx：K = 2/3*prev_K + 1/3*RSV；D = 2/3*prev_D + 1/3*K
    for idx in (n + 1)..len {
        let win = &bars[idx + 1 - n..=idx];
        let close = bars[idx].close;
        let hi = win.iter().map(|k| k.high).fold(f64::NEG_INFINITY, f64::max);
        let lo = win.iter().map(|k| k.low).fold(f64::INFINITY, f64::min);
        let rsv = if (hi - lo).abs() < 1e-9 {
            50.0
        } else {
            ((close - lo) / (hi - lo) * 100.0).clamp(0.0, 100.0)
        };
        let prev_k = k_vec[idx - 1];
        let prev_d = d_vec[idx - 1];
        let k = (2.0 / 3.0 * prev_k + 1.0 / 3.0 * rsv).clamp(0.0, 100.0);
        let d = (2.0 / 3.0 * prev_d + 1.0 / 3.0 * k).clamp(0.0, 100.0);
        k_vec[idx] = k;
        d_vec[idx] = d;
        j_vec[idx] = (3.0 * k - 2.0 * d).clamp(0.0, 100.0);
    }
    (k_vec, d_vec, j_vec)
}

/// KDJ 单点查询（调用方需保证 precompute_kdj 已预计算）。
fn kdj_at(idx: usize, k_vec: &[f64], d_vec: &[f64], j_vec: &[f64]) -> (f64, f64, f64) {
    if idx < k_vec.len() {
        (k_vec[idx], d_vec[idx], j_vec[idx])
    } else {
        (50.0, 50.0, 50.0)
    }
}

/// ATR 预计算：一次性 O(n) 求每个 idx 的 ATR(14)。
fn compute_atr_array(bars: &[crate::model::Kline], n: usize) -> Vec<Option<f64>> {
    let len = bars.len();
    let mut out = vec![None; len];
    if len <= n {
        return out;
    }
    let tr_vals: Vec<f64> = (0..len)
        .map(|i| {
            if i == 0 {
                bars[i].high - bars[i].low
            } else {
                let prev_close = bars[i - 1].close;
                (bars[i].high - bars[i].low).max((bars[i].high - prev_close).abs()).max((bars[i].low - prev_close).abs())
            }
        })
        .collect();
    for idx in n..len {
        let sum: f64 = tr_vals[idx + 1 - n..=idx].iter().sum();
        out[idx] = Some(sum / n as f64);
    }
    out
}


/// ATR（Average True Range）：真实波动的指数移动平均
/// True Range = max(H-L, |H-PrevClose|, |L-PrevClose|)
fn atr(bars: &[crate::model::Kline], idx: usize, n: usize) -> Option<f64> {
    if idx < n { return None; }
    let mut tr_sum = 0.0;
    let mut count = 0;
    for i in (idx + 1 - n)..=idx {
        let tr = if i == 0 {
            bars[i].high - bars[i].low
        } else {
            let prev_close = bars[i - 1].close;
            let h_l = bars[i].high - bars[i].low;
            let h_pc = (bars[i].high - prev_close).abs();
            let l_pc = (bars[i].low - prev_close).abs();
            h_l.max(h_pc).max(l_pc)
        };
        tr_sum += tr;
        count += 1;
    }
    if count < n { return None; }
    Some(tr_sum / count as f64)
}


/// 市场状态检测：基于 MA 多头排列判断
///
/// - bull:    MA5 > MA10 > MA20（短期均线全部在长期均线上方）
/// - bear:   MA5 < MA10 < MA20（空头排列）
/// - neutral: 其他情况（震荡市）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketRegime {
    Bull,
    Bear,
    Neutral,
}

/// 检测市场状态
pub fn detect_regime(bars: &[crate::model::Kline], idx: usize) -> MarketRegime {
    let ma5 = ma(bars, idx, 5).unwrap_or(0.0);
    let ma10 = ma(bars, idx, 10).unwrap_or(0.0);
    let ma20 = ma(bars, idx, 20).unwrap_or(0.0);
    if ma5 > ma10 && ma10 > ma20 {
        MarketRegime::Bull
    } else if ma5 < ma10 && ma10 < ma20 {
        MarketRegime::Bear
    } else {
        MarketRegime::Neutral
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Kline, KlineSeries};
    use chrono::NaiveDate;

    fn trending_series() -> KlineSeries {
        let mut bars = Vec::with_capacity(35);
        let mut p = 100.0;
        for i in 0..35 {
            let date = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()
                + chrono::Duration::days(i as i64);
            let open = p;
            let close = p * 1.005;
            let high = close + 0.3;
            let low = open - 0.2;
            bars.push(Kline { date, open, high, low, close, volume: 1_000_000 + i as i64 * 1000 });
            p = close;
        }
        KlineSeries::new(bars)
    }

    fn flat_series() -> KlineSeries {
        let mut bars = Vec::with_capacity(35);
        for i in 0..35 {
            let date = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()
                + chrono::Duration::days(i as i64);
            bars.push(Kline { date, open: 100.0, high: 100.5, low: 99.5, close: 100.0, volume: 1_000_000 });
        }
        KlineSeries::new(bars)
    }

    #[test]
    fn test_feature_names_count_matches_dim() {
        assert_eq!(FEATURE_NAMES.len(), NUM_FEATURES);
    }

    #[test]
    fn test_features_from_klines_flat_shape() {
        let s = flat_series();
        let f = Features::from_klines(&s, 27, None).unwrap();
        assert!((f.values[5] - 50.0).abs() < 1e-4, "rsi14={}", f.values[5]);
        assert!((f.values[8] - 1.0).abs() < 1e-4, "vol_ratio={}", f.values[8]);
        assert!((f.values[7] - 0.5).abs() < 1e-4, "bb_pos={}", f.values[7]);
        assert_eq!(f.values[14], 0.0);
    }

    #[test]
    fn test_features_from_klines_uptrend() {
        let s = trending_series();
        let f = Features::from_klines(&s, 30, Some(0.02)).unwrap();
        assert!(f.values[0] > 0.0, "ma5_ratio={}", f.values[0]);
        assert!(f.values[5] > 80.0, "rsi14={}", f.values[5]);
        assert!(f.values[10] > 0.0, "momentum_5={}", f.values[10]);
        assert_eq!(f.values[14], 0.02);
        assert_eq!(f.values[19], 1.0, "ma_cross={}", f.values[19]);
    }

    #[test]
    fn test_features_insufficient_history() {
        let s = trending_series();
        assert!(Features::from_klines(&s, 5, None).is_none());
        assert!(Features::from_klines(&s, 26, None).is_none());
        assert!(Features::from_klines(&s, 27, None).is_some());
    }

    #[test]
    fn test_kdj_below_oversold() {
        let mut bars = Vec::with_capacity(35);
        let mut p = 100.0;
        for i in 0..35 {
            let date = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()
                + chrono::Duration::days(i as i64);
            p *= 0.97;
            bars.push(Kline { date, open: p * 1.01, high: p * 1.02, low: p * 0.98, close: p, volume: 1_000_000 });
        }
        let s = KlineSeries::new(bars);
        let f = Features::from_klines(&s, 30, None).unwrap();
        assert!(f.values[15] < 30.0, "kdj_k={} (expect low)", f.values[15]);
        assert!(f.values[19] < 0.0, "ma_cross={}", f.values[19]);
    }

    #[test]
    fn test_atr_positive() {
        let s = trending_series();
        let f = Features::from_klines(&s, 30, None).unwrap();
        assert!(f.values[18] > 0.0, "atr_ratio={}", f.values[18]);
    }

    #[test]
    fn test_vol_ratio_5_short_spike() {
        // 35 bars, spike at bar 30 (index 30 = 10x vol). Use idx=30
        // window = bars 26-30: 4 bars normal + 1 bar spike
        let mut bars = Vec::with_capacity(35);
        for i in 0..35 {
            let date = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()
                + chrono::Duration::days(i as i64);
            let vol = if i < 30 { 100_000 } else { 1_000_000 };
            bars.push(Kline { date, open: 100.0, high: 100.5, low: 99.5, close: 100.0, volume: vol });
        }
        let s = KlineSeries::new(bars);
        // idx=30: window bars 26-30 includes the spike at index 30
        let f = Features::from_klines(&s, 30, None).unwrap();
        // avg of 5 bars = (4*100k + 1*1000k)/5 = 280k, ratio = 1000k/280k ≈ 3.57
        assert!(f.values[9] > 3.0, "vol_ratio_5={} (expect > 3)", f.values[9]);
    }
}
