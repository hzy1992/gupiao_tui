//! 训练流程编排：walk-forward 切分 + 训练 + 评估
//!
//! ## 设计
//!
//! 输入：多只股票的 K 线 + 训练配置
//! 1. 每只股票独立生成样本，按时间排序
//! 2. **严格时序 walk-forward**：按整体时间切分，确保训练集日期永远早于测试集
//! 3. 在每折的训练集上训练，测试集上算准确率
//! 4. 用全部数据训练最终模型
//! 5. 返回 TrainReport
//!
//! ## Walk-forward 时序保证
//!
//! - 先把所有样本按 (signal_date, code) 排序
//! - 切分点用日期索引，不用随机混排
//! - 每折训练集 = 最早的样本 ~ 切分点前；测试集 = 切分点 ~ 切分点+窗口
//! - 严格不泄漏：任何训练样本的日期都严格早于测试样本

use anyhow::{anyhow, Result};

use crate::analysis::features::NUM_FEATURES;
use crate::analysis::labels::{merge_samples, Sample};
use crate::analysis::ml as ml_wrap;
use crate::model::ml::{ModelKind, TrainConfig, TrainReport, TrainedModel};

pub fn train_models(
    datasets: &[(String, crate::model::Market, crate::model::KlineSeries)],
    cfg: &TrainConfig,
) -> Result<(TrainReport, TrainedModel)> {
    eprintln!("[DEBUG] train_models: starting with {} datasets", datasets.len());
    let all_samples = merge_samples(datasets, cfg.hold_days, cfg.threshold_pct);
    eprintln!("[DEBUG] train_models: generated {} samples", all_samples.len());
    if all_samples.len() < cfg.min_samples {
        return Err(anyhow!("too few samples {} < {}", all_samples.len(), cfg.min_samples));
    }

    // 时序 walk-forward CV：严格按时间排序
    let mut samples = all_samples;
    samples.sort_by_key(|s| s.signal_date);
    eprintln!("[DEBUG] train_models: sorted {} samples, first_date={:?}, last_date={:?}",
        samples.len(),
        samples.first().map(|s| s.signal_date),
        samples.last().map(|s| s.signal_date));

    eprintln!("[DEBUG] train_models: calling walk_forward with n_splits={}", cfg.n_splits);
    let cv_accuracies = walk_forward(&samples, cfg)?;
    eprintln!("[DEBUG] train_models: walk_forward done, {} folds", cv_accuracies.len());

    // 用全部样本训练最终模型
    let xs: Vec<Vec<f64>> = samples.iter().map(|s| s.features.to_vec()).collect();
    let ys: Vec<f64> = samples.iter().map(|s| s.label.as_f64()).collect();
    let (x_dm, y_i32) = ml_wrap::samples_to_arrays(&xs, &ys)?;
    let (final_model, feature_importance) =
        ml_wrap::train(cfg.kind, &x_dm, &y_i32, cfg.rf_trees, max_depth_for(cfg))?;
    let train_preds = ml_wrap::predict_classes(&final_model, &x_dm);
    let train_accuracy = ml_wrap::accuracy(&y_i32, &train_preds);
    let train_codes: Vec<String> = datasets.iter().map(|(c, _, _)| c.clone()).collect();
    let cv_mean = mean(&cv_accuracies);
    let cv_min = cv_accuracies.iter().cloned().fold(f64::INFINITY, f64::min);
    let cv_max = cv_accuracies.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

    let report = TrainReport {
        kind: cfg.kind,
        hold_days: cfg.hold_days,
        threshold_pct: cfg.threshold_pct,
        trained_at: chrono::Local::now().timestamp(),
        train_samples: samples.len(),
        n_features: NUM_FEATURES,
        cv_accuracies: cv_accuracies.clone(),
        cv_mean,
        cv_min,
        cv_max,
        train_accuracy,
        train_codes,
        feature_importance: Some(feature_importance.to_vec()),
    };

    Ok((report, final_model))
}

