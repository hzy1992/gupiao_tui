//! 命令行参数定义
//!
//! 采用 clap 子命令架构：
//! - 默认子命令为 `query`（保持旧版 CLI 兼容：`gupiao 600519`）
//! - `fav add/remove/list` 管理自选股
//! - `watch` 一键查看所有自选股的实时行情

use clap::{Parser, Subcommand};

/// A 股实时行情命令行查询工具
#[derive(Parser, Debug)]
#[command(
    name = "gupiao",
    version,
    about = "A 股实时行情命令行查询工具",
    long_about = None
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// 所有子命令
#[derive(Subcommand, Debug)]
pub enum Command {
    /// 启动简洁的终端 GUI
    #[command(visible_alias = "ui")]
    Tui,

    ///
    /// 用法：`gupiao 600519` 或 `gupiao query 600519 -i 3`
    #[command(visible_alias = "q")]
    Query(QueryArgs),

    /// 自选股（收藏）管理
    #[command(subcommand)]
    Fav(FavCommand),

    /// 查看所有自选股的实时行情
    #[command(visible_alias = "w")]
    Watch(WatchArgs),

    /// 自动分析股票趋势，提示买卖、预测买点卖点
    /// 用法：`gupiao analyze 600519`
    #[command(visible_alias = "a")]
    Analyze(AnalyzeArgs),

    /// 拉取并缓存历史日 K 线
    /// 用法：`gupiao kline 600519 --days 250`
    #[command(visible_alias = "k")]
    Kline(KlineArgs),

    /// 用历史数据评估当前算法的准确度
    /// 用法：`gupiao backtest 600519 --hold 5`
    #[command(visible_alias = "bt")]
    Backtest(BacktestArgs),

    /// 批量回测自选股，输出汇总表（CSV + 控制台表格）
    /// 用法：`gupiao report [--hold 1,3,5,10,20] [--days 250]`
    Report(ReportArgs),

    /// 算法优化：贡献度分析 + 网格搜索最优权重
    /// 用法：`gupiao optimize [--days 250] [--hold 1,3,5,10,20]`
    Optimize(OptimizeArgs),

    /// 用历史 K 线训练机器学习模型
    /// 用法：`gupiao train [--kind rf] [--hold 5] [--days 1000]`
    #[command(visible_alias = "t")]
    Train(TrainArgs),

    /// 管理已训练的模型（list/show/use/delete）
    /// 用法：`gupiao model list`
    #[command(subcommand)]
    Model(ModelCommand),
}

/// `query` 子命令的参数
#[derive(Parser, Debug)]
pub struct QueryArgs {
    /// 股票代码，例如 600519、000001、300750、830799
    pub code: String,

    /// 自动刷新间隔（秒），0 表示只查询一次
    #[arg(short, long, default_value_t = 0)]
    pub interval: u64,

    /// 同时显示五档买卖盘
    #[arg(short, long, default_value_t = true)]
    pub depth: bool,

    /// 同时把当前股票加入自选股（等价于 fav add）
    #[arg(short, long, default_value_t = false)]
    pub fav: bool,
}

/// `watch` 子命令的参数
#[derive(Parser, Debug)]
pub struct WatchArgs {
    /// 自动刷新间隔（秒），0 表示只查询一次
    #[arg(short, long, default_value_t = 0)]
    pub interval: u64,

    /// 不显示五档买卖盘（默认显示以减少横向宽度）
    #[arg(short, long, default_value_t = false)]
    pub depth: bool,
}

/// `fav` 子命令的子命令
#[derive(Subcommand, Debug)]
pub enum FavCommand {
    /// 添加一只股票到自选股
    ///
    /// 用法：`gupiao fav add 600519 -n 长线`
    Add(FavAddArgs),

    /// 从自选股中移除
    ///
    /// 用法：`gupiao fav remove 600519`
    Remove(FavRemoveArgs),

    /// 列出所有自选股
    ///
    /// 用法：`gupiao fav list`
    #[command(visible_alias = "ls")]
    List,

    /// 清空所有自选股
    ///
    /// 用法：`gupiao fav clear`
    Clear,
}

/// `fav add` 的参数
#[derive(Parser, Debug)]
pub struct FavAddArgs {
    /// 股票代码
    pub code: String,
    /// 备注（可选，如"长线"、"观察"、"目标价 1500"）
    #[arg(short, long)]
    pub note: Option<String>,
}

/// `fav remove` 的参数
#[derive(Parser, Debug)]
pub struct FavRemoveArgs {
    /// 股票代码
    pub code: String,
}

/// `analyze` 子命令的参数
#[derive(Parser, Debug)]
pub struct AnalyzeArgs {
    /// 股票代码，例如 600519、000001、300750
    pub code: String,

    /// 自动刷新间隔（秒），0 表示只分析一次
    #[arg(short, long, default_value_t = 0)]
    pub interval: u64,

    /// 仅以 JSON 格式输出（便于脚本消费）
    #[arg(long, default_value_t = false)]
    pub json: bool,

    /// 权重配置：default（默认）/ optimal（网格搜索最优）
    #[arg(long, default_value = "default")]
    pub weights: String,

    /// 融合策略：rules / model / weighted（默认 weighted）
    #[arg(long, default_value = "weighted")]
    pub fusion: String,

    /// 融合权重 α（仅 weighted 生效，0-1，默认 0.5）
    #[arg(long, default_value_t = 0.5)]
    pub alpha: f64,
}

/// `kline` 子命令的参数
#[derive(Parser, Debug)]
pub struct KlineArgs {
    /// 股票代码，例如 600519、000001
    pub code: String,

    /// 拉取最近多少个交易日（默认 250 ≈ 一年）
    #[arg(short, long, default_value_t = 250)]
    pub days: usize,

