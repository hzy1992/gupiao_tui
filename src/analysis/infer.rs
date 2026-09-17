//! ML 模型与规则打分的融合策略
//!
//! ## 设计
//!
//! 给定：
//! - 规则打分 rule_score ∈ [-100, +100]（analyze_with 已算出）
//! - 模型概率 model_prob ∈ [0, 1] = P(Up)
//! - 特征向量 Features（20 维）
//!
//! 输出最终 final_score ∈ [-100, +100]，传给原阈值映射（StrongBuy/...）。
//!
//! ## 策略
//!
//! - RulesOnly：忽略模型，final_score = rule_score
//! - ModelOnly：忽略规则，final_score = (model_prob - 0.5) * 200
//! - Weighted：final_score = (1-α)*rule + α*model
//!
//! 默认 α = 0.4（模型权重略低，因为规则打分有当日盘口信息补充）

use serde::{Deserialize, Serialize};

use crate::model::ml::LoadedModel;

/// 推理融合策略
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FusionStrategy {
    /// 仅使用规则打分（不加载模型）
    RulesOnly,
    /// 仅使用模型（完全替代规则打分）
    ModelOnly,
    /// 加权融合：final_score = (1-α)*rule + α*model
    Weighted,
}

impl Default for FusionStrategy {
    fn default() -> Self {
        FusionStrategy::Weighted
    }
}

impl FusionStrategy {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "rules" | "rules-only" | "rulesonly" => Some(FusionStrategy::RulesOnly),
            "model" | "model-only" | "modelonly" => Some(FusionStrategy::ModelOnly),
            "weighted" | "w" => Some(FusionStrategy::Weighted),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FusionStrategy::RulesOnly => "rules",
            FusionStrategy::ModelOnly => "model",
            FusionStrategy::Weighted => "weighted",
        }
    }
}

/// 模型推理的上下文
#[derive(Debug, Clone)]
pub struct InferenceContext {
    pub model: Option<LoadedModel>,
    pub strategy: FusionStrategy,
    pub alpha: f64,
}

impl InferenceContext {
    pub fn rules_only() -> Self {
        Self { model: None, strategy: FusionStrategy::RulesOnly, alpha: 0.4 }
    }

    pub fn with_model(model: LoadedModel, strategy: FusionStrategy, alpha: f64) -> Self {
        Self { model: Some(model), strategy, alpha: alpha.clamp(0.0, 1.0) }
    }

    pub fn uses_model(&self) -> bool {
        self.model.is_some() && self.strategy != FusionStrategy::RulesOnly
    }
}

pub fn fuse_score(rule_score: f64, model_prob: f64, strategy: FusionStrategy, alpha: f64) -> f64 {
    let model_score = (model_prob - 0.5) * 200.0;
    match strategy {
        FusionStrategy::RulesOnly => rule_score,
        FusionStrategy::ModelOnly => model_score,
        FusionStrategy::Weighted => {
            let a = alpha.clamp(0.0, 1.0);
            (1.0 - a) * rule_score + a * model_score
        }
    }
}

pub fn score_to_strength(final_score: f64) -> u8 {
    ((final_score + 100.0) / 2.0).clamp(0.0, 100.0).round() as u8
}

pub fn model_predict(model: &LoadedModel, features: &[f64; 20]) -> f64 {
    model.predict_proba(features)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::features::NUM_FEATURES;

    #[test]
    fn test_fusion_strategy_default() {
        assert_eq!(FusionStrategy::default(), FusionStrategy::Weighted);
    }

    #[test]
    fn test_from_str() {
        assert_eq!(FusionStrategy::from_str("rules"), Some(FusionStrategy::RulesOnly));
        assert_eq!(FusionStrategy::from_str("model"), Some(FusionStrategy::ModelOnly));
        assert_eq!(FusionStrategy::from_str("weighted"), Some(FusionStrategy::Weighted));
        assert_eq!(FusionStrategy::from_str("nope"), None);
    }

    #[test]
    fn test_label() {
        assert_eq!(FusionStrategy::RulesOnly.label(), "rules");
        assert_eq!(FusionStrategy::Weighted.label(), "weighted");
    }

    #[test]
    fn test_fuse_score_rules_only() {
        assert_eq!(fuse_score(50.0, 0.9, FusionStrategy::RulesOnly, 0.5), 50.0);
    }

    #[test]
    fn test_fuse_score_model_only() {
        assert!((fuse_score(50.0, 1.0, FusionStrategy::ModelOnly, 0.5) - 100.0).abs() < 1e-9);
        assert!((fuse_score(50.0, 0.5, FusionStrategy::ModelOnly, 0.5) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn test_fuse_score_weighted() {
        assert!((fuse_score(50.0, 1.0, FusionStrategy::Weighted, 0.4) - 70.0).abs() < 1e-9);
        assert!((fuse_score(50.0, 0.9, FusionStrategy::Weighted, 0.0) - 50.0).abs() < 1e-9);
    }

    #[test]
    fn test_score_to_strength() {
        assert_eq!(score_to_strength(100.0), 100);
        assert_eq!(score_to_strength(-100.0), 0);
        assert_eq!(score_to_strength(0.0), 50);
    }

    #[test]
    fn test_inference_context_with_model() {
        use crate::model::ml::{DtNode, DtParams, ModelKind, TrainReport, TrainedModel};
        let model = TrainedModel::Dt(DtParams {
            root: DtNode::Leaf { prob: 0.7, n: 5 },
            n_classes: 2,
        });
        let report = TrainReport {
            kind: ModelKind::DecisionTree,
            hold_days: 5,
            threshold_pct: 0.5,
            trained_at: 0,
            train_samples: 100,
            n_features: NUM_FEATURES,
            cv_accuracies: vec![0.55; 4],
            cv_mean: 0.55,
            cv_min: 0.55,
            cv_max: 0.55,
            train_accuracy: 0.6,
            train_codes: vec!["600519".into()] as Vec<String>,
            feature_importance: None,
        };
        let loaded = LoadedModel { id: 1, kind: ModelKind::DecisionTree, hold_days: 5,
            threshold_pct: 0.5, model, report };
        let ctx = InferenceContext::with_model(loaded, FusionStrategy::Weighted, 0.4);
        assert!(ctx.uses_model());
        let mut x = [0.0; 20];
        x[0] = 0.5;
        assert!((model_predict(ctx.model.as_ref().unwrap(), &x) - 0.7).abs() < 1e-9);
    }
}