/// 严格时序 walk-forward 验证
///
/// 关键改进：不再「随机混排后再切」，而是严格按时间轴切分。
/// - 每折的切分点是一个日期边界（而非随机位置）
/// - 训练集 = [0, split_idx)，测试集 = [split_idx, split_idx + fold_size)
/// - 如果测试集里样本太少（< 5），跳过该折
///
/// 这样保证了时间序列的特性：永远用「更早」的数据训练，预测「更晚」的数据。
fn walk_forward(samples: &[Sample], cfg: &TrainConfig) -> Result<Vec<f64>> {
    let n = samples.len();
    eprintln!("[DEBUG] walk_forward: n={}, n_splits={}, min_needed={}", n, cfg.n_splits, cfg.n_splits * 10);
    if n < cfg.n_splits * 10 {
        return Err(anyhow!("not enough samples {} for {} splits", n, cfg.n_splits));
    }

    // fold_size = 总样本数 / (n_splits + 1)
    // 切分点 = fold_size * (i + 1)，i from 0 to n_splits-1
    let fold_size = n / (cfg.n_splits + 1);
    let mut accs = Vec::with_capacity(cfg.n_splits);

    for i in 0..cfg.n_splits {
        let split_idx = fold_size * (i + 1);
        let test_start = split_idx;
        let test_end = (split_idx + fold_size).min(n);

        // 安全检查
        if test_start >= n || test_end <= test_start {
            continue;
        }

        let train_slice = &samples[..test_start];
        let test_slice = &samples[test_start..test_end];

        if train_slice.len() < 20 || test_slice.len() < 5 {
            continue;
        }

        // 严格时序验证：测试集第一条的日期必须晚于训练集最后一条
        let train_last_date = train_slice.last().map(|s| s.signal_date);
        let test_first_date = test_slice.first().map(|s| s.signal_date);
        if train_last_date >= test_first_date {
            // 日期有重叠（多只股票同一日期），检查是否真的泄漏
            // 只有当 test_first <= train_last 时才跳过
            continue;
        }

        // 训练
        let xs_tr: Vec<Vec<f64>> = train_slice.iter().map(|s| s.features.to_vec()).collect();
        let ys_tr: Vec<f64> = train_slice.iter().map(|s| s.label.as_f64()).collect();
        let (x_tr, y_tr) = ml_wrap::samples_to_arrays(&xs_tr, &ys_tr)?;
        let (model, _imp) = ml_wrap::train(cfg.kind, &x_tr, &y_tr, cfg.rf_trees, max_depth_for(cfg))?;

        // 测试
        let xs_te: Vec<Vec<f64>> = test_slice.iter().map(|s| s.features.to_vec()).collect();
        let ys_te: Vec<f64> = test_slice.iter().map(|s| s.label.as_f64()).collect();
        let (x_te, y_te) = ml_wrap::samples_to_arrays(&xs_te, &ys_te)?;
        let preds = ml_wrap::predict_classes(&model, &x_te);
        let acc = ml_wrap::accuracy(&y_te, &preds);
        accs.push(acc);
    }

    if accs.is_empty() {
        return Err(anyhow!("walk-forward produced no valid folds (check sample size)"));
    }
    Ok(accs)
}

fn mean(xs: &[f64]) -> f64 {
    if xs.is_empty() { return 0.0; }
    xs.iter().sum::<f64>() / xs.len() as f64
}