    /// 强制从网络重新拉取（不使用缓存）
    #[arg(long, default_value_t = false)]
    pub refresh: bool,
}

/// `backtest` 子命令的参数
#[derive(Parser, Debug)]
pub struct BacktestArgs {
    /// 股票代码，例如 600519、000001
    pub code: String,

    /// 持仓天数：发出信号后持有 N 个交易日再平仓（默认 5）
    #[arg(long, default_value_t = 5)]
    pub hold: usize,

    /// 拉取最近多少个交易日（默认 500 ≈ 两年）
    #[arg(long, default_value_t = 500)]
    pub days: usize,

    /// 包含全部信号（默认只看多头：Buy/StrongBuy）
    #[arg(long, default_value_t = false)]
    pub all_signals: bool,

    /// 强制从网络重新拉取 K 线（不使用缓存）
    #[arg(long, default_value_t = false)]
    pub refresh: bool,
}

/// `report` 子命令的参数：批量回测自选股并出汇总表
#[derive(Parser, Debug)]
pub struct ReportArgs {
    /// 持仓天数列表，用逗号分隔（默认 1,3,5,10,20）
    #[arg(long, value_delimiter = ',', default_values_t = vec![1, 3, 5, 10, 20])]
    pub hold: Vec<usize>,

    /// 拉取最近多少个交易日（默认 250 ≈ 一年）
    #[arg(long, default_value_t = 250)]
    pub days: usize,

    /// 强制刷新缓存
    #[arg(long, default_value_t = false)]
    pub refresh: bool,

    /// CSV 输出路径（默认 gupiao_report.csv）
    #[arg(long)]
    pub csv: Option<String>,
}

/// `optimize` 子命令的参数：算法权重优化
#[derive(Parser, Debug)]
pub struct OptimizeArgs {
    /// 持仓天数列表（默认 1,3,5,10,20）
    #[arg(long, value_delimiter = ',', default_values_t = vec![1, 3, 5, 10, 20])]
    pub hold: Vec<usize>,

    /// 拉取最近多少个交易日（默认 250 ≈ 一年）
    #[arg(long, default_value_t = 250)]
    pub days: usize,

    /// 强制刷新缓存
    #[arg(long, default_value_t = false)]
    pub refresh: bool,

    /// 网格搜索 Top-N 输出（默认 10）
    #[arg(long, default_value_t = 10)]
    pub top: usize,
}

/// `train` 子命令的参数：ML 模型训练
#[derive(Parser, Debug)]
pub struct TrainArgs {
    /// 模型种类：lr / dt / rf（默认 rf）
    #[arg(long, default_value = "rf")]
    pub kind: String,

    /// 持仓天数（默认 5）
    #[arg(long, default_value_t = 5)]
    pub hold: usize,

    /// 标签阈值：未来 N 日收益 > 此值视为 Up（%）。默认 0.5
    #[arg(long, default_value_t = 0.5)]
    pub threshold: f64,

    /// walk-forward 折数（默认 4）
    #[arg(long, default_value_t = 4)]
    pub splits: usize,

    /// 拉取最近多少个交易日（默认 1000 ≈ 4 年）
    #[arg(long, default_value_t = 1000)]
    pub days: usize,

    /// 强制刷新 K 线缓存
    #[arg(long, default_value_t = false)]
    pub refresh: bool,

    /// 训练后设为默认模型（供 `analyze` 使用）
    #[arg(long, default_value_t = true)]
    pub set_default: bool,

    /// 自选股代码列表，逗号分隔；留空用所有自选股
    #[arg(long)]
    pub stocks: Option<String>,
}

/// `model` 子命令
#[derive(Subcommand, Debug)]
pub enum ModelCommand {
    /// 列出所有已训练模型
    #[command(visible_alias = "ls")]
    List,
    /// 显示模型详情
    Show {
        /// 模型 id
        id: i64,
    },
    /// 设置为默认推理模型
    Use {
        /// 模型 id
        id: i64,
    },
    /// 删除模型
    Delete {
        /// 模型 id
        id: i64,
    },
}

/// 让 `gupiao 600519` 等价于 `gupiao query 600519`
///
/// clap 默认不支持「默认子命令」，因此在 `parse_cli` 中做一次手动分发：
/// - 第一个非选项的位置参数是已知子命令（query/q/fav/watch/w）→ 直接 `Cli::parse()`
/// - 否则视为旧式 `query` 子命令，把 argv[0] + 剩余参数当作 `query ...`
///
/// 注：当 argv 中**只有**全局标志（`-V`/`--version`/`-h`/`--help`）而没有子命令
/// 和位置参数时，也要走 `Cli::parse()` 让 clap 打印顶层 help/version。
pub fn parse_cli() -> Cli {
    let raw: Vec<String> = std::env::args().collect();
    let known_subs = ["query", "q", "fav", "watch", "w", "tui", "ui", "analyze", "a", "kline", "k", "backtest", "bt", "report", "optimize", "train", "t", "model"];

    // 第一个非选项参数
    let first_positional = raw.iter().skip(1).find(|a| !a.starts_with('-'));

    match first_positional {
        Some(p) if known_subs.iter().any(|s| s == p) => Cli::parse(),
        // 没有位置参数：让 clap 走顶层分支（处理 -V/--version/-h/--help）
        None => Cli::parse(),
        // 有位置参数但不是已知子命令：当作 query 子命令（兼容旧 CLI）
        Some(_) => {
            let mut new_argv = vec![raw[0].clone(), "query".to_string()];
            new_argv.extend_from_slice(&raw[1..]);
            Cli::parse_from(new_argv)
        }
    }
}
