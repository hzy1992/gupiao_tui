//! 股票行情分析与决策建议模块
//!
//! ## 设计原则
//!
//! 本模块是**纯函数**层：不发起任何 IO、不持有任何全局状态。
//! 依赖关系：`analysis` → `model`，`analysis` 不依赖 `provider`/`storage`/`display`。
//!
//! ## 能力边界
//!
//! 由于当前 Provider（腾讯）只返回**单帧实时行情**，无法拿到历史 K 线，
//! 因此本模块采用「单帧衍生指标 + 五档盘口」的技术分析方法，不强依赖历史数据。
//!
//! ### 已实现的指标
//!
//! | 指标 | 输入 | 含义 |
//! |------|------|------|
//! | 价量比 | amount / amplitude | 单位振幅内成交金额 → 主力活跃度 |
//! | 委比 | 五档买卖量 | 买盘力量 − 卖盘力量，越正越强 |
//! | 量比 | volume / (近 N 日均量) | 当前成交是否异常放大 |
//! | 价格位置 | price vs 开/高/低/昨收 | 当日所处百分位 (0~1) |
//! | 趋势强度 | change_pct + 量能 | 趋势是「实在」还是「无量空涨」 |
//!
//! ### 综合信号
//!
//! 五维加权打分 (-100 ~ +100)，映射到四档信号：
//! - **StrongBuy** (≥ 60)：多指标共振，多重支撑
//! - **Buy**      (30 ~ 60)：偏多，可分批建仓
//! - **Hold**     (-30 ~ 30)：震荡，观望
//! - **Sell**     (-60 ~ -30)：偏空，建议减仓
//! - **StrongSell** (≤ -60)：空头共振，建议离场
//!
//! ### 价位预测
//!
//! - **支撑位**（建议买点）：基于当日最低、五档买盘价、技术位加权
//! - **压力位**（建议卖点）：基于当日最高、五档卖盘价、技术位加权
//!
//! > ⚠️ **免责声明**：本模块输出仅基于公开行情数据的技术面启发式打分，
//! > **不构成任何投资建议**。股市有风险，决策请独立判断并自行承担风险。

pub mod backtest;
pub mod features;
pub mod indicators;
pub mod infer;
pub mod labels;
pub mod ml;
pub mod optimize;
pub mod signals;
pub mod train;

pub use indicators::{Indicators, PriceLevel, PriceLevelSource};
pub use signals::{analyze, analyze_with, AnalysisReport, ScoreThresholds, ScoreWeights, Signal, SignalKind};

// M1 新增：特征与标签（机器学习上游）
pub use features::{Features, FEATURE_NAMES, NUM_FEATURES};
pub use labels::{build_samples, future_return, merge_samples, Label, Sample};

// M2 新增：训练流程
pub use train::train_models;

// M3 新增：推理融合
pub use infer::{fuse_score, score_to_strength, FusionStrategy, InferenceContext};
pub use signals::analyze_with_model;