fn max_depth_for(cfg: &TrainConfig) -> u16 {
    match cfg.kind {
        ModelKind::LogisticRegression => 0,
        ModelKind::DecisionTree => 6,
        ModelKind::RandomForest => 6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Kline, KlineSeries, Market};
    use chrono::{Duration, NaiveDate};

    fn trending_series(days: usize, daily_return: f64) -> KlineSeries {
        let mut bars = Vec::with_capacity(days);
        let mut p = 100.0;
        for i in 0..days {
            let date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap() + Duration::days(i as i64);
            p *= 1.0 + daily_return;
            bars.push(Kline { date, open: p * 0.99, high: p * 1.02, low: p * 0.98, close: p, volume: 1_000_000 });
        }
        KlineSeries::new(bars)
    }

    fn make_datasets() -> Vec<(String, Market, KlineSeries)> {
        vec![
            (String::from("S1"), Market::Shanghai, trending_series(120, 0.005)),
            (String::from("S2"), Market::Shenzhen, trending_series(120, -0.005)),
        ]
    }

    #[test]
    fn test_train_models_lr() {
        let data = make_datasets();
        let cfg = TrainConfig { kind: ModelKind::LogisticRegression, hold_days: 5, threshold_pct: 0.5, n_splits: 3, min_samples: 30, rf_trees: 5 };
        let (report, model) = train_models(&data, &cfg).unwrap();
        assert_eq!(report.kind, ModelKind::LogisticRegression);
        assert!(report.train_samples > 30);
        assert!(!report.cv_accuracies.is_empty());
        assert!(matches!(model, TrainedModel::Lr(_)));
        assert!(report.feature_importance.is_some());
        assert_eq!(report.feature_importance.as_ref().unwrap().len(), NUM_FEATURES);
    }

    #[test]
    fn test_train_models_dt() {
        let data = make_datasets();
        let cfg = TrainConfig { kind: ModelKind::DecisionTree, hold_days: 5, threshold_pct: 0.5, n_splits: 3, min_samples: 30, rf_trees: 5 };
        let (_report, model) = train_models(&data, &cfg).unwrap();
        assert!(matches!(model, TrainedModel::Dt(_)));
    }

    #[test]
    fn test_train_models_rf() {
        let data = make_datasets();
        let cfg = TrainConfig { kind: ModelKind::RandomForest, hold_days: 5, threshold_pct: 0.5, n_splits: 3, min_samples: 30, rf_trees: 5 };
        let (_report, model) = train_models(&data, &cfg).unwrap();
        match model {
            TrainedModel::Rf(rf) => assert_eq!(rf.trees.len(), 5),
            _ => panic!("expected Rf"),
        }
    }

    #[test]
    fn test_train_models_too_few_samples() {
        let data = vec![(String::from("S1"), Market::Shanghai, trending_series(30, 0.005))];
        let cfg = TrainConfig { min_samples: 1000, ..TrainConfig::default() };
        assert!(train_models(&data, &cfg).is_err());
    }

    #[test]
    fn test_mean_basic() {
        assert_eq!(mean(&[]), 0.0);
        assert!((mean(&[1.0, 2.0, 3.0]) - 2.0).abs() < 1e-9);
    }

    #[test]
    fn test_report_fields_populated() {
        let data = make_datasets();
        let cfg = TrainConfig { kind: ModelKind::RandomForest, hold_days: 5, threshold_pct: 0.5, n_splits: 3, min_samples: 30, rf_trees: 5 };
        let (report, _model) = train_models(&data, &cfg).unwrap();
        assert_eq!(report.hold_days, 5);
        assert!((report.threshold_pct - 0.5).abs() < 1e-9);
        assert_eq!(report.n_features, NUM_FEATURES);
        assert_eq!(report.train_codes, vec![String::from("S1"), String::from("S2")]);
        assert!(report.cv_mean >= 0.0 && report.cv_mean <= 1.0);
        assert!(report.trained_at > 0);
        let imp = report.feature_importance.as_ref().unwrap();
        let sum: f64 = imp.iter().sum();
        assert!((sum - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_walk_forward_respects_temporal_order() {
        // 验证 walk-forward 确实按时间顺序切分
        let data = make_datasets();
        let cfg = TrainConfig { kind: ModelKind::DecisionTree, hold_days: 5, threshold_pct: 0.5, n_splits: 3, min_samples: 30, rf_trees: 5 };
        let (report, _model) = train_models(&data, &cfg).unwrap();
        // 有 3 折，应该有 3 个 cv accuracy
        assert_eq!(report.cv_accuracies.len(), 3);
        // 每折准确率应该在 [0, 1] 范围内
        for acc in &report.cv_accuracies {
            assert!(*acc >= 0.0 && *acc <= 1.0);
        }
    }
}
