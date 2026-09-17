//! 分析报告的彩色终端输出

use std::io::{self, Write};

use colored::*;

use crate::analysis::{AnalysisReport, FusionStrategy, SignalKind};

/// 分析报告展示器
pub struct AnalysisDisplay;

impl AnalysisDisplay {
    /// 把分析报告写入输出流
    pub fn write<W: Write>(&self, w: &mut W, report: &AnalysisReport) -> io::Result<()> {
        write_header(w, report)?;
        write_signal_banner(w, report)?;
        write_model_info(w, report)?;
        write_indicators(w, report)?;
        write_levels(w, report)?;
        write_advice(w, report)?;
        write_news(w, report)?;
        Ok(())
    }
}

/// M3 新增：模型融合信息
fn write_model_info<W: Write>(w: &mut W, r: &AnalysisReport) -> io::Result<()> {
    if let Some(prob) = r.model_prob {
        let strategy = r.fusion_strategy.unwrap_or(FusionStrategy::Weighted);
        let prob_pct = prob * 100.0;
        let prob_colored = if prob >= 0.6 {
            format!("{:.1}%", prob_pct).red().bold().to_string()
        } else if prob <= 0.4 {
            format!("{:.1}%", prob_pct).green().bold().to_string()
        } else {
            format!("{:.1}%", prob_pct).yellow().to_string()
        };
        writeln!(
            w,
            "  模型置信: {}  (融合: {})",
            prob_colored,
            strategy.label().dimmed()
        )?;
        if (r.rule_score - r.final_score).abs() > 1e-6 {
            writeln!(
                w,
                "  得分分解: 规则={:+.1}  最终={:+.1}",
                r.rule_score, r.final_score
            )?;
        }
        writeln!(w)?;
    } else if let Some(strategy) = r.fusion_strategy {
        // 模型不可用（K 线不足），但策略被设置了
        writeln!(
            w,
            "  {} 模型不可用（数据不足），策略={}（纯规则）",
            "⚠".yellow(),
            strategy.label().dimmed()
        )?;
        writeln!(w)?;
    }
    Ok(())
}

fn write_header<W: Write>(w: &mut W, r: &AnalysisReport) -> io::Result<()> {
    let title = format!(
        " {} {}.{} - 自动分析 ",
        r.symbol_name, r.symbol_code, r.market
    );
    writeln!(w, "{}", title.on_blue().white().bold())?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    writeln!(w, "  {}", now.dimmed())?;
    writeln!(w, "  {}", "─".repeat(60).dimmed())?;
    Ok(())
}

fn write_signal_banner<W: Write>(w: &mut W, r: &AnalysisReport) -> io::Result<()> {
    let label = format!(" {} ", r.signal.kind.label());
    let strength_pct = r.signal.strength;
    let banner = match r.signal.kind {
        SignalKind::StrongBuy => label.on_red().white().bold().to_string(),
        SignalKind::Buy => label.red().bold().to_string(),
        SignalKind::Hold => label.yellow().bold().to_string(),
        SignalKind::Sell => label.green().bold().to_string(),
        SignalKind::StrongSell => label.on_green().white().bold().to_string(),
    };
    writeln!(
        w,
        "  操作建议: {}  强度: {}%",
        banner,
        strength_pct
    )?;
    if !r.key_factors.is_empty() {
        writeln!(
            w,
            "  关键因子: {}",
            r.key_factors.join(" | ").cyan()
        )?;
    }
    writeln!(w)
}

fn write_indicators<W: Write>(w: &mut W, r: &AnalysisReport) -> io::Result<()> {
    writeln!(w, "  {}", "技术指标".yellow().bold())?;
    let ind = &r.indicators;
    writeln!(
        w,
        "    位置: {:.1}%   委比: {:+.1}%   量比: {:.2}   价量: {:.0}",
        ind.position * 100.0,
        ind.committee_ratio * 100.0,
        ind.volume_ratio,
        ind.pv_ratio,
    )?;
    writeln!(
        w,
        "    趋势强度: {:+.2}   当日振幅: {}",
        ind.trend_strength,
        ind.amplitude
            .map(|a| format!("{:.2}%", a))
            .unwrap_or_else(|| "-".into())
    )?;
    writeln!(w)
}

