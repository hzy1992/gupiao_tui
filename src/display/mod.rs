//! 显示层
//!
//! 通过 `QuoteDisplay` trait 抽象不同的输出格式：
//! - `terminal`：彩色终端输出（默认，单只股票详情）
//! - `watch`：自选股列表式输出（多只股票一览）
//!
//! 后续可扩展：
//! - `json`：结构化 JSON 输出（适合脚本管道）
//! - `csv`：表格导出
//!

use std::io::Write;

use crate::model::Quote;

mod analysis;
mod backtest;
mod terminal;
mod watch;

pub use analysis::AnalysisDisplay;
pub use backtest::BacktestDisplay;
pub use terminal::TerminalDisplay;
pub use watch::WatchDisplay;

/// 行情展示的通用接口
pub trait QuoteDisplay {
    /// 将行情写入输出流
    fn write<W: Write>(
        &self,
        w: &mut W,
        quote: &Quote,
        options: &DisplayOptions,
    ) -> std::io::Result<()>;
}

/// 显示选项
#[derive(Debug, Clone)]
pub struct DisplayOptions {
    /// 是否显示五档买卖盘
    pub show_depth: bool,
    /// 是否在底部追加一行额外信息（如时间、提示）
    pub show_footer: bool,
}

impl Default for DisplayOptions {
    fn default() -> Self {
        Self {
            show_depth: true,
            show_footer: true,
        }
    }
}
