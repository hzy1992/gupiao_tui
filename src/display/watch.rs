//! 自选股列表式输出（多只股票一览）
//!
//! 输出为一张紧凑的表格：
//!
//! ```text
//!   名称       代码     市场    现价       涨跌        涨跌幅     换手率
//!   贵州茅台   600519   SH     1316.01   -13.99       -1.05%      0.20%
//!   ...
//! ```

use std::io::{self, Write};

use colored::*;

use crate::model::{Quote, Trend};

/// 自选股列表展示器
pub struct WatchDisplay;

impl WatchDisplay {
    /// 把一组行情写入输出流
    pub fn write_all<W: Write>(&self, w: &mut W, quotes: &[Quote]) -> io::Result<()> {
        if quotes.is_empty() {
            writeln!(w, "{}", "  (空) 当前没有可显示的行情".dimmed())?;
            return Ok(());
        }

        // 表头
        writeln!(
            w,
            "  {} {} {} {} {} {} {}",
            pad_right_cn("名称", 10),
            pad_right_cn("代码", 8),
            pad_left("市场", 4),
            pad_left("现价", 10),
            pad_left("涨跌", 10),
            pad_left("涨跌幅", 10),
            pad_left("更新时间", 20),
        )?;
        writeln!(w, "  {}", "─".repeat(76).dimmed())?;

        for q in quotes {
            write_row(w, q)?;
        }
        Ok(())
    }
}

fn write_row<W: Write>(w: &mut W, q: &Quote) -> io::Result<()> {
    let trend = q.trend();
    let name_colored = match trend {
        Trend::Up => q.name.red().bold().to_string(),
        Trend::Down => q.name.green().bold().to_string(),
        Trend::Flat => q.name.normal().to_string(),
    };
    let price_str = format!("{:>8.2}", q.price);
    let price_colored = match trend {
        Trend::Up => price_str.red().bold().to_string(),
        Trend::Down => price_str.green().bold().to_string(),
        Trend::Flat => price_str.white().to_string(),
    };
    let change_str = format!("{:+.2}", q.change);
    let change_colored = match trend {
        Trend::Up => change_str.red().to_string(),
        Trend::Down => change_str.green().to_string(),
        Trend::Flat => change_str.normal().to_string(),
    };
    let pct_str = format!("{:+.2}%", q.change_pct);
    let pct_colored = match trend {
        Trend::Up => pct_str.red().to_string(),
        Trend::Down => pct_str.green().to_string(),
        Trend::Flat => pct_str.normal().to_string(),
    };

    let time = q.update_time.format("%Y-%m-%d %H:%M:%S").to_string();

    writeln!(
        w,
        "  {} {} {}  {}   {}  {}  {}",
        pad_right_cn_to_width(&name_colored, 10),
        format!("{}.{}", q.code, q.market),
        pad_left(q.market.code(), 4),
        price_colored,
        change_colored,
        pct_colored,
        time.dimmed(),
    )
}

/// 中文按显示宽度（左对齐填充空格）
fn pad_right_cn(s: &str, width: usize) -> String {
    let w = display_width(s);
    if w >= width {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(width - w))
    }
}

fn pad_right_cn_to_width(s: &str, width: usize) -> String {
    // 已含 ANSI 颜色码，需要按「显示宽度」裁剪
    let w = display_width(s);
    if w >= width {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(width - w))
    }
}

/// 左对齐（数字列）：先取指定显示宽，再左补空格
fn pad_left(s: &str, width: usize) -> String {
    let w = display_width(s);
    if w >= width {
        s.to_string()
    } else {
        format!("{}{}", " ".repeat(width - w), s)
    }
}

/// 粗略的「显示宽度」：CJK 字符算 2，其它算 1
fn display_width(s: &str) -> usize {
    s.chars().map(|c| if is_wide(c) { 2 } else { 1 }).sum()
}

fn is_wide(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x115F   // Hangul Jamo
        | 0x2E80..=0x303E  // CJK Radicals
        | 0x3041..=0x33FF  // Hiragana/Katakana/CJK
        | 0x3400..=0x4DBF  // CJK Ext A
        | 0x4E00..=0x9FFF  // CJK Unified
        | 0xA000..=0xA4CF  // Yi
        | 0xAC00..=0xD7A3  // Hangul Syllables
        | 0xF900..=0xFAFF  // CJK Compat
        | 0xFE30..=0xFE4F  // CJK Compat Forms
        | 0xFF00..=0xFF60  // Fullwidth
        | 0xFFE0..=0xFFE6
        | 0x20000..=0x2FFFD
        | 0x30000..=0x3FFFD
    )
}