fn write_levels<W: Write>(w: &mut W, r: &AnalysisReport) -> io::Result<()> {
    writeln!(w, "  {}", "价位预测".yellow().bold())?;
    if let Some(s) = r.support {
        let dist = (r.current_price - s.price) / r.current_price * 100.0;
        writeln!(
            w,
            "    建议买点: {:>8.2}  距现价 {:>5.2}%  ({})",
            s.price,
            dist,
            s.source.label().dimmed()
        )?;
    } else {
        writeln!(w, "    建议买点: {}", "(无足够数据)".dimmed())?;
    }
    if let Some(rs) = r.resistance {
        let upside = (rs.price - r.current_price) / r.current_price * 100.0;
        writeln!(
            w,
            "    建议卖点: {:>8.2}  距现价 {:>5.2}%  ({})",
            rs.price,
            upside,
            rs.source.label().dimmed()
        )?;
    } else {
        writeln!(w, "    建议卖点: {}", "(无足够数据)".dimmed())?;
    }
    writeln!(w)
}

fn write_advice<W: Write>(w: &mut W, r: &AnalysisReport) -> io::Result<()> {
    writeln!(w, "  {}", "详细建议".yellow().bold())?;
    for (i, line) in r.advice.iter().enumerate() {
        if i == 0 {
            // 第一行是综合得分，单独高亮
            writeln!(w, "    {}", line.bright_white())?;
        } else if line.starts_with("⚠") {
            writeln!(w, "    {}", line.red())?;
        } else if line.starts_with("  ") {
            writeln!(w, "    {}", line.dimmed())?;
        } else {
            writeln!(w, "    {}", line)?;
        }
    }
    writeln!(w, "  {}", "─".repeat(60).dimmed())?;
    Ok(())
}

fn write_news<W: Write>(w: &mut W, r: &AnalysisReport) -> io::Result<()> {
    if let Some(ref news) = r.news {
        let has_announcements = !news.announcements.is_empty();
        let has_items = !news.news.is_empty();

        if has_announcements || has_items {
            writeln!(w, "  {}", "最新资讯".yellow().bold())?;

            if has_announcements {
                writeln!(w, "  公司公告:")?;
                for ann in news.announcements.iter().take(5) {
                    writeln!(
                        w,
                        "    [{}] {}",
                        ann.date.dimmed(),
                        truncate_cn(&ann.title, 50)
                    )?;
                }
                if news.announcements.len() > 5 {
                    writeln!(w, "    ... 还有 {} 条公告", news.announcements.len() - 5)?;
                }
            }

            if has_items {
                writeln!(w, "  最新资讯/股评:")?;
                for item in news.news.iter().take(5) {
                    let time = item.datetime.format("%m-%d %H:%M").to_string();
                    writeln!(
                        w,
                        "    [{}] {} - {}",
                        time.dimmed(),
                        truncate_cn(&item.title, 45),
                        item.source.dimmed()
                    )?;
                }
                if news.news.len() > 5 {
                    writeln!(w, "    ... 还有 {} 条资讯", news.news.len() - 5)?;
                }
            }
            writeln!(w, "  {}", "─".repeat(60).dimmed())?;
        }
    }
    Ok(())
}

/// 截断中文字符串到指定显示宽度
fn truncate_cn(s: &str, max_width: usize) -> String {
    let mut width = 0;
    let mut result = String::new();
    for c in s.chars() {
        let cw = if c.is_ascii() { 1 } else { 2 };
        if width + cw > max_width {
            break;
        }
        width += cw;
        result.push(c);
    }
    if width < s.chars().count() {
        result.push_str("..");
    }
    result
}