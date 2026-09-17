//! 市场（交易所）枚举与代码归一化工具
//!
//! 单一事实来源：所有关于"这只股票属于哪个市场"的判断都集中在这里。

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};

/// A 股三大市场
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Market {
    /// 上交所（沪市）
    Shanghai,
    /// 深交所（深市，含创业板/中小板）
    Shenzhen,
    /// 北交所
    Beijing,
}

impl Market {
    /// 字母缩写
    pub fn code(self) -> &'static str {
        match self {
            Market::Shanghai => "SH",
            Market::Shenzhen => "SZ",
            Market::Beijing => "BJ",
        }
    }

    /// 中文名称
    pub fn name(self) -> &'static str {
        match self {
            Market::Shanghai => "沪市",
            Market::Shenzhen => "深市",
            Market::Beijing => "北交所",
        }
    }

    /// 东方财富接口使用的数字市场代号
    pub fn eastmoney_id(self) -> &'static str {
        match self {
            Market::Shanghai => "1",
            Market::Shenzhen | Market::Beijing => "0",
        }
    }

    /// 腾讯接口使用的前缀（小写）
    pub fn tencent_prefix(self) -> &'static str {
        match self {
            Market::Shanghai => "sh",
            Market::Shenzhen => "sz",
            Market::Beijing => "bj",
        }
    }

    /// 由字母代码解析
    pub fn from_code(s: &str) -> Option<Self> {
        match s.to_ascii_uppercase().as_str() {
            "SH" => Some(Market::Shanghai),
            "SZ" => Some(Market::Shenzhen),
            "BJ" => Some(Market::Beijing),
            _ => None,
        }
    }
}

impl std::fmt::Display for Market {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}

/// 解析后的股票标识
///
/// 包含原始输入归一化后的所有信息。
/// 一个 `Symbol` 可同时用于多个数据源（腾讯/东财）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    /// 归一化的 6 位股票代码
    pub code: String,
    /// 自动识别或用户指定的市场
    pub market: Market,
}

impl Symbol {
    /// 创建 Symbol，自动检测市场
    pub fn new(code: &str) -> Result<Self> {
        Self::with_market(code, None)
    }

    /// 创建 Symbol，可显式指定市场
    pub fn with_market(code: &str, market: Option<Market>) -> Result<Self> {
        let normalized = normalize_input(code)?;
        let market = match market {
            Some(m) => m,
            None => detect_market(&normalized).ok_or_else(|| Error::UnknownMarket {
                code: code.to_string(),
            })?,
        };
        Ok(Self {
            code: normalized,
            market,
        })
    }

    /// 东财 secid 格式：`{market_id}.{code}`，如 `1.600519`
    pub fn eastmoney_secid(&self) -> String {
        format!("{}.{}", self.market.eastmoney_id(), self.code)
    }

    /// 腾讯 q 参数后缀：`{prefix}{code}`，如 `sh600519`
    pub fn tencent_key(&self) -> String {
        format!("{}{}", self.market.tencent_prefix(), self.code)
    }
}

impl std::fmt::Display for Symbol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.code, self.market)
    }
}

/// 把各种用户输入格式归一化为纯 6 位数字代码
fn normalize_input(input: &str) -> Result<String> {
    let s = input.trim().to_lowercase();
    let s = s
        .strip_suffix(".sh")
        .or_else(|| s.strip_suffix(".sz"))
        .or_else(|| s.strip_suffix(".bj"))
        .unwrap_or(&s);

    let digits = if s.starts_with("sh") || s.starts_with("sz") || s.starts_with("bj") {
        &s[2..]
    } else {
        s
    };

    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err(Error::InvalidCode(input.to_string()));
    }
    Ok(format!("{:0>6}", digits))
}

/// 根据代码前缀识别市场
fn detect_market(code: &str) -> Option<Market> {
    if code.len() != 6 {
        return None;
    }
    match code {
        c if c.starts_with("600")
            || c.starts_with("601")
            || c.starts_with("603")
            || c.starts_with("605") =>
        {
            Some(Market::Shanghai)
        }
        c if c.starts_with("688") => Some(Market::Shanghai),
        c if c.starts_with("900") => Some(Market::Shanghai),
        c if c.starts_with('5') => Some(Market::Shanghai),
        c if c.starts_with("11") || c.starts_with("13") => Some(Market::Shanghai),
        c if c.starts_with("000")
            || c.starts_with("001")
            || c.starts_with("002")
            || c.starts_with("003") =>
        {
            Some(Market::Shenzhen)
        }
        c if c.starts_with("300") || c.starts_with("301") => Some(Market::Shenzhen),
        c if c.starts_with("200") => Some(Market::Shenzhen),
        c if c.starts_with("15") || c.starts_with("16") || c.starts_with("18") => {
            Some(Market::Shenzhen)
        }
        c if c.starts_with('8') || c.starts_with('4') || c.starts_with("920") => {
            Some(Market::Beijing)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_input() {
        assert_eq!(normalize_input("600519").unwrap(), "600519");
        assert_eq!(normalize_input("sh600519").unwrap(), "600519");
        assert_eq!(normalize_input("600519.SH").unwrap(), "600519");
        assert_eq!(normalize_input("sz000001").unwrap(), "000001");
        assert_eq!(normalize_input("bj830799").unwrap(), "830799");
        assert_eq!(normalize_input("  600519  ").unwrap(), "600519");
        assert!(normalize_input("abc").is_err());
        assert!(normalize_input("").is_err());
    }

    #[test]
    fn test_detect_market() {
        assert_eq!(detect_market("600519"), Some(Market::Shanghai));
        assert_eq!(detect_market("688981"), Some(Market::Shanghai));
        assert_eq!(detect_market("000001"), Some(Market::Shenzhen));
        assert_eq!(detect_market("300750"), Some(Market::Shenzhen));
        assert_eq!(detect_market("830799"), Some(Market::Beijing));
        assert_eq!(detect_market("920985"), Some(Market::Beijing));
        assert_eq!(detect_market("999999"), None);
    }

    #[test]
    fn test_symbol() {
        let s = Symbol::new("sh600519").unwrap();
        assert_eq!(s.code, "600519");
        assert_eq!(s.market, Market::Shanghai);
        assert_eq!(s.eastmoney_secid(), "1.600519");
        assert_eq!(s.tencent_key(), "sh600519");

        let s = Symbol::new("300750").unwrap();
        assert_eq!(s.market, Market::Shenzhen);
        assert_eq!(s.tencent_key(), "sz300750");
    }
}
