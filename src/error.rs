//! 统一的错误类型定义
//!
//! 提供 `Error` 枚举用于库内部错误传播，
//! 以及 `Result<T>` 别名用于简化函数签名。

use thiserror::Error;

/// gupiao 库的统一错误类型
#[derive(Debug, Error)]
pub enum Error {
    /// 用户输入的股票代码无效（包含非数字字符或长度异常）
    #[error("无效的股票代码: {0}")]
    InvalidCode(String),

    /// 无法根据代码判断所属市场
    #[error("无法识别股票代码 {code} 所属市场")]
    UnknownMarket { code: String },

    /// HTTP 请求失败
    #[error("网络请求失败: {0}")]
    Http(#[from] reqwest::Error),

    /// 接口返回的响应格式异常
    #[error("接口响应解析失败: {0}")]
    BadResponse(String),

    /// 数据源未返回数据（例如股票不存在或非交易时段）
    #[error("未获取到行情数据{0}")]
    NoData(String),

    /// 其它 IO 错误
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
