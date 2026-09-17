//! 回测报告的彩色终端输出

use std::io::{self, Write};

use colored::*;

use crate::analysis::backtest::{BacktestResult, KindStats};
use crate::analysis::SignalKind;

/// 回测展示器
pub struct BacktestDisplay;

impl BacktestDisplay {
    pub fn write<W: Write>(&self, w: &mut W, r: &BacktestResult) -> io::Result<()> {
        write_header(w, r)?;
        write_summary(w, r)?;
        write_by_kind(w, r)?;
        write_by_strength(w, r)?;
        write_samples(w, r)?;
        write_legend(w)?;
        Ok(())
    }
}

fn write_header<W: Write>(w: &mut W, r: &BacktestResult) -> io::Result<()> {
    let title = format!(" {} {}.{} - 回测报告 ", r.code, r.code, r.market);
    writeln!(w, "{}", title.on_blue().white().bold())?;
    writeln!(
        w,
        "  回测区间: {} ~ {}   K 线数: {}   持仓: {} 个交易日   模式: {}",
        r.period_start,
        r.period_end,
        r.total_klines,
        r.params.hold_days,
        if r.params.only_buy_side { "只看多头信号" } else { "全部信号" }
    )?;
    writeln!(w, "  {}", "─".repeat(78).dimmed())?;
    Ok(())
}

fn write_summary<W: Write>(w: &mut W, r: &BacktestResult) -> io::Result<()> {
    writeln!(w, "  {}", "总体表现".yellow().bold())?;

    let win_rate_str = format!("{:.1}%", r.win_rate * 100.0);
    let win_rate_colored = if r.win_rate >= 0.55 {
        win_rate_str.green()
    } else if r.win_rate <= 0.45 {
        win_rate_str.red()
    } else {
        win_rate_str.yellow()
    };

    let avg_str = format!("{:+.2}%", r.avg_return_pct);
    let avg_colored = if r.avg_return_pct >= 0.0 {
        avg_str.green()
    } else {
        avg_str.red()
    };

    let pf_str = if r.profit_factor.is_finite() {
        format!("{:.2}", r.profit_factor)
    } else {
        "∞".to_string()
    };
    let pf_colored = if r.profit_factor >= 1.5 && r.profit_factor.is_finite() {
        pf_str.green()
    } else if r.profit_factor < 1.0 && r.profit_factor.is_finite() {
        pf_str.red()
    } else {
        pf_str.yellow()
    };

    writeln!(
        w,
        "    信号数: {}   多头信号: {}   胜: {}   负: {}",
        r.total_signals,
        r.buy_signals,
        r.win_count,
        r.total_signals - r.win_count
    )?;
    writeln!(
        w,
        "    胜率: {}   平均收益: {}   中位收益: {:+.2}%",
        win_rate_colored, avg_colored, r.median_return_pct
    )?;
    writeln!(
        w,
        "    最大回撤: {:.2}%   Profit Factor: {}",
        r.max_drawdown_pct, pf_colored
    )?;
    writeln!(w)?;
    Ok(())
}

fn write_by_kind<W: Write>(w: &mut W, r: &BacktestResult) -> io::Result<()> {
    if r.by_signal_kind.is_empty() {
        return Ok(());
    }
    writeln!(w, "  {}", "按信号类型".yellow().bold())?;
    writeln!(
        w,
        "    {:<14} {:>6} {:>6} {:>10} {:>14}",
        "信号", "样本", "胜", "胜率", "平均收益"
    )?;
    for kind in [
        SignalKind::StrongBuy,
        SignalKind::Buy,
        SignalKind::Hold,
        SignalKind::Sell,
        SignalKind::StrongSell,
    ] {
        if let Some(s) = r.by_signal_kind.get(&kind) {
            writeln!(w, "    {}", format_kind_row(kind, s))?;
        }
    }
    writeln!(w)?;
    Ok(())
}

fn format_kind_row(kind: SignalKind, s: &KindStats) -> String {
    let win_str = format!("{:>9.1}%", s.win_rate * 100.0);
    let win_colored = if s.win_rate >= 0.55 {
        win_str.green()
    } else if s.win_rate <= 0.45 {
        win_str.red()
    } else {
        win_str.normal()
    };
    let avg_str = format!("{:>+12.2}%", s.avg_return_pct);
    let avg_colored = if s.avg_return_pct >= 0.0 {
        avg_str.green()
    } else {
        avg_str.red()
    };
    format!(
        "{:<14} {:>6} {:>6} {} {}",
        kind.label(),
        s.total,
        s.win,
        win_colored,
        avg_colored
    )
}

fn write_by_strength<W: Write>(w: &mut W, r: &BacktestResult) -> io::Result<()> {
    if r.by_strength.is_empty() {
        return Ok(());
    }
    writeln!(w, "  {}", "按强度分桶".yellow().bold())?;
    writeln!(
        w,
        "    {:<14} {:>6} {:>6} {:>10} {:>14}",
        "强度区间", "样本", "胜", "胜率", "平均收益"
    )?;
    for b in &r.by_strength {
        let win_str = format!("{:>9.1}%", b.win_rate * 100.0);
        let win_colored = if b.win_rate >= 0.55 {
            win_str.green()
        } else if b.win_rate <= 0.45 {
            win_str.red()
        } else {
            win_str.normal()
        };
        let avg_str = format!("{:>+12.2}%", b.avg_return_pct);
        let avg_colored = if b.avg_return_pct >= 0.0 {
            avg_str.green()
        } else {
            avg_str.red()
        };
        writeln!(
            w,
            "    {:<14} {:>6} {:>6} {} {}",
            format!("{}-{}", b.min, b.max),
            b.total,
            b.win,
            win_colored,
            avg_colored
        )?;
    }
    writeln!(w)?;
    Ok(())
}

