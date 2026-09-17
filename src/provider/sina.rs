//! 新浪财经历史 K 线接口
//!
//! 接口：`https://money.finance.sina.com.cn/quotes_service/api/json_v2.php/CN_MarketData.getKLineData`
//!
//! 参数：
//! - `symbol=sh600519`（沪市）/ `sz000001`（深市）/ `bj830799`（北交所）
//! - `scale=240`（日 K 周期，单位：分钟）
//! - `datalen=N`（返回最近 N 条）
//! - `ma=no`（不要 MA 字段，节省带宽）
//!
//! 响应：纯 JSON 数组，每条 K 线形如
//! ```json
//! {"day":"2026-09-03","open":"1297.500","high":"1305.000","low":"1293.020","close":"1298.880","volume":"1774765"}
//! ```
//!
//! 注意：服务端返回的字符串可能含 BOM/HTML 噪音，需要清洗。

use async_trait::async_trait;
use chrono::NaiveDate;

use super::KlineProvider;
use crate::error::{Error, Result};
use crate::model::{Kline, KlineSeries, Market, Symbol};

/// 新浪 K 线 Provider
#[derive(Debug, Clone, Default)]
pub struct SinaKlineProvider {
    client: reqwest::Client,
    base_url: String,
}

impl SinaKlineProvider {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .build()
            .expect("构建 HTTP 客户端失败");
        Self {
            client,
            base_url:
                "https://money.finance.sina.com.cn/quotes_service/api/json_v2.php/CN_MarketData.getKLineData"
                    .to_string(),
        }
    }

    /// 把 `Symbol` 转为新浪的 `sh600519` / `sz000001` / `bj830799` 形式
    fn sina_symbol(sym: &Symbol) -> String {
        let prefix = match sym.market {
            Market::Shanghai => "sh",
            Market::Shenzhen => "sz",
            Market::Beijing => "bj",
        };
        format!("{}{}", prefix, sym.code)
    }

    /// 清洗响应字符串：去掉 BOM、前后空白，以及接口偶发的 HTML 噪音
    fn clean_response(raw: &str) -> &str {
        let s = raw.trim_start_matches('\u{feff}').trim();
        match s.find('[') {
            Some(idx) => &s[idx..],
            None => s,
        }
    }

    /// 解析 JSON 响应
    fn parse(json_text: &str) -> Result<KlineSeries> {
        let cleaned = Self::clean_response(json_text);
        let arr: Vec<serde_json::Value> = serde_json::from_str(cleaned)
            .map_err(|e| Error::BadResponse(format!("JSON 解析失败: {}", e)))?;

        let mut bars = Vec::with_capacity(arr.len());
        for v in arr {
            let obj = v.as_object().ok_or_else(|| {
                Error::BadResponse("K 线数组元素不是对象".into())
            })?;

            let date_str = obj
                .get("day")
                .and_then(|x| x.as_str())
                .ok_or_else(|| Error::BadResponse("缺少 day 字段".into()))?;
            let date = NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
                .map_err(|e| Error::BadResponse(format!("日期解析失败 {}: {}", date_str, e)))?;

            let open = parse_f64(obj.get("open"))?;
            let high = parse_f64(obj.get("high"))?;
            let low = parse_f64(obj.get("low"))?;
            let close = parse_f64(obj.get("close"))?;
            let volume = parse_i64(obj.get("volume"))?;

            bars.push(Kline {
                date,
                open,
                high,
                low,
                close,
                volume,
            });
        }
        Ok(KlineSeries::new(bars))
    }
}

fn parse_f64(v: Option<&serde_json::Value>) -> Result<f64> {
    v.and_then(|x| x.as_str())
        .and_then(|s| s.parse().ok())
        .or_else(|| v.and_then(|x| x.as_f64()))
        .ok_or_else(|| Error::BadResponse("数值字段缺失或类型错".into()))
}

fn parse_i64(v: Option<&serde_json::Value>) -> Result<i64> {
    v.and_then(|x| x.as_str())
        .and_then(|s| s.parse().ok())
        .or_else(|| v.and_then(|x| x.as_i64()))
        .ok_or_else(|| Error::BadResponse("整数字段缺失或类型错".into()))
}

#[async_trait]
impl KlineProvider for SinaKlineProvider {
    fn name(&self) -> &'static str {
        "sina"
    }

    async fn fetch(&self, symbol: &Symbol, days: usize) -> Result<KlineSeries> {
        let url = format!(
            "{}?symbol={}&scale=240&ma=no&datalen={}",
            self.base_url,
            Self::sina_symbol(symbol),
            days.max(1)
        );

        let resp = self
            .client
            .get(&url)
            .header("Referer", "https://finance.sina.com.cn/")
            .send()
            .await?;
        let body = resp.text().await?;

        if body.is_empty() || !body.contains('[') {
            return Err(Error::NoData(format!(
                "新浪返回空响应（股票 {} 可能不存在）",
                symbol
            )));
        }
        Self::parse(&body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_response_with_leading_html() {
        let raw = "<html><body>oops</body></html>[{\"day\":\"2026-09-03\",\"open\":\"1\",\"high\":\"1\",\"low\":\"1\",\"close\":\"1\",\"volume\":\"1\"}]";
        let cleaned = SinaKlineProvider::clean_response(raw);
        assert!(cleaned.starts_with('['));
    }

    #[test]
    fn test_clean_response_bom() {
        let raw = "\u{feff}[{\"day\":\"2026-09-03\",\"open\":\"1\",\"high\":\"1\",\"low\":\"1\",\"close\":\"1\",\"volume\":\"1\"}]";
        let cleaned = SinaKlineProvider::clean_response(raw);
        assert!(cleaned.starts_with('['));
    }

    #[test]
    fn test_parse_valid() {
        let json = r#"[{"day":"2026-09-03","open":"1297.500","high":"1305.000","low":"1293.020","close":"1298.880","volume":"1774765"},{"day":"2026-09-04","open":"1295.880","high":"1338.860","low":"1295.600","close":"1330.000","volume":"4541564"}]"#;
        let s = SinaKlineProvider::parse(json).unwrap();
        assert_eq!(s.len(), 2);
        assert!((s.bars[0].close - 1298.880).abs() < 1e-6);
        assert_eq!(s.bars[1].volume, 4541564);
    }

    #[test]
    fn test_parse_invalid() {
        assert!(SinaKlineProvider::parse("not json").is_err());
        assert!(SinaKlineProvider::parse("[]").unwrap().is_empty());
    }

    #[test]
    fn test_sina_symbol() {
        let s = Symbol::new("600519").unwrap();
        assert_eq!(SinaKlineProvider::sina_symbol(&s), "sh600519");
        let s = Symbol::new("000001").unwrap();
        assert_eq!(SinaKlineProvider::sina_symbol(&s), "sz000001");
        let s = Symbol::new("830799").unwrap();
        assert_eq!(SinaKlineProvider::sina_symbol(&s), "bj830799");
    }
}
