//! 新闻与公告接口实现
//!
//! 数据来源：
//! - 市场快讯：东方财富 newsapi.eastmoney.com（多频道合并，实时市场资讯）
//! - 公司公告：东方财富 np-anotice-stock.eastmoney.com（如可用）

use chrono::Local;

use crate::model::{Announcement, NewsItem, StockNews, Symbol};

/// EastMoney 快讯频道 ID
const KUAIXUN_LIDS: &[&str] = &["101", "102", "103", "107"];

pub struct EastMoneyNewsProvider {
    client: reqwest::Client,
}

impl EastMoneyNewsProvider {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .build()
            .expect("构建 HTTP 客户端失败");
        Self { client }
    }

    fn decode_body(bytes: &[u8]) -> String {
        if let Ok(s) = std::str::from_utf8(bytes) { return s.to_string(); }
        let (cow, _, had_errors) = encoding_rs::GBK.decode(bytes);
        if had_errors { String::from_utf8_lossy(bytes).into_owned() } else { cow.into_owned() }
    }

    /// 拉取公司公告（东方财富）
    async fn fetch_announcements(&self, symbol: &Symbol, page: usize, page_size: usize) -> Vec<Announcement> {
        let market = match symbol.market.code() { "SH" => "sh", "SZ" => "sz", "BJ" => "bj", _ => "sh" };
        let url = "https://np-anotice-stock.eastmoney.com/notice/search";
        let params = [
            ("cb", "callback"), ("page", &page.to_string()), ("pageSize", &page_size.to_string()),
            ("announceType", "0101,0102,0103,0104,0105,0106,0107,0108,0109,0110,0111,0112,0113,0114,0115,0116,0117,0118,0119,0120,0121,0122,0123,0124"),
            ("stockCode", &format!("{}{}", market, symbol.code)), ("endDate", ""), ("startDate", ""),
            ("_", &chrono::Utc::now().timestamp_millis().to_string()),
        ];

        let resp = match self.client.get(url).query(&params)
            .header("Referer", "https://data.eastmoney.com/")
            .header("Accept", "application/json, text/javascript, */*; q=0.01")
            .send().await {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };

        let bytes = match resp.bytes().await { Ok(b) => b, Err(_) => return Vec::new() };
        let body = Self::decode_body(&bytes);
        if body.is_empty() || body.len() < 20 { return Vec::new(); }

        let json_str = body.strip_prefix("callback(").map(|s| s.trim().trim_end_matches(')')).unwrap_or(&body);
        let data: serde_json::Value = match serde_json::from_str(json_str) { Ok(v) => v, Err(_) => return Vec::new() };
        let items = match data.get("data").and_then(|d| d.get("list")).and_then(|v| v.as_array()) {
            Some(v) => v,
            None => return Vec::new(),
        };

        items.iter().take(page_size).filter_map(|item| {
            let title = item.get("title")?.as_str()?.to_string();
            let date = item.get("notice_date").or_else(|| item.get("publishDate"))
                .and_then(|s| s.as_str()).unwrap_or("").to_string();
            let notice_type = item.get("announceType").or_else(|| item.get("type"))
                .and_then(|s| s.as_str()).unwrap_or("公告").to_string();
            let url = item.get("art_url").or_else(|| item.get("url"))
                .and_then(|u| u.as_str()).unwrap_or("").to_string();
            Some(Announcement { title, date, notice_type, url })
        }).collect()
    }

    /// 拉取单个频道的快讯
    async fn fetch_kuaixun_lid(&self, lid: &str) -> Vec<NewsItem> {
        let url = format!(
            "https://newsapi.eastmoney.com/kuaixun/v1/getlist_{}_ajaxResult_1_10_.html",
            lid
        );

        let resp = match self.client.get(&url)
            .header("Referer", "https://finance.eastmoney.com/")
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .header("Accept", "application/json, text/javascript, */*; q=0.01")
            .send().await {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };

        let bytes = match resp.bytes().await { Ok(b) => b, Err(_) => return Vec::new() };
        let body = Self::decode_body(&bytes);
        if body.is_empty() || body.len() < 10 { return Vec::new(); }

        let json_str = body.strip_prefix("var ajaxResult=").map(|s| s.trim()).unwrap_or(&body);
        let data: serde_json::Value = match serde_json::from_str(json_str) { Ok(v) => v, Err(_) => return Vec::new() };

        let items = match data.get("LivesList").and_then(|v| v.as_array()) {
            Some(v) => v,
            None => return Vec::new(),
        };

        items.iter().filter_map(|item| {
            let title = item.get("title").and_then(|t| t.as_str())?.to_string();
            let summary = item.get("digest").or_else(|| item.get("intro"))
                .and_then(|s| s.as_str()).unwrap_or("").to_string();
            let sort_str = item.get("sort").and_then(|s| s.as_str()).unwrap_or("0");
            let datetime = sort_to_datetime(sort_str).unwrap_or_else(Local::now);
            let url = item.get("url_w").or_else(|| item.get("url_m"))
                .and_then(|u| u.as_str()).unwrap_or("").to_string();
            Some(NewsItem { title, summary, datetime, source: "东方财富".to_string(), url })
        }).collect()
    }

    /// 并行拉取多频道东方财富快讯
    async fn fetch_eastmoney_news(&self) -> Vec<NewsItem> {
        let mut handles = Vec::new();
        for lid in KUAIXUN_LIDS {
            let fut = self.fetch_kuaixun_lid(lid);
            handles.push(fut);
        }

        let results = futures::future::join_all(handles).await;
        let mut all_news = Vec::new();
        for news in results {
            all_news.extend(news);
        }

        // 按时间倒序
        all_news.sort_by(|a, b| b.datetime.cmp(&a.datetime));
        all_news
    }
}

impl Default for EastMoneyNewsProvider { fn default() -> Self { Self::new() } }

/// 从 sort 字段解析时间（19 位毫秒时间戳，取前 10 位为 Unix 秒）
fn sort_to_datetime(sort: &str) -> Option<chrono::DateTime<Local>> {
    if sort.len() < 10 { return None; }
    let secs: i64 = sort[..10].parse().ok()?;
    if secs > 0 {
        chrono::DateTime::from_timestamp(secs, 0).map(|dt| dt.with_timezone(&Local))
    } else { None }
}

fn parse_datetime(s: &str) -> Option<chrono::DateTime<Local>> {
    if s.is_empty() { return None; }
    if let Ok(ts) = s.parse::<i64>() {
        let secs = if ts > 1_000_000_000_000 { ts / 1000 } else { ts };
        if secs > 0 { return chrono::DateTime::from_timestamp(secs, 0).map(|dt| dt.with_timezone(&Local)); }
    }
    for fmt in &["%Y-%m-%d %H:%M:%S", "%Y/%m/%d %H:%M:%S", "%Y-%m-%d"] {
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, fmt) {
            return Some(dt.and_local_timezone(Local).single()?);
        }
    }
    None
}

impl EastMoneyNewsProvider {
    /// 获取快讯和公告
    pub async fn fetch_all(&self, symbol: &Symbol) -> crate::error::Result<StockNews> {
        let announce_fut = self.fetch_announcements(symbol, 1, 5);
        let news = self.fetch_eastmoney_news().await;
        let announcements = announce_fut.await;
        Ok(StockNews { news, announcements })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_sort_to_datetime() {
        let dt = sort_to_datetime("1789016981023792");
        assert!(dt.is_some());
        assert_eq!(dt.unwrap().timestamp(), 1789016981);
    }
}