fn write_samples<W: Write>(w: &mut W, r: &BacktestResult) -> io::Result<()> {
    if r.sample_trades.is_empty() {
        return Ok(());
    }
    writeln!(w, "  {}", "抽样交易 (按时间均匀抽样)".yellow().bold())?;
    writeln!(
        w,
        "    {:<12} {:<8} {:>5} {:>10} {:>10} {:>10} {:>10}",
        "信号日", "信号", "强度", "买入价", "卖出价", "卖出日", "收益"
    )?;
    for t in &r.sample_trades {
        let ret_str = format!("{:>+9.2}%", t.return_pct);
        let ret_colored = if t.return_pct >= 0.0 {
            ret_str.green()
        } else {
            ret_str.red()
        };
        writeln!(
            w,
            "    {:<12} {:<8} {:>5} {:>10.2} {:>10.2} {:>10} {}",
            t.signal_date,
            t.kind.label(),
            t.strength,
            t.entry_price,
            t.exit_price,
            t.exit_date,
            ret_colored
        )?;
    }
    writeln!(w)?;
    Ok(())
}

fn write_legend<W: Write>(w: &mut W) -> io::Result<()> {
    writeln!(w, "  {}", "─".repeat(78).dimmed())?;
    writeln!(
        w,
        "  {} 回测仅基于价格+成交量维度（无五档盘口/基本面数据），不构成投资建议。",
        "⚠".yellow()
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::backtest::{BacktestParams, BacktestResult, StrengthBucket, Trade};
    use chrono::NaiveDate;
    use std::collections::HashMap;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn sample_result() -> BacktestResult {
        let mut by_kind = HashMap::new();
        by_kind.insert(
            SignalKind::Buy,
            KindStats { total: 10, win: 6, win_rate: 0.6, avg_return_pct: 1.2 },
        );
        by_kind.insert(
            SignalKind::Hold,
            KindStats { total: 20, win: 11, win_rate: 0.55, avg_return_pct: 0.3 },
        );
        let by_strength = vec![
            StrengthBucket { min: 0, max: 29, total: 5, win: 2, win_rate: 0.4, avg_return_pct: -0.5 },
            StrengthBucket { min: 30, max: 59, total: 8, win: 5, win_rate: 0.625, avg_return_pct: 1.5 },
        ];
        let sample_trades = vec![
            Trade {
                signal_date: d("2026-09-01"),
                kind: SignalKind::Buy,
                strength: 45,
                entry_price: 100.0,
                exit_price: 103.0,
                exit_date: d("2026-09-06"),
                return_pct: 3.0,
                stopped_loss: false,
                took_profit: false,
            },
            Trade {
                signal_date: d("2026-09-02"),
                kind: SignalKind::Buy,
                strength: 50,
                entry_price: 102.0,
                exit_price: 100.0,
                exit_date: d("2026-09-07"),
                return_pct: -1.96,
                stopped_loss: false,
                took_profit: false,
            },
        ];
        BacktestResult {
            code: "600519".into(),
            market: "SH".into(),
            period_start: d("2026-09-01"),
            period_end: d("2026-09-30"),
            total_klines: 22,
            total_signals: 17,
            buy_signals: 10,
            win_count: 10,
            win_rate: 10.0 / 17.0,
            avg_return_pct: 0.8,
            median_return_pct: 0.5,
            max_drawdown_pct: 5.5,
            profit_factor: 1.8,
            by_signal_kind: by_kind,
            by_strength,
            sample_trades,
            params: BacktestParams::default(),
            stop_loss_count: 0,
            take_profit_count: 0,
            avg_stop_loss_pct: 0.0,
            avg_take_profit_pct: 0.0,
        }
    }

    #[test]
    fn test_display_writes_all_sections() {
        let r = sample_result();
        let mut buf = Vec::new();
        BacktestDisplay.write(&mut buf, &r).unwrap();
        let s = String::from_utf8(buf).unwrap();
        // 标题
        assert!(s.contains("回测报告"));
        // 总体
        assert!(s.contains("总体表现"));
        assert!(s.contains("胜率"));
        assert!(s.contains("Profit Factor"));
        // 按信号类型
        assert!(s.contains("按信号类型"));
        assert!(s.contains("买入"));
        assert!(s.contains("观望"));
        // 按强度
        assert!(s.contains("按强度分桶"));
        assert!(s.contains("0-29"));
        // 抽样
        assert!(s.contains("抽样交易"));
        // 免责声明
        assert!(s.contains("不构成投资建议"));
    }

    #[test]
    fn test_display_handles_empty_by_kind() {
        let mut r = sample_result();
        r.by_signal_kind.clear();
        r.by_strength.clear();
        r.sample_trades.clear();
        let mut buf = Vec::new();
        BacktestDisplay.write(&mut buf, &r).unwrap();
        let s = String::from_utf8(buf).unwrap();
        // 没样本时不要崩
        assert!(s.contains("回测报告"));
    }
}
