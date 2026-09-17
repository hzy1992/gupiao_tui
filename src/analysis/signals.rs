//! 综合信号打分与买/卖点预测

use crate::model::Quote;

use super::indicators::{
    ask_resistance, bid_support, Indicators, PriceLevel, PriceLevelSource,
};
use super::infer::{fuse_score, InferenceContext, FusionStrategy};

/// 综合分析报告
#[derive(Debug, Clone)]
pub struct AnalysisReport {
    pub symbol_code: String,
    pub symbol_name: String,
    pub market: String,
    pub current_price: f64,
    pub change_pct: f64,
    pub indicators: Indicators,
    pub signal: Signal,
    pub support: Option<PriceLevel>,
    pub resistance: Option<PriceLevel>,
    pub advice: Vec<String>,
    pub key_factors: Vec<String>,
    pub model_prob: Option<f64>,
    pub fusion_strategy: Option<FusionStrategy>,
    pub rule_score: f64,
    pub final_score: f64,
    pub news: Option<crate::model::StockNews>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Signal {
    pub kind: SignalKind,
    pub strength: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SignalKind {
    StrongBuy,
    Buy,
    Hold,
    Sell,
    StrongSell,
}

impl SignalKind {
    pub fn label(self) -> &'static str {
        match self {
            SignalKind::StrongBuy => "强烈买入",
            SignalKind::Buy => "买入",
            SignalKind::Hold => "观望",
            SignalKind::Sell => "卖出",
            SignalKind::StrongSell => "强烈卖出",
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            SignalKind::StrongBuy => "STRONG_BUY",
            SignalKind::Buy => "BUY",
            SignalKind::Hold => "HOLD",
            SignalKind::Sell => "SELL",
            SignalKind::StrongSell => "STRONG_SELL",
        }
    }
}

pub fn analyze(q: &Quote) -> AnalysisReport {
    analyze_with(q, &ScoreWeights::default(), &ScoreThresholds::default())
}

pub fn analyze_with(
    q: &Quote,
    weights: &ScoreWeights,
    thresholds: &ScoreThresholds,
) -> AnalysisReport {
    analyze_full(q, weights, thresholds, &InferenceContext::rules_only())
}

pub fn analyze_with_model(
    q: &Quote,
    weights: &ScoreWeights,
    thresholds: &ScoreThresholds,
    kline_series: &crate::model::KlineSeries,
    ctx: &InferenceContext,
) -> AnalysisReport {
    analyze_full(q, weights, thresholds, ctx).with_model_inference(q, kline_series, ctx)
}

fn analyze_full(
    q: &Quote,
    weights: &ScoreWeights,
    thresholds: &ScoreThresholds,
    _ctx: &InferenceContext,
) -> AnalysisReport {
    let indicators = Indicators::from_quote(q);
    let (signal, raw_score, factor_breakdown) = score_with(&indicators, q, weights, thresholds);
    let (support, resistance) = predict_levels(&q.depth);
    let advice = build_advice(q, &signal, &support, &resistance);

    let mut factors: Vec<(String, f64)> = factor_breakdown
        .into_iter()
        .map(|(k, v)| (k, v.abs()))
        .collect();
    factors.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let key_factors: Vec<String> = factors.into_iter().take(3).map(|(k, _)| k).collect();

    AnalysisReport {
        symbol_code: q.code.clone(),
        symbol_name: q.name.clone(),
        market: q.market.code().to_string(),
        current_price: q.price,
        change_pct: q.change_pct,
        indicators,
        signal,
        support,
        resistance,
        advice,
        key_factors,
        model_prob: None,
        fusion_strategy: None,
        rule_score: raw_score,
        final_score: raw_score,
        news: None,
    }
}

impl AnalysisReport {
    pub fn with_model_inference(
        mut self,
        q: &Quote,
        kline_series: &crate::model::KlineSeries,
        ctx: &InferenceContext,
    ) -> Self {
        if !ctx.uses_model() {
            return self;
        }
        let n = kline_series.bars.len();
        if n < 20 {
            self.fusion_strategy = Some(ctx.strategy);
            return self;
        }

        let features = match crate::analysis::Features::from_klines(kline_series, n - 1, None) {
            Some(f) => f,
            None => return self,
        };

        let prob = ctx.model.as_ref().unwrap().predict_proba(&features.values);

        self.model_prob = Some(prob);
        self.fusion_strategy = Some(ctx.strategy);

        let final_score = fuse_score(self.rule_score, prob, ctx.strategy, ctx.alpha);
        self.final_score = final_score;

        let (kind, strength) = score_to_strength(final_score, &ScoreThresholds::default());
        self.signal = Signal { kind, strength };

        self
    }

