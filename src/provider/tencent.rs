//! 腾讯财经行情接口实现
//!
//! 接口：`http://qt.gtimg.cn/q={sh|sz|bj}{code}`
//! 特点：
//! - 公开免费，无需鉴权
//! - 数据完整（含五档）
//! - 服务端返回 **GBK** 编码（已自动处理）
//!
//! 响应格式（`~` 分隔）：
//! ```text
//! v_sh600519="1~贵州茅台~600519~现价~昨收~开~成交量~外盘~内盘~买一价~买一量~...~卖一价~...~"
//! ```
//!

use async_trait::async_trait;
use chrono::Local;

use super::QuoteProvider;
use crate::error::{Error, Result};
use crate::model::{Depth, Market, Quote, Symbol};

/// 腾讯财经行情 Provider
pub struct TencentProvider {
    /// 复用的 HTTP 客户端
    client: reqwest::Client,
    /// 接口基础地址
    base_url: String,
}

impl TencentProvider {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .build()
            .expect("构建 HTTP 客户端失败");
        Self {
            client,
            base_url: "http://qt.gtimg.cn/q=".to_string(),
        }
    }

    /// 从接口响应字节解码为字符串（处理 GBK 编码）
    fn decode_body(bytes: &[u8]) -> String {
        let (cow, _, had_errors) = encoding_rs::GBK.decode(bytes);
        if had_errors {
            String::from_utf8_lossy(bytes).into_owned()
        } else {
            cow.into_owned()
        }
    }

    /// 解析 `v_sh600519="..."` 的响应主体
    fn parse(body: &str) -> Result<Quote> {
        let inner = body
            .split('"')
            .nth(1)
            .ok_or_else(|| Error::BadResponse("缺少响应主体".into()))?;
        let fields: Vec<&str> = inner.split('~').collect();
        if fields.len() < 50 {
            return Err(Error::BadResponse(format!("字段不足: {}", fields.len())));
        }

        let f = |i: usize| -> f64 { fields.get(i).and_then(|s| s.parse().ok()).unwrap_or(0.0) };
        let fo = |i: usize| -> Option<f64> {
            fields
                .get(i)
                .and_then(|s| if s.is_empty() { None } else { s.parse().ok() })
        };
        let io = |i: usize| -> Option<i64> {
            fields
                .get(i)
                .and_then(|s| if s.is_empty() { None } else { s.parse().ok() })
        };

        // 字段 0 是腾讯市场标识：1=沪市，51/0=深市，62/60/2/3=北交所
        let market = match fields.get(0).copied().unwrap_or("") {
            "1" => Market::Shanghai,
            "51" | "0" | "50" => Market::Shenzhen,
            "62" | "60" | "2" | "3" => Market::Beijing,
            _ => Market::Shenzhen,
        };

        let code = fields.get(2).unwrap_or(&"").to_string();
        let name = fields.get(1).unwrap_or(&"").to_string();
        let price = f(3);
        let prev_close = f(4);
        let change = f(31);
        let change_pct = f(32);

        let update_time = Quote::parse_tencent_time(fields.get(30).copied().unwrap_or(""))
            .unwrap_or_else(|| Local::now());

        let depth = Depth {
            bid_prices: [fo(9), fo(11), fo(13), fo(15), fo(17)],
            bid_vols: [io(10), io(12), io(14), io(16), io(18)],
            ask_prices: [fo(19), fo(21), fo(23), fo(25), fo(27)],
            ask_vols: [io(20), io(22), io(24), io(26), io(28)],
        };

        Ok(Quote {
            code,
            name,
            market,
            price,
            prev_close,
            open: f(5),
            high: f(33),
            low: f(34),
            change,
            change_pct,
            volume: f(6) as i64,
            amount: f(37),
            turnover_rate: fo(38),
            pe: fo(39),
            pb: fo(46),
            market_cap: fo(45),
            float_cap: fo(44),
            amplitude: fo(43),
            update_time,
            depth,
        })
    }
}

impl Default for TencentProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl QuoteProvider for TencentProvider {
    fn name(&self) -> &'static str {
        "tencent"
    }

    async fn fetch(&self, symbol: &Symbol) -> Result<Quote> {
        let url = format!("{}{}", self.base_url, symbol.tencent_key());
        let resp = self
            .client
            .get(&url)
            .header("Referer", "https://gu.qq.com/")
            .send()
            .await?;
        let bytes = resp.bytes().await?;
        let body = Self::decode_body(&bytes);

        if !body.contains('=') || body.trim().is_empty() {
            return Err(Error::NoData(String::new()));
        }
        Self::parse(&body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_tencent_response() {
        let raw = r#"v_sh600519="1~贵州茅台~600519~1316.01~1330.00~1324.00~25250~11260~13989~1316.01~44~1316.00~37~1315.80~1~1315.67~1~1315.62~1~1316.02~1~1316.07~1~1316.13~1~1316.15~3~1316.17~2~~20260907161458~-13.99~-1.05~1333.60~1312.66~1316.01/25250/3336029501~25250~333603~0.20~20.20~~1333.60~1312.66~1.57~16451.20~16451.20~6.55~1463.00~1197.00~0.91~76~1321.22~18.48~19.98~~~0.09~333602.9501~52.6404~4~   A~GP-A~-2.46~1.27~3.95~32.41~27.30~1539.98~1151.01~0.87~-2.44~4.12~1250081601~1250081601~82.61~-2.58~1250081601~~~-8.04~-0.05~~CNY~0~___D__F__N~1315.05~6~";"#;
        let q = TencentProvider::parse(raw).unwrap();
        assert_eq!(q.code, "600519");
        assert_eq!(q.market, Market::Shanghai);
        assert_eq!(q.name, "贵州茅台");
        assert!((q.price - 1316.01).abs() < 1e-6);
        assert_eq!(q.depth.bid_prices[0], Some(1316.01));
    }
}
