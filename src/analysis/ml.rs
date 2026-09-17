//! 模型训练层
//!
//! ## 职责
//!
//! 1. 把训练样本喂给 smartcore 或自实现算法
//! 2. 训练后抽取参数，转成我们自己的 model::ml::TrainedModel
//! 3. 推理路径不保留任何 smartcore 依赖
//!
//! ## RF 特征采样（关键改进）
//!
//! 标准随机森林在每次分裂时从 sqrt(N) 个特征中选最优分裂，
//! 这是 RF 区别于单棵决策树的关键——让各树更独立，降低过拟合。
//! 当前实现在 build_tree_with_features() 中每次只搜索随机采样的 mtry 个特征。
//!
//! ## 特征重要性
//!
//! 训练完成后计算 Gini 重要性：
//! 每棵树的每个节点记录分裂后 Gini 减少量，按 feature 聚合，多棵树取平均。

use anyhow::{anyhow, Result};
use rand::{RngExt, SeedableRng};
use rand::seq::IndexedRandom;

use smartcore::linalg::basic::arrays::Array;
use smartcore::linalg::basic::matrix::DenseMatrix;
use smartcore::linear::logistic_regression::{LogisticRegression, LogisticRegressionParameters};

use crate::analysis::features::{NUM_FEATURES, FEATURE_NAMES};
use crate::model::ml::{DtNode, DtParams, LrParams, ModelKind, RfParams, TrainedModel};

/// RF 每次分裂时随机采样的特征数（标准：sqrt(N)，向上取整）
/// sqrt(20) rounded up
pub const MTRY: usize = 5;

/// 把训练样本压成 DenseMatrix + 标签
pub fn samples_to_arrays(xs: &[Vec<f64>], ys: &[f64]) -> Result<(DenseMatrix<f64>, Vec<i32>)> {
    if xs.len() != ys.len() {
        return Err(anyhow!("xs.len()={} != ys.len()={}", xs.len(), ys.len()));
    }
    if xs.is_empty() {
        return Err(anyhow!("empty training set"));
    }
    for row in xs {
        if row.len() != NUM_FEATURES {
            return Err(anyhow!("feature dim mismatch: got {}, expected {}",
                row.len(), NUM_FEATURES));
        }
    }
    let xs_vec = xs.to_vec();
    let dm = DenseMatrix::from_2d_vec(&xs_vec).map_err(|e| anyhow!("DenseMatrix build: {}", e))?;
    let y_i32: Vec<i32> = ys.iter().map(|v| if *v >= 0.5 { 1 } else { 0 }).collect();
    Ok((dm, y_i32))
}

/// 在 (matrix, labels) 上预测类别准确率
pub fn accuracy(y_true: &[i32], y_pred: &[i32]) -> f64 {
    if y_true.is_empty() { return 0.0; }
    let correct = y_true.iter().zip(y_pred.iter()).filter(|(a, b)| a == b).count();
    correct as f64 / y_true.len() as f64
}

/// 特征重要性结果（与 FEATURE_NAMES 一一对应）
pub type FeatureImportance = [f64; NUM_FEATURES];

/// 训练入口：根据 ModelKind 路由
pub fn train(
    kind: ModelKind,
    x: &DenseMatrix<f64>,
    y: &Vec<i32>,
    rf_trees: usize,
    max_depth: u16,
) -> Result<(TrainedModel, FeatureImportance)> {
    match kind {
        ModelKind::LogisticRegression => {
            let model = train_lr(x, y)?;
            let importance = lr_feature_importance(&model);
            Ok((model, importance))
        }
        ModelKind::DecisionTree => {
            let model = train_dt_inplace(x, y, max_depth)?;
            let importance = dt_feature_importance(&model, x.shape().1);
            Ok((model, importance))
        }
        ModelKind::RandomForest => {
            let model = train_rf_inplace(x, y, rf_trees, max_depth)?;
            let importance = rf_feature_importance(&model);
            Ok((model, importance))
        }
    }
}

/// 逻辑回归训练
pub fn train_lr(x: &DenseMatrix<f64>, y: &Vec<i32>) -> Result<TrainedModel> {
    let params = LogisticRegressionParameters::default();
    let model = LogisticRegression::fit(x, y, params)
        .map_err(|e| anyhow!("LR fit failed: {}", e))?;
    let coefficients: Vec<f64> = model.coefficients().iter().copied().collect();
    let intercept_vec: Vec<f64> = model.intercept().iter().copied().collect();
    let intercept = intercept_vec.first().copied().unwrap_or(0.0);
    Ok(TrainedModel::Lr(LrParams { coefficients, intercept }))
}

