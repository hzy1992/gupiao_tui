//! 股票新闻与公告数据模型
//!
//! 数据来源：东方财富（East Money）公开接口
//! - 快讯/资讯：https://newsapi.eastmoney.com/kuaixun/v1/getlist_101_ajaxResult_{page}_{pageSize}_.html
//! - 公司公告：https://np-anotice-stock.eastmoney.com/notice/search

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

/// 单条资讯（快讯/新闻/股评）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewsItem {
    /// 标题
    pub title: String,
    /// 摘要（可能为空）
    pub summary: String,
    /// 发布时间
    pub datetime: DateTime<Local>,
    /// 来源媒体
    pub source: String,
    /// 原文 URL（可能为空）
    pub url: String,
}

/// 单条公司公告
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Announcement {
    /// 公告标题
    pub title: String,
    /// 公告日期
    pub date: String,
    /// 公告类型（如"业绩预告"、"董事会决议"等）
    pub notice_type: String,
    /// 原文链接
    pub url: String,
}

/// 股票新闻汇总（资讯 + 公告）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StockNews {
    /// 最新资讯/快讯/股评（按时间倒序）
    pub news: Vec<NewsItem>,
    /// 最新公司公告（按日期倒序）
    pub announcements: Vec<Announcement>,
}

impl StockNews {
    /// 是否为空（没有任何数据）
    pub fn is_empty(&self) -> bool {
        self.news.is_empty() && self.announcements.is_empty()
    }
}
