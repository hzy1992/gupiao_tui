//! # gupiao - A 股实时行情查询库
//!
//! 模块结构：
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │  CLI 层 (cli.rs)                                             │
//! │     解析命令行参数（子命令：查询/收藏/查看自选）              │
//! ├─────────────────────────────────────────────────────────────┤
//! │  Provider 层 (provider/)                                     │
//! │     数据源抽象 + 多实现（腾讯/东财/新浪...）                  │
//! ├─────────────────────────────────────────────────────────────┤
//! │  Display 层 (display/)                                       │
//! │     输出格式抽象 + 多实现（终端/JSON/CSV...）                 │
//! ├─────────────────────────────────────────────────────────────┤
//! │  Storage 层 (storage/)                                       │
//! │     本地持久化：SQLite 存储自选股/历史/配置                   │
//! ├─────────────────────────────────────────────────────────────┤
//! │  Model 层 (model/)                                           │
//! │     Market / Symbol / Quote / Depth / Favorite               │
//! ├─────────────────────────────────────────────────────────────┤
//! │  Platform / Error                                            │
//! │     跨平台兼容与统一错误类型                                  │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## 快速上手（作为库使用）
//!
//! ```no_run
//! use gupiao::model::Symbol;
//! use gupiao::provider::{QuoteProvider, TencentProvider};
//!
//! # async fn run() -> anyhow::Result<()> {
//! let sym = Symbol::new("600519")?;
//! let quote = TencentProvider::new().fetch(&sym).await?;
//! println!("{}: {} 元", quote.name, quote.price);
//! # Ok(()) }
//! ```

pub mod analysis;
pub mod cli;
pub mod display;
pub mod error;
pub mod model;
pub mod platform;
pub mod provider;
pub mod storage;
pub mod tui;

// 重新导出常用类型，方便用户
pub use error::{Error, Result};
pub use model::{Depth, Favorite, Market, Quote, Symbol, Trend};
pub use storage::{Database, StorageError, StorageResult};