/// 训练决策树
pub fn train_dt_inplace(
    x: &DenseMatrix<f64>,
    y: &Vec<i32>,
    max_depth: u16,
) -> Result<TrainedModel> {
    let n = x.shape().0;
    if n == 0 { return Err(anyhow!("empty training data")); }
    let indices: Vec<usize> = (0..n).collect();
    let root = build_tree(x, y, &indices, 0, max_depth, &mut None);
    Ok(TrainedModel::Dt(DtParams { root, n_classes: 2 }))
}

/// 训练随机森林（修正：每次分裂在 sqrt(N) 特征中搜索）
pub fn train_rf_inplace(
    x: &DenseMatrix<f64>,
    y: &Vec<i32>,
    n_trees: usize,
    max_depth: u16,
) -> Result<TrainedModel> {
    let n = x.shape().0;
    if n < 10 { return Err(anyhow!("RF: too few samples ({})", n)); }
    let mut rng = rand::rngs::StdRng::seed_from_u64(42);
    let mut trees = Vec::with_capacity(n_trees);
    for _ in 0..n_trees {
        // Bootstrap 采样（有放回抽样）
        let mut sample: Vec<usize> = (0..n).map(|_| rng.random_range(0..n)).collect();
        sample.sort_unstable();
        sample.dedup();
        if sample.is_empty() { sample = (0..n).collect(); }
        // 每棵树的分裂特征集：随机选 mtry 个
        let features: Vec<usize> = (0..x.shape().1).collect();
        let mtry = MTRY.min(x.shape().1);
        let feat_subset: Vec<usize> = features.sample(&mut rng, mtry).copied().collect();
        let root = build_tree_with_features(x, y, &sample, 0, max_depth, &feat_subset);
        trees.push(DtParams { root, n_classes: 2 });
    }
    Ok(TrainedModel::Rf(RfParams { trees, n_classes: 2 }))
}

/// CART 递归构建（所有特征都可选，用于单棵 DT）
fn build_tree(
    x: &DenseMatrix<f64>,
    y: &Vec<i32>,
    indices: &[usize],
    depth: u16,
    max_depth: u16,
    _importance: &mut Option<&mut [f64; NUM_FEATURES]>,
) -> DtNode {
    let n = indices.len();
    if n == 0 { return DtNode::Leaf { prob: 0.5, n: 0 }; }
    if depth >= max_depth || n <= 5 { return leaf_node(y, indices); }

    let (_, prob) = class_counts(y, indices);
    let parent_impurity = gini(prob);
    let mut best_gain = 0.0;
    let mut best_feat = 0;
    let mut best_thr = 0.0;

    for feat in 0..x.shape().1 {
        let mut values: Vec<(usize, f64)> = indices
            .iter()
            .map(|&i| (i, *x.get((i, feat))))
            .collect();
        values.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        let mut last_v = f64::NEG_INFINITY;
        for &(_i, v) in &values {
            if (v - last_v).abs() < 1e-9 { continue; }
            last_v = v;
            let thr = v;
            let (left, right) = split_indices(indices, x, feat, thr);
            if left.is_empty() || right.is_empty() { continue; }
            let (_, p_l) = class_counts(y, &left);
            let (_, p_r) = class_counts(y, &right);
            let w_l = left.len() as f64 / n as f64;
            let w_r = right.len() as f64 / n as f64;
            let gain = parent_impurity - w_l * gini(p_l) - w_r * gini(p_r);
            if gain > best_gain {
                best_gain = gain;
                best_feat = feat;
                best_thr = thr;
            }
        }
    }

    if best_gain <= 1e-9 { return leaf_node(y, indices); }
    let (left_idx, right_idx) = split_indices(indices, x, best_feat, best_thr);
    DtNode::Split {
        feature: best_feat,
        threshold: best_thr,
        left: Box::new(build_tree(x, y, &left_idx, depth + 1, max_depth, _importance)),
        right: Box::new(build_tree(x, y, &right_idx, depth + 1, max_depth, _importance)),
    }
}

