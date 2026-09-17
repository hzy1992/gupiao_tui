//! 彩色终端输出

use std::io::{self, Write};

use colored::*;

use super::{DisplayOptions, QuoteDisplay};
use crate::model::{Quote, Trend};

/// 终端格式的行情展示器
pub struct TerminalDisplay;

impl QuoteDisplay for TerminalDisplay {
    fn write<W: Write>(&self, w: &mut W, q: &Quote, opts: &DisplayOptions) -> io::Result<()> {
        write_header(w, q)?;
        write_basic(w, q)?;
        write_optionals(w, q)?;
        write_timestamp(w, q)?;

        if opts.show_depth && !q.depth.is_empty() {
            writeln!(w)?;
            write_depth(w, q)?;
        }

        if opts.show_footer {
            writeln!(w)?;
            let now = chrono::Local::now().format("%H:%M:%S").to_string();
            writeln!(w, "  按 Ctrl+C 退出  {}", now.dimmed())?;
        }
        Ok(())
    }
}

fn write_header<W: Write>(w: &mut W, q: &Quote) -> io::Result<()> {
    let title = format!(" {} {}.{} ", q.name, q.code, q.market);
    writeln!(w, "{}", title.on_blue().white().bold())
}

fn write_basic<W: Write>(w: &mut W, q: &Quote) -> io::Result<()> {
    let price_str = format!("{:>8.2}", q.price);
    let change_str_up = format!("+{:.2} ({:+.2}%)", q.change, q.change_pct);
    let change_str_down = format!("{:.2} ({:+.2}%)", q.change, q.change_pct);

    let color_price = match q.trend() {
        Trend::Up => price_str.red().bold().to_string(),
        Trend::Down => price_str.green().bold().to_string(),
        Trend::Flat => price_str.white().bold().to_string(),
    };
    let color_change = match q.trend() {
        Trend::Up => change_str_up.red().bold().to_string(),
        Trend::Down => change_str_down.green().bold().to_string(),
        Trend::Flat => change_str_down.white().to_string(),
    };

    writeln!(w, "  最新价: {}   涨跌: {}", color_price, color_change)?;
    writeln!(w, "  开  盘: {:>8.2}   最  高: {:>8.2}", q.open, q.high)?;
    writeln!(
        w,
        "  昨  收: {:>8.2}   最  低: {:>8.2}",
        q.prev_close, q.low
    )?;
    writeln!(
        w,
        "  成交量: {:>10} 手   成交额: {:>12.2} 万元",
        q.volume, q.amount
    )?;
    Ok(())
}

fn write_optionals<W: Write>(w: &mut W, q: &Quote) -> io::Result<()> {
    if let Some(t) = q.turnover_rate {
        writeln!(w, "  换手率: {:>6.2} %", t)?;
    }
    if let (Some(pe), Some(pb)) = (q.pe, q.pb) {
        writeln!(w, "  市盈率: {:>8.2}   市净率: {:>8.2}", pe, pb)?;
    }
    if let (Some(mc), Some(fc)) = (q.market_cap, q.float_cap) {
        // 字段含义：原 API 单位为亿元
        writeln!(w, "  总市值: {:>8.2} 亿  流通市值: {:>8.2} 亿", mc, fc)?;
    }
    if let Some(a) = q.amplitude {
        writeln!(w, "  振  幅: {:>6.2} %", a)?;
    }
    Ok(())
}

fn write_timestamp<W: Write>(w: &mut W, q: &Quote) -> io::Result<()> {
    let s = q.update_time.format("%Y-%m-%d %H:%M:%S").to_string();
    writeln!(w, "  更新时间: {}", s.dimmed())
}

fn write_depth<W: Write>(w: &mut W, q: &Quote) -> io::Result<()> {
    writeln!(w, "  {}", "五档行情".yellow().bold())?;
    writeln!(
        w,
        "  {:>10} | {:>10}   ||   {:>10} | {:>10}",
        "卖价".bold(),
        "卖量(手)".bold(),
        "买价".bold(),
        "买量(手)".bold()
    )?;
    writeln!(
        w,
        "  {:-<10}-+-{:->10}   ++   {:->10}-+-{:-<10}",
        "", "", "", ""
    )?;

    // 从卖5到卖1，再到买1到买5
    for i in (0..5).rev() {
        let ap = q.depth.ask_prices[i]
            .map(|p| format!("{:.2}", p))
            .unwrap_or_else(|| "-".into());
        let av = q.depth.ask_vols[i]
            .map(|v| format!("{}", v))
            .unwrap_or_else(|| "-".into());
        let bp = q.depth.bid_prices[i]
            .map(|p| format!("{:.2}", p))
            .unwrap_or_else(|| "-".into());
        let bv = q.depth.bid_vols[i]
            .map(|v| format!("{}", v))
            .unwrap_or_else(|| "-".into());

        let ap_colored = if q.depth.ask_prices[i].is_some() {
            ap.green().to_string()
        } else {
            ap.dimmed().to_string()
        };
        let av_colored = if q.depth.ask_vols[i].is_some() {
            av.green().to_string()
        } else {
            av.dimmed().to_string()
        };
        let bp_colored = if q.depth.bid_prices[i].is_some() {
            bp.red().to_string()
        } else {
            bp.dimmed().to_string()
        };
        let bv_colored = if q.depth.bid_vols[i].is_some() {
            bv.red().to_string()
        } else {
            bv.dimmed().to_string()
        };

        writeln!(
            w,
            "  {:>10} | {:>10}   ||   {:>10} | {:>10}",
            ap_colored, av_colored, bp_colored, bv_colored
        )?;
    }
    Ok(())
}
