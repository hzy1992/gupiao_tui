//! 机器学习相关的数据类型
//!
//! ## 设计原则
//!
//! - 纯数据 + 枚举，不持有任何 IO / 全局状态
//! - 所有结构都派生 Serialize/Deserialize，便于存到 SQLite
//! - 模型参数采用我们自己的稳定表示（不直接序列化 smartcore 内部结构）
//!
//! ## 依赖
//!
//! model 层不依赖 nalysis 或 smartcore，仅依赖 serde / serde_json。

use serde::{Deserialize, Serialize};

/// 模型种类
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModelKind {
    LogisticRegression,
    DecisionTree,
    RandomForest,
}

impl ModelKind {
    pub fn label(self) -> &'static str {
        match self {
            ModelKind::LogisticRegression => "LR",
            ModelKind::DecisionTree => "DT",
            ModelKind::RandomForest => "RF",
        }
    }

    pub fn full_name(self) -> &'static str {
        match self {
            ModelKind::LogisticRegression => "逻辑回归",
            ModelKind::DecisionTree => "决策树",
            ModelKind::RandomForest => "随机森林",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "lr" | "logistic" | "logisticregression" => Some(ModelKind::LogisticRegression),
            "dt" | "tree" | "decisiontree" => Some(ModelKind::DecisionTree),
            "rf" | "forest" | "randomforest" => Some(ModelKind::RandomForest),
            _ => None,
        }
    }
}

/// 训练配置
#[derive(Debug, Clone)]
pub struct TrainConfig {
    pub kind: ModelKind,
    pub hold_days: usize,
    pub threshold_pct: f64,
    pub n_splits: usize,
    pub min_samples: usize,
    pub rf_trees: usize,
}

impl Default for TrainConfig {
    fn default() -> Self {
        Self {
            kind: ModelKind::RandomForest,
            hold_days: 5,
            threshold_pct: 0.5,
            n_splits: 4,
            min_samples: 200,
            rf_trees: 50,
        }
    }
}

/// 训练报告（持久化到 SQLite）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrainReport {
    pub kind: ModelKind,
    pub hold_days: usize,
    pub threshold_pct: f64,
    pub trained_at: i64,
    pub train_samples: usize,
    pub n_features: usize,
    pub cv_accuracies: Vec<f64>,
    pub cv_mean: f64,
    pub cv_min: f64,
    pub cv_max: f64,
    pub train_accuracy: f64,
    pub train_codes: Vec<String>,
    /// 特征重要性（归一化到和为1），与 FEATURE_NAMES 一一对应
    pub feature_importance: Option<Vec<f64>>,
}

/// 已训练模型的参数
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TrainedModel {
    Lr(LrParams),
    Dt(DtParams),
    Rf(RfParams),
}

/// 逻辑回归参数
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LrParams {
    pub coefficients: Vec<f64>,
    pub intercept: f64,
}