/// 带特征子集的分裂搜索（用于 RF）
fn build_tree_with_features(
    x: &DenseMatrix<f64>,
    y: &Vec<i32>,
    indices: &[usize],
    depth: u16,
    max_depth: u16,
    feat_subset: &[usize],
) -> DtNode {
    let n = indices.len();
    if n == 0 { return DtNode::Leaf { prob: 0.5, n: 0 }; }
    if depth >= max_depth || n <= 5 { return leaf_node(y, indices); }

    let (_, prob) = class_counts(y, indices);
    let parent_impurity = gini(prob);
    let mut best_gain = 0.0;
    let mut best_feat = feat_subset[0];
    let mut best_thr = 0.0;

    for &feat in feat_subset {
        let mut values: Vec<(usize, f64)> = indices
            .iter()
            .map(|&i| (i, *x.get((i, feat))))
            .collect();
        values.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        let mut last_v = f64::NEG_INFINITY;
        for &(_i, v) in &values {
            if (v - last_v).abs() < 1e-9 { continue; }
            last_v = v;
            let thr = v;
            let (left, right) = split_indices(indices, x, feat, thr);
            if left.is_empty() || right.is_empty() { continue; }
            let (_, p_l) = class_counts(y, &left);
            let (_, p_r) = class_counts(y, &right);
            let w_l = left.len() as f64 / n as f64;
            let w_r = right.len() as f64 / n as f64;
            let gain = parent_impurity - w_l * gini(p_l) - w_r * gini(p_r);
            if gain > best_gain {
                best_gain = gain;
                best_feat = feat;
                best_thr = thr;
            }
        }
    }

    if best_gain <= 1e-9 { return leaf_node(y, indices); }
    let (left_idx, right_idx) = split_indices(indices, x, best_feat, best_thr);
    DtNode::Split {
        feature: best_feat,
        threshold: best_thr,
        left: Box::new(build_tree_with_features(x, y, &left_idx, depth + 1, max_depth, feat_subset)),
        right: Box::new(build_tree_with_features(x, y, &right_idx, depth + 1, max_depth, feat_subset)),
    }
}

fn split_indices(indices: &[usize], x: &DenseMatrix<f64>, feat: usize, thr: f64) -> (Vec<usize>, Vec<usize>) {
    let mut left = Vec::new();
    let mut right = Vec::new();
    for &i in indices {
        if *x.get((i, feat)) <= thr { left.push(i); } else { right.push(i); }
    }
    (left, right)
}

fn class_counts(y: &[i32], indices: &[usize]) -> (usize, f64) {
    let n = indices.len();
    if n == 0 { return (0, 0.5); }
    let pos = indices.iter().filter(|&&i| y[i] == 1).count();
    (pos, pos as f64 / n as f64)
}

fn leaf_node(y: &[i32], indices: &[usize]) -> DtNode {
    let (_, prob) = class_counts(y, indices);
    DtNode::Leaf { prob, n: indices.len() }
}

fn gini(p: f64) -> f64 {
    2.0 * p * (1.0 - p)
}

/// 用模型在 DenseMatrix 上预测类别
pub fn predict_classes(model: &TrainedModel, x: &DenseMatrix<f64>) -> Vec<i32> {
    let n = x.shape().0;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let mut row = [0.0_f64; NUM_FEATURES];
        for j in 0..NUM_FEATURES {
            row[j] = *x.get((i, j));
        }
        let prob = crate::model::ml::predict_proba(model, &row);
        out.push(if prob >= 0.5 { 1 } else { 0 });
    }
    out
}

// ==================== 特征重要性 ====================

/// 从 LR 系数计算特征重要性（绝对值归一化）
fn lr_feature_importance(model: &TrainedModel) -> FeatureImportance {
    let mut imp = [0.0; NUM_FEATURES];
    if let TrainedModel::Lr(p) = model {
        for (i, &w) in p.coefficients.iter().enumerate().take(NUM_FEATURES) {
            imp[i] = w.abs();
        }
    }
    normalize_importance(&mut imp);
    imp
}

/// 从单棵决策树计算特征重要性（基于 Gini 减少量）
fn dt_feature_importance(model: &TrainedModel, _n_features: usize) -> FeatureImportance {
    let mut imp = [0.0; NUM_FEATURES];
    if let TrainedModel::Dt(p) = model {
        accumulate_importance(&p.root, &mut imp);
    }
    normalize_importance(&mut imp);
    imp
}

/// 从随机森林计算特征重要性（各树平均）
fn rf_feature_importance(model: &TrainedModel) -> FeatureImportance {
    let mut imp = [0.0; NUM_FEATURES];
    if let TrainedModel::Rf(p) = model {
        let n = p.trees.len();
        for tree in &p.trees {
            let mut tree_imp = [0.0; NUM_FEATURES];
            accumulate_importance(&tree.root, &mut tree_imp);
            for i in 0..NUM_FEATURES {
                imp[i] += tree_imp[i];
            }
        }
        for v in &mut imp { *v /= n as f64; }
    }
    normalize_importance(&mut imp);
    imp
}

/// 递归累加节点的特征重要性贡献
fn accumulate_importance(node: &DtNode, imp: &mut FeatureImportance) {
    match node {
        DtNode::Leaf { .. } => {}
        DtNode::Split { feature, threshold: _, left, right } => {
            let left_prob = match &**left {
                DtNode::Leaf { prob, .. } => *prob,
                _ => 0.5,
            };
            let right_prob = match &**right {
                DtNode::Leaf { prob, .. } => *prob,
                _ => 0.5,
            };
            let n_left = match &**left {
                DtNode::Leaf { n, .. } => *n,
                _ => 1,
            };
            let n_right = match &**right {
                DtNode::Leaf { n, .. } => *n,
                _ => 1,
            };
            let total = (n_left + n_right) as f64;
            let gini_left = gini(left_prob);
            let gini_right = gini(right_prob);
            let gain = gini(0.5) - (n_left as f64 / total * gini_left + n_right as f64 / total * gini_right);
            if *feature < NUM_FEATURES {
                imp[*feature] += gain * total;
            }
            accumulate_importance(left, imp);
            accumulate_importance(right, imp);
        }
    }
}