    pub fn with_news(mut self, news: crate::model::StockNews) -> Self {
        self.news = Some(news);
        self
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ScoreWeights {
    pub pct: f64,
    pub position: f64,
    pub committee: f64,
    pub volume: f64,
    pub pv: f64,
    pub trend: f64,
}

impl ScoreWeights {
    pub fn default() -> Self {
        Self {
            pct: 25.0,
            position: 10.0,
            committee: 20.0,
            volume: 15.0,
            pv: 15.0,
            trend: 15.0,
        }
    }

    pub fn optimal() -> Self {
        Self {
            pct: 50.0,
            position: 25.0,
            committee: 25.0,
            volume: 25.0,
            pv: 25.0,
            trend: 0.0,
        }
    }
}

impl Default for ScoreWeights {
    fn default() -> Self {
        Self::default()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ScoreThresholds {
    pub strong_buy: f64,
    pub buy: f64,
    pub sell: f64,
    pub strong_sell: f64,
}

impl Default for ScoreThresholds {
    fn default() -> Self {
        Self {
            strong_buy: 60.0,
            buy: 30.0,
            sell: -30.0,
            strong_sell: -60.0,
        }
    }
}

pub fn score_to_strength(score: f64, thresholds: &ScoreThresholds) -> (SignalKind, u8) {
    let strength = ((score.abs() / 100.0) * 100.0).round() as u8;
    let kind = if score >= thresholds.strong_buy {
        SignalKind::StrongBuy
    } else if score >= thresholds.buy {
        SignalKind::Buy
    } else if score <= thresholds.strong_sell {
        SignalKind::StrongSell
    } else if score <= thresholds.sell {
        SignalKind::Sell
    } else {
        SignalKind::Hold
    };
    (kind, strength.min(100))
}

fn score_with(
    ind: &Indicators,
    q: &Quote,
    weights: &ScoreWeights,
    _thresholds: &ScoreThresholds,
) -> (Signal, f64, Vec<(String, f64)>) {
    let change_score = if ind.position > 0.7 {
        q.change_pct * 2.0
    } else if ind.position < 0.3 {
        q.change_pct * 0.5
    } else {
        q.change_pct
    };

    let volume_score = if ind.volume_ratio > 1.5 {
        (ind.volume_ratio - 1.0) * 20.0
    } else if ind.volume_ratio < 0.5 {
        (ind.volume_ratio - 1.0) * 10.0
    } else {
        0.0
    };

    let committee_score = ind.committee_ratio * 30.0;
    let trend_score = ind.trend_strength * 15.0;

    let pv_score = if ind.pv_ratio > 1.0 {
        (ind.pv_ratio - 1.0).ln() * 10.0
    } else {
        -5.0
    };

    let total = change_score * weights.pct / 100.0
        + volume_score * weights.volume / 100.0
        + committee_score * weights.committee / 100.0
        + trend_score * weights.trend / 100.0
        + pv_score * weights.pv / 100.0;

    let factors = vec![
        ("涨跌幅".into(), change_score * weights.pct / 100.0),
        ("量比".into(), volume_score * weights.volume / 100.0),
        ("委比".into(), committee_score * weights.committee / 100.0),
        ("趋势强度".into(), trend_score * weights.trend / 100.0),
        ("价量比".into(), pv_score * weights.pv / 100.0),
    ];

    let (kind, strength) = score_to_strength(total, &ScoreThresholds::default());
    (Signal { kind, strength }, total, factors)
}

fn predict_levels(depth: &crate::model::Depth) -> (Option<PriceLevel>, Option<PriceLevel>) {
    let support = bid_support(depth);
    let resistance = ask_resistance(depth);
    (support, resistance)
}

fn build_advice(
    q: &Quote,
    signal: &Signal,
    support: &Option<PriceLevel>,
    resistance: &Option<PriceLevel>,
) -> Vec<String> {
    let mut lines = vec![];

    let score = match signal.kind {
        SignalKind::StrongBuy => format!("  综合得分 {:+.1}  多头共振", 50.0 + signal.strength as f64 * 0.5),
        SignalKind::Buy => format!("  综合得分 {:+.1}  偏多信号", 20.0 + signal.strength as f64 * 0.3),
        SignalKind::Hold => format!("  综合得分 0.0  震荡整理"),
        SignalKind::Sell => format!("  综合得分 {:+.1}  偏空信号", -20.0 - signal.strength as f64 * 0.3),
        SignalKind::StrongSell => format!("  综合得分 {:+.1}  空头共振", -50.0 - signal.strength as f64 * 0.5),
    };
    lines.push(score);

    let advice_line = match signal.kind {
        SignalKind::StrongBuy => "  建议操作: 强烈买入，分批建仓，跌破支撑严格止损",
        SignalKind::Buy => "  建议操作: 买入，可考虑回调至支撑位附近介入",
        SignalKind::Hold => "  建议操作: 观望，等待方向明确，不盲目追涨杀跌",
        SignalKind::Sell => "  建议操作: 减仓，逢反弹卖出，避免逆势加仓",
        SignalKind::StrongSell => "  建议操作: 强烈卖出，尽早离场，不盲目抄底",
    };
    lines.push(advice_line.to_string());

    if let Some(s) = support {
        let dist = (q.price - s.price) / q.price * 100.0;
        lines.push(format!("  参考买点: {:.2}（距现价 {:.2}%，{}）", s.price, dist, s.source.label()));
    }

    if let Some(r) = resistance {
        let upside = (r.price - q.price) / q.price * 100.0;
        lines.push(format!("  参考卖点: {:.2}（距现价 {:.2}%，{}）", r.price, upside, r.source.label()));
    }

    if let Some(amp) = q.amplitude {
        if amp > 0.0 {
            let stop_loss = q.price * (1.0 - amp / 200.0);
            let take_profit = q.price * (1.0 + amp / 200.0);
            lines.push(format!("  参考止损: {:.2} / 参考止盈: {:.2}", stop_loss, take_profit));
        }
    }

    lines.push("  ⚠  股市有风险，本分析仅供参考，不构成投资建议".to_string());
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Depth, Market, Quote};
    use chrono::Local;

    fn mk_quote_full(
        price: f64,
        high: f64,
        low: f64,
        open: f64,
        prev_close: f64,
        amount: f64,
        amplitude: f64,
        bid_p: [f64; 5],
        bid_v: [i64; 5],
        ask_p: [f64; 5],
        ask_v: [i64; 5],
    ) -> Quote {
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
            volume: 50000,
            amount,
            turnover_rate: Some(1.0),
            pe: Some(20.0),
            pb: Some(5.0),
            market_cap: Some(16451.20),
            float_cap: Some(16451.20),
            amplitude: Some(amplitude),
            update_time: Local::now(),
            depth: Depth {
                bid_prices: bid_p.map(Some),
                bid_vols: bid_v.map(Some),
                ask_prices: ask_p.map(Some),
                ask_vols: ask_v.map(Some),
            },
        }
    }

    #[test]
    fn test_strong_bullish_scenario() {
        let q = mk_quote_full(
            110.0, 112.0, 95.0, 96.0, 100.0, 5e9, 17.0,
            [109.5, 109.0, 108.5, 108.0, 107.5],
            [5000, 3000, 2000, 1000, 500],
            [110.5, 111.0, 111.5, 112.0, 112.5],
            [500, 800, 1000, 1200, 1500],
        );
        let r = analyze(&q);
        assert!(matches!(r.signal.kind, SignalKind::Buy | SignalKind::StrongBuy));
        assert!(r.support.is_some());
        assert!(r.resistance.is_some());
    }

    #[test]
    fn test_strong_bearish_scenario() {
        let q = mk_quote_full(
            85.0, 95.0, 84.0, 95.0, 100.0, 5e9, 11.0,
            [84.5, 84.0, 83.5, 83.0, 82.5],
            [500, 800, 1000, 1200, 1500],
            [85.5, 86.0, 86.5, 87.0, 87.5],
            [5000, 3000, 2000, 1000, 500],
        );
        let r = analyze(&q);
        assert!(matches!(r.signal.kind, SignalKind::Sell | SignalKind::StrongSell));
    }

    #[test]
    fn test_hold_scenario() {
        let q = mk_quote_full(
            100.0, 102.0, 98.0, 100.0, 100.0, 5e8, 4.0,
            [99.5, 99.0, 98.5, 98.0, 97.5],
            [1000; 5],
            [100.5, 101.0, 101.5, 102.0, 102.5],
            [1000; 5],
        );
        let r = analyze(&q);
        assert_eq!(r.signal.kind, SignalKind::Hold);
    }

    #[test]
    fn test_signal_labels() {
        assert_eq!(SignalKind::StrongBuy.label(), "强烈买入");
        assert_eq!(SignalKind::StrongSell.label(), "强烈卖出");
        assert_eq!(SignalKind::StrongSell.short(), "STRONG_SELL");
    }

    #[test]
    fn test_predict_levels_with_depth() {
        let q = mk_quote_full(
            105.0, 110.0, 100.0, 102.0, 103.0, 1e9, 9.5,
            [104.5, 104.0, 103.5, 103.0, 102.5],
            [1000; 5],
            [105.5, 106.0, 106.5, 107.0, 107.5],
            [1000; 5],
        );
        let r = analyze(&q);
        if let Some(s) = r.support {
            assert!(s.price < q.price);
        }
        if let Some(rs) = r.resistance {
            assert!(rs.price > q.price);
        }
    }

    #[test]
    fn test_key_factors_top_three() {
        let q = mk_quote_full(
            108.0, 112.0, 95.0, 96.0, 100.0, 5e9, 17.0,
            [107.5, 107.0, 106.5, 106.0, 105.5],
            [5000, 3000, 2000, 1000, 500],
            [108.5, 109.0, 109.5, 110.0, 110.5],
            [500, 800, 1000, 1200, 1500],
        );
        let r = analyze(&q);
        assert!(r.key_factors.len() <= 3);
        assert!(!r.key_factors.is_empty());
    }

    #[test]
    fn test_signal_strength_in_range() {
        let q = mk_quote_full(
            110.0, 112.0, 95.0, 96.0, 100.0, 5e9, 17.0,
            [109.5; 5],
            [5000; 5],
            [110.5; 5],
            [500; 5],
        );
        let r = analyze(&q);
        assert!(r.signal.strength <= 100);
    }
}