/// 决策树参数（递归结构）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DtNode {
    Leaf {
        prob: f64,
        n: usize,
    },
    Split {
        feature: usize,
        threshold: f64,
        left: Box<DtNode>,
        right: Box<DtNode>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DtParams {
    pub root: DtNode,
    pub n_classes: usize,
}

/// 随机森林参数
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RfParams {
    pub trees: Vec<DtParams>,
    pub n_classes: usize,
}

/// 已加载到内存的模型 + 元数据
#[derive(Debug, Clone)]
pub struct LoadedModel {
    pub id: i64,
    pub kind: ModelKind,
    pub hold_days: usize,
    pub threshold_pct: f64,
    pub model: TrainedModel,
    pub report: TrainReport,
}

impl LoadedModel {
    /// 推理：返回 P(Up) ∈ [0, 1]
    /// 注意：features 必须与训练时的特征维度一致（当前 20 维）
    pub fn predict_proba(&self, features: &[f64; 20]) -> f64 {
        predict_proba(&self.model, features)
    }
}

/// 模型推理统一入口：返回 P(Up) ∈ [0, 1]
pub fn predict_proba(model: &TrainedModel, features: &[f64]) -> f64 {
    match model {
        TrainedModel::Lr(p) => predict_lr(p, features),
        TrainedModel::Dt(p) => predict_dt(&p.root, features),
        TrainedModel::Rf(p) => {
            if p.trees.is_empty() {
                return 0.5;
            }
            let sum: f64 = p.trees.iter().map(|t| predict_dt(&t.root, features)).sum();
            sum / p.trees.len() as f64
        }
    }
}

fn predict_lr(p: &LrParams, features: &[f64]) -> f64 {
    let z: f64 = features
        .iter()
        .zip(p.coefficients.iter())
        .map(|(x, w)| x * w)
        .sum::<f64>()
        + p.intercept;
    sigmoid(z)
}

/// 数值稳定版 sigmoid
pub fn sigmoid(z: f64) -> f64 {
    if z >= 0.0 {
        1.0 / (1.0 + (-z).exp())
    } else {
        let e = z.exp();
        e / (1.0 + e)
    }
}

fn predict_dt(node: &DtNode, features: &[f64]) -> f64 {
    match node {
        DtNode::Leaf { prob, .. } => *prob,
        DtNode::Split {
            feature,
            threshold,
            left,
            right,
        } => {
            let v = features.get(*feature).copied().unwrap_or(0.0);
            if v <= *threshold {
                predict_dt(left, features)
            } else {
                predict_dt(right, features)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_kind_from_str() {
        assert_eq!(ModelKind::from_str("LR"), Some(ModelKind::LogisticRegression));
        assert_eq!(ModelKind::from_str("logistic"), Some(ModelKind::LogisticRegression));
        assert_eq!(ModelKind::from_str("DT"), Some(ModelKind::DecisionTree));
        assert_eq!(ModelKind::from_str("tree"), Some(ModelKind::DecisionTree));
        assert_eq!(ModelKind::from_str("RF"), Some(ModelKind::RandomForest));
        assert_eq!(ModelKind::from_str("nope"), None);
    }

    #[test]
    fn test_sigmoid_zero_is_half() {
        assert!((sigmoid(0.0) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn test_sigmoid_extremes() {
        assert!(sigmoid(100.0) > 0.9999);
        assert!(sigmoid(-100.0) < 0.0001);
    }

    #[test]
    fn test_predict_lr_basic() {
        let p = LrParams {
            coefficients: vec![1.0; 20],
            intercept: 0.0,
        };
        let mut x = [0.0; 20];
        x[0] = 1.0;
        let prob = predict_lr(&p, &x);
        assert!((prob - 0.7310585786).abs() < 1e-6, "prob={}", prob);
    }

    #[test]
    fn test_predict_dt_leaf_returns_prob() {
        let node = DtNode::Leaf { prob: 0.8, n: 10 };
        assert_eq!(predict_dt(&node, &[0.0; 20]), 0.8);
    }

    #[test]
    fn test_predict_dt_split_routes_correctly() {
        let node = DtNode::Split {
            feature: 0,
            threshold: 0.5,
            left: Box::new(DtNode::Leaf { prob: 0.2, n: 5 }),
            right: Box::new(DtNode::Leaf { prob: 0.8, n: 5 }),
        };
        let mut x_low = [1.0; 20];
        x_low[0] = 0.0;
        let mut x_high = [1.0; 20];
        x_high[0] = 1.0;
        assert_eq!(predict_dt(&node, &x_low), 0.2);
        assert_eq!(predict_dt(&node, &x_high), 0.8);
    }

    #[test]
    fn test_predict_rf_averages_trees() {
        let trees = vec![
            DtParams { root: DtNode::Leaf { prob: 0.6, n: 1 }, n_classes: 2 },
            DtParams { root: DtNode::Leaf { prob: 0.8, n: 1 }, n_classes: 2 },
        ];
        let rf = RfParams { trees, n_classes: 2 };
        let model = TrainedModel::Rf(rf);
        assert!((predict_proba(&model, &[0.0; 20]) - 0.7).abs() < 1e-9);
    }

    #[test]
    fn test_predict_proba_top_level_dispatch() {
        let m = TrainedModel::Lr(LrParams {
            coefficients: vec![0.0; 20],
            intercept: 0.0,
        });
        assert!((predict_proba(&m, &[0.0; 20]) - 0.5).abs() < 1e-9);
        let m2 = TrainedModel::Dt(DtParams {
            root: DtNode::Leaf { prob: 0.42, n: 1 },
            n_classes: 2,
        });
        assert!((predict_proba(&m2, &[0.0; 20]) - 0.42).abs() < 1e-9);
    }

    #[test]
    fn test_loaded_model_predict_proba_delegates() {
        let loaded = LoadedModel {
            id: 1,
            kind: ModelKind::LogisticRegression,
            hold_days: 5,
            threshold_pct: 0.5,
            model: TrainedModel::Lr(LrParams {
                coefficients: vec![0.0; 20],
                intercept: 0.0,
            }),
            report: TrainReport {
                kind: ModelKind::LogisticRegression,
                hold_days: 5,
                threshold_pct: 0.5,
                trained_at: 0,
                train_samples: 100,
                n_features: 20,
                cv_accuracies: vec![0.55; 4],
                cv_mean: 0.55,
                cv_min: 0.55,
                cv_max: 0.55,
                train_accuracy: 0.6,
                train_codes: vec!["600519".into()],
                feature_importance: None,
            },
        };
        let x = [0.0; 20];
        assert!((loaded.predict_proba(&x) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn test_trained_model_serde_roundtrip() {
        let m = TrainedModel::Lr(LrParams {
            coefficients: vec![0.1, 0.2, 0.3],
            intercept: -0.5,
        });
        let json = serde_json::to_string(&m).unwrap();
        let back: TrainedModel = serde_json::from_str(&json).unwrap();
        match back {
            TrainedModel::Lr(p) => {
                assert_eq!(p.coefficients, vec![0.1, 0.2, 0.3]);
                assert!((p.intercept - -0.5).abs() < 1e-9);
            }
            _ => panic!("wrong variant after roundtrip"),
        }

        let dt = TrainedModel::Dt(DtParams {
            root: DtNode::Split {
                feature: 0,
                threshold: 0.5,
                left: Box::new(DtNode::Leaf { prob: 0.3, n: 10 }),
                right: Box::new(DtNode::Leaf { prob: 0.7, n: 10 }),
            },
            n_classes: 2,
        });
        let json = serde_json::to_string(&dt).unwrap();
        let back: TrainedModel = serde_json::from_str(&json).unwrap();
        let x_low = [0.0; 20];
        let mut x_high = [0.0; 20];
        x_high[0] = 1.0;
        assert_eq!(predict_proba(&back, &x_low), 0.3);
        assert_eq!(predict_proba(&back, &x_high), 0.7);
    }
}