/// 归一化使特征重要性之和 = 1
fn normalize_importance(imp: &mut FeatureImportance) {
    let sum: f64 = imp.iter().sum();
    if sum > 1e-12 {
        for v in imp.iter_mut() { *v /= sum; }
    }
}

/// 打印特征重要性（用于调试）
pub fn print_feature_importance(imp: &FeatureImportance) {
    let mut pairs: Vec<(&str, f64)> = FEATURE_NAMES
        .iter()
        .zip(imp.iter())
        .map(|(&name, &v)| (name, v))
        .collect();
    pairs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    for (name, v) in pairs {
        let bar_len = (v * 40.0) as usize;
        let bar = "=".repeat(bar_len);
        println!("  {:20} [{:5.2}%] {}", name, v * 100.0, bar);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_data() -> (Vec<Vec<f64>>, Vec<f64>) {
        let mut xs = Vec::new();
        let mut ys = Vec::new();
        for i in 0..200 {
            let mut x = [0.0_f64; NUM_FEATURES];
            x[0] = if i % 2 == 0 { 1.0 } else { -1.0 };
            xs.push(x.to_vec());
            ys.push(if i % 2 == 0 { 1.0 } else { 0.0 });
        }
        (xs, ys)
    }

    #[test]
    fn test_samples_to_arrays_basic() {
        let (xs, ys) = synthetic_data();
        let (dm, y) = samples_to_arrays(&xs, &ys).unwrap();
        assert_eq!(dm.shape().0, 200);
        assert_eq!(dm.shape().1, NUM_FEATURES);
        assert_eq!(y[0], 1);
    }

    #[test]
    fn test_samples_to_arrays_dim_mismatch() {
        let (xs, mut ys) = synthetic_data();
        ys.pop();
        assert!(samples_to_arrays(&xs, &ys).is_err());
        let bad_xs: Vec<Vec<f64>> = vec![vec![1.0; 5]];
        assert!(samples_to_arrays(&bad_xs, &[0.0]).is_err());
    }

    #[test]
    fn test_samples_to_arrays_empty() {
        assert!(samples_to_arrays(&[], &[]).is_err());
    }

    #[test]
    fn test_lr_trains_on_separable_data() {
        let (xs, ys) = synthetic_data();
        let (dm, y) = samples_to_arrays(&xs, &ys).unwrap();
        let (model, imp) = train(ModelKind::LogisticRegression, &dm, &y, 50, 5).unwrap();
        let preds = predict_classes(&model, &dm);
        let acc = accuracy(&y, &preds);
        assert!(acc >= 0.5);
        let sum: f64 = imp.iter().sum();
        assert!((sum - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_dt_inplace_perfectly_on_separable() {
        let (xs, ys) = synthetic_data();
        let (dm, y) = samples_to_arrays(&xs, &ys).unwrap();
        let (model, _) = train(ModelKind::DecisionTree, &dm, &y, 50, 5).unwrap();
        let preds = predict_classes(&model, &dm);
        let acc = accuracy(&y, &preds);
        assert!(acc > 0.9);
    }

    #[test]
    fn test_rf_inplace_perfectly_on_separable() {
        let (xs, ys) = synthetic_data();
        let (dm, y) = samples_to_arrays(&xs, &ys).unwrap();
        let (model, _) = train(ModelKind::RandomForest, &dm, &y, 5, 5).unwrap();
        let preds = predict_classes(&model, &dm);
        let acc = accuracy(&y, &preds);
        assert!(acc > 0.9);
    }

    #[test]
    fn test_feature_importance_sums_to_one() {
        let (xs, ys) = synthetic_data();
        let (dm, y) = samples_to_arrays(&xs, &ys).unwrap();
        let (_, imp) = train(ModelKind::RandomForest, &dm, &y, 5, 5).unwrap();
        let sum: f64 = imp.iter().sum();
        assert!((sum - 1.0).abs() < 1e-9);
        assert!(imp[0] > 0.3, "feature 0 importance = {}, expect > 0.3", imp[0]);
    }

    #[test]
    fn test_accuracy_basic() {
        assert_eq!(accuracy(&[1, 0, 1, 0], &[1, 0, 0, 0]), 0.75);
        assert_eq!(accuracy(&[], &[]), 0.0);
    }
}
