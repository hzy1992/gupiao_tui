//! 低调、键盘驱动的终端界面。
//!
//! ## 布局
//!
//! ```text
//!   GUPIAO / 自选股
//!   自动刷新：5 秒 · ↑/↓ 切换 · r 刷新 · Enter 详情 · Tab 翻页 · m 切融合 · M 切模型 · q 退出
//!   ────────────────────────────────────────────────────────────────────────────
//! > 名称       代码       现价      涨跌%      信号     强度  更新时间
//!   贵州茅台   600519.SH   1316.01   -1.05%     观望     42%   16:14:58
//!   ...
//!   ────────────────────────────────────────────────────────────────────────────
//!   a 添加  d 删除  r 刷新  Enter 详情  Tab 翻页  m 切融合  M 切模型  q 退出  模型=RF5#3
//!
//!   ┌─ 600519.SH 贵州茅台 ─ 分析详情 [概览] ──────────────────────────────────┐
//!   │ 操作建议: 观望  强度 42%   关键因子: 涨跌幅 | 委比                       │
//!   │ 模型置信:  67.5%  融合: weighted  规则=+12.3  最终=+39.9                 │
//!   │ 支撑 1310.00 (-0.46%)         压力 1325.00 (+0.68%)                       │
//!   │ 现价 1316.01  涨跌幅 -1.05%                                             │
//!   └──────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## 模型与融合策略
//!
//! - 默认加载数据库中所有训练好的模型（按 trained_at 倒序），自动选中最新一个
//! - `m`：循环切换融合策略 rules → model → weighted
//! - `M`：循环切换到下一个模型
//! - `x`：关闭模型（回到规则模式）
//! - 底部状态栏实时显示当前模型 / 融合策略 / 是否启用

use std::io::{self, Write};
use std::time::Duration;

use anyhow::Context;
use colored::Colorize;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use futures::future::join_all;

use crate::analysis::{
    analyze_with_model, AnalysisReport, FusionStrategy, InferenceContext, SignalKind,
};
use crate::model::ml::LoadedModel;
use crate::model::{Favorite, Symbol};
use crate::provider::{EastMoneyNewsProvider, KlineProvider, QuoteProvider, SinaKlineProvider, TencentProvider};
use crate::storage::{Database, FavoriteStore, KlineStore, ModelStore};

const REFRESH_SECONDS: u64 = 5;
/// TUI 表格宽度（不含左侧 marker）
const TABLE_WIDTH: usize = 86;
/// detail 面板最大行数（避免在矮终端里溢出）
const DETAIL_MAX_LINES: usize = 10;

/// 详情面板的三页：概览 / 指标 / 建议
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DetailPage {
    Overview,
    Indicators,
    Advice,
    News,
}

impl DetailPage {
    fn next(self) -> Self {
        match self {
            DetailPage::Overview => DetailPage::Indicators,
            DetailPage::Indicators => DetailPage::Advice,
            DetailPage::Advice => DetailPage::News,
            DetailPage::News => DetailPage::Overview,
        }
    }

    fn label(self) -> &'static str {
        match self {
            DetailPage::Overview => "概览",
            DetailPage::Indicators => "指标",
            DetailPage::Advice => "建议",
            DetailPage::News => "资讯",
        }
    }
}

/// TUI 的模型 / 融合策略运行时状态
#[derive(Debug, Clone)]
struct TuiState {
    /// 当前融合策略（用户可用 `m` 切换）
    fusion: FusionStrategy,
    /// 融合权重 α
    alpha: f64,
    /// 全部已训练模型（按 trained_at 倒序）
    models: Vec<LoadedModel>,
    /// 当前选中的模型在 `models` 里的下标（None 表示规则模式）
    model_idx: Option<usize>,
    /// 当前 InferenceContext（每次 fusion / model 变化时重建）
    ctx: InferenceContext,
}

impl TuiState {
    fn new(fusion: FusionStrategy, alpha: f64, models: Vec<LoadedModel>) -> Self {
        let mut s = Self {
            fusion,
            alpha,
            models,
            model_idx: None,
            ctx: InferenceContext::rules_only(),
        };
        // 默认选最新模型
        if !s.models.is_empty() {
            s.model_idx = Some(0);
        }
        s.rebuild_ctx();
        s
    }

    /// 根据当前 fusion / model_idx 重建 ctx
    fn rebuild_ctx(&mut self) {
        self.ctx = if self.fusion == FusionStrategy::RulesOnly || self.model_idx.is_none() {
            InferenceContext::rules_only()
        } else if let Some(idx) = self.model_idx {
            if let Some(m) = self.models.get(idx) {
                InferenceContext::with_model(m.clone(), self.fusion, self.alpha)
            } else {
                InferenceContext::rules_only()
            }
        } else {
            InferenceContext::rules_only()
        };
    }

    /// 切换融合策略（rules → model → weighted → rules）
    fn cycle_fusion(&mut self) {
        self.fusion = match self.fusion {
            FusionStrategy::RulesOnly => FusionStrategy::ModelOnly,
            FusionStrategy::ModelOnly => FusionStrategy::Weighted,
            FusionStrategy::Weighted => FusionStrategy::RulesOnly,
        };
        self.rebuild_ctx();
    }

    /// 切换到下一个模型（循环）
    fn cycle_model(&mut self) {
        if self.models.is_empty() {
            return;
        }
        // 当前可能为 None -> 跳到 0
        let next = match self.model_idx {
            None => 0,
            Some(i) => (i + 1) % self.models.len(),
        };
        self.model_idx = Some(next);
        self.rebuild_ctx();
    }

    /// 关闭模型（回到规则模式）
    fn clear_model(&mut self) {
        self.model_idx = None;
        self.rebuild_ctx();
    }

    fn current_model_label(&self) -> String {
        match self.model_idx {
            None => "(无模型)".to_string(),
            Some(i) => match self.models.get(i) {
                Some(m) => format!(
                    "{}{}#{}(cv={:.1}%)",
                    m.kind.label(),
                    m.hold_days,
                    m.id,
                    m.report.cv_mean * 100.0
                ),
                None => "(无模型)".to_string(),
            },
        }
    }
}

/// 加载默认模型 + 可用模型列表（按 trained_at 倒序）
fn load_models(db: &Database) -> Vec<LoadedModel> {
    let store = ModelStore::new(db);
    let records = match store.list() {
        Ok(rs) => rs,
        Err(_) => return Vec::new(),
    };
    records
        .into_iter()
        .filter_map(|r| store.load(r.id).ok().flatten())
        .collect()
}

/// 给某只股票加载 K 线（用缓存，不足时尝试新浪拉取）
async fn load_klines(db: &Database, code: &str, market: crate::model::Market) -> Option<crate::model::KlineSeries> {
    let store = KlineStore::new(db);
    let cached = store.count(code, market).ok()?;
    if cached >= 60 {
        return store.load_range(code, market, None, None).ok();
    }
    // 缓存不足：尝试异步拉取
    let symbol = Symbol::with_market(code, Some(market)).ok()?;
    let provider = SinaKlineProvider::new();
    let series = provider.fetch(&symbol, 250).await.ok()?;
    let _ = store.upsert_series(code, market, &series);
    Some(series)
}

/// 启动终端 GUI：方向键选择，后台定时刷新行情。
pub async fn run() -> anyhow::Result<()> {
    let db = Database::open().context("打开自选股数据库失败")?;
    let provider = TencentProvider::new();
    let models = load_models(&db);
    let mut state = TuiState::new(FusionStrategy::default(), 0.5, models);
    if !state.models.is_empty() {
        eprintln!(
            "{} 已加载 {} 个模型，当前: {}",
            "✓".green(),
            state.models.len(),
            state.current_model_label()
        );
    } else {
        eprintln!(
            "{} 未找到训练模型，使用纯规则模式（先跑 `gupiao train`）",
            "⚠".yellow()
        );
    }
    let mut selected = 0usize;
    let mut needs_refresh = true;
    let mut next_refresh = tokio::time::Instant::now();
    let mut detail_expanded = false;
    let mut detail_page = DetailPage::Overview;

    enable_raw_mode().context("启用终端键盘模式失败")?;
    print!("\x1b[?25l");
    io::stdout().flush()?;
    let result = loop {
        let favorites = FavoriteStore::new(&db).list()?;
        selected = selected.min(favorites.len().saturating_sub(1));
        if needs_refresh || tokio::time::Instant::now() >= next_refresh {
            let reports = fetch_favorites(&provider, &db, &favorites, &state).await;
            draw(
                &favorites,
                &reports,
                &state,
                selected,
                detail_expanded,
                detail_page,
            );
            next_refresh = tokio::time::Instant::now() + Duration::from_secs(REFRESH_SECONDS);
            needs_refresh = false;
        } else if !detail_expanded {
            draw_status(selected, favorites.len(), next_refresh, &state);
        }

        if event::poll(Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                // crossterm 0.28 在 Windows 上每个按键都会产生 press + release
                // 两个事件；只处理 press，避免方向键 selected 被双倍调整、
                // 以及 release 抢占掉下一次 poll 的 press 导致按键"失灵"。
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key {
                    KeyEvent {
                        code: KeyCode::Char('q'),
                        ..
                    }
                    | KeyEvent {
                        code: KeyCode::Esc, ..
                    } => break Ok(()),
                    KeyEvent {
                        code: KeyCode::Char('c'),
                        modifiers,
                        ..
                    } if modifiers.contains(KeyModifiers::CONTROL) => break Ok(()),
                    KeyEvent {
                        code: KeyCode::Up, ..
                    } => {
                        if !detail_expanded && selected > 0 {
                            selected -= 1;
                            needs_refresh = true;
                        }
                    }
                    KeyEvent {
                        code: KeyCode::Down,
                        ..
                    } => {
                        if !detail_expanded
                            && selected + 1 < favorites.len()
                        {
                            selected += 1;
                            needs_refresh = true;
                        }
                    }
                    KeyEvent {
                        code: KeyCode::Char('r'),
                        ..
                    } => needs_refresh = true,
                    KeyEvent {
                        code: KeyCode::Char('a'),
                        ..
                    } => {
                        add_favorite(&db)?;
                        needs_refresh = true;
                    }
                    KeyEvent {
                        code: KeyCode::Char('d'),
                        ..
                    } if !favorites.is_empty() => {
                        let item = &favorites[selected];
                        FavoriteStore::new(&db).remove(&item.code, item.market)?;
                        selected = selected.saturating_sub(1);
                        needs_refresh = true;
                    }
                    KeyEvent {
                        code: KeyCode::Enter,
                        ..
                    } => {
                        if !favorites.is_empty() {
                            detail_expanded = !detail_expanded;
                            detail_page = DetailPage::Overview;
                            needs_refresh = true;
                        }
                    }
                    KeyEvent {
                        code: KeyCode::Tab, ..
                    } if detail_expanded => {
                        detail_page = detail_page.next();
                        needs_refresh = true;
                    }
                    KeyEvent {
                        code: KeyCode::Char('m'), ..
                    } => {
                        state.cycle_fusion();
                        needs_refresh = true;
                    }
                    KeyEvent {
                        code: KeyCode::Char('M'), ..
                    } => {
                        state.cycle_model();
                        needs_refresh = true;
                    }
                    KeyEvent {
                        code: KeyCode::Char('x'), ..
                    } => {
                        state.clear_model();
                        needs_refresh = true;
                    }
                    _ => {}
                }
            }
        }
    };
    let _ = disable_raw_mode();
    print!("\x1b[?25h\x1b[0m\n");
    result
}

async fn fetch_favorites<P: QuoteProvider>(
    provider: &P,
    db: &Database,
    list: &[Favorite],
    state: &TuiState,
) -> Vec<Option<AnalysisReport>> {
    let news_provider = EastMoneyNewsProvider::new();
    join_all(list.iter().map(|fav| {
        let news_provider = &news_provider;
        async move {
            let symbol = Symbol::with_market(&fav.code, Some(fav.market)).ok()?;
            let quote = provider.fetch(&symbol).await.ok()?;
            let weights = crate::analysis::ScoreWeights::default();
            let thresholds = crate::analysis::ScoreThresholds::default();
            // 并行获取新闻（不阻塞分析）
            let news = news_provider.fetch_all(&symbol).await.ok();
            // 有模型且需要 K 线：尝试加载 K 线
            let report = if state.ctx.uses_model() {
                let series = load_klines(db, &fav.code, fav.market).await;
                match series {
                    Some(s) => analyze_with_model(&quote, &weights, &thresholds, &s, &state.ctx),
                    None => crate::analysis::analyze_with(&quote, &weights, &thresholds),
                }
            } else {
                crate::analysis::analyze_with(&quote, &weights, &thresholds)
            };
            // 附上新闻
            let report = if let Some(news) = news {
                report.with_news(news)
            } else {
                report
            };
            Some(report)
        }
    }))
    .await
}

/// 在表格底部的按键提示行下方刷新一次"X 秒后自动刷新"倒计时，
/// 避免覆盖选中行或 detail 面板。
fn draw_status(
    selected: usize,
    favorites_len: usize,
    next_refresh: tokio::time::Instant,
    state: &TuiState,
) {
    let left = next_refresh
        .saturating_duration_since(tokio::time::Instant::now())
        .as_secs()
        + 1;
    let hint = format!(
        "第 {} / {} 项 · {} 秒后自动刷新 · ↑/↓ 切换",
        selected + 1,
        favorites_len,
        left
    );
    // 模型状态行
    let model_status = format!(
        "模型: {} · 融合: {} · {}",
        state.current_model_label(),
        state.fusion.label(),
        if state.ctx.uses_model() { "启用" } else { "关闭" }
    );
    // 布局：1 标题 + 1 副标题 + 1 上分隔线 + 1 表头 + N 数据行 + 1 下分隔线 + 1 按键提示
    // 状态行放在按键提示的下一行（7 + N）。
    let row = 7 + favorites_len;
    print!("\x1b[{};1H\x1b[2K  {}", row, hint.dimmed());
    let row2 = 7 + favorites_len + 1;
    print!("\x1b[{};1H\x1b[2K  {}", row2, model_status.cyan());
    let _ = io::stdout().flush();
}

fn draw(
    favorites: &[Favorite],
    reports: &[Option<AnalysisReport>],
    state: &TuiState,
    selected: usize,
    detail_expanded: bool,
    detail_page: DetailPage,
) {
    print!("\x1b[2J\x1b[H");
    println!("  {}", "GUPIAO / Favorites".bold());
    let model_subtitle = format!(
        "自动刷新：{} 秒 · ↑/↓ 切换 · r 刷新 · Enter 详情 · Tab 翻页 · m 切融合 · M 切模型 · q 退出",
        REFRESH_SECONDS
    );
    println!("  {}", model_subtitle.dimmed());
    println!("  {}", "─".repeat(TABLE_WIDTH).dimmed());

    // 表头
    println!(
        "  {} {} {}  {}  {} {} {}  {}",
        format!("{:<10}", "名称").dimmed(),
        format!("{:<10}", "代码").dimmed(),
        format!("{:>8}", "现价").dimmed(),
        format!("{:>6}", "涨跌%").dimmed(),
        format!("{:<4}", "信号").dimmed(),
        format!("{:<4}", "强度").dimmed(),
        format!("{:<5}", "模型").dimmed(),
        "更新时间".dimmed(),
    );
    if favorites.is_empty() {
        println!("  {}", "暂无自选股，请按 a 添加".dimmed());
    }
    for (index, favorite) in favorites.iter().enumerate() {
        let marker = if index == selected { ">" } else { " " };
        match reports.get(index).and_then(|r| r.as_ref()) {
            Some(r) => {
                let model_cell = format_model_prob(r.model_prob);
                println!(
                    "{} {} {} {:>10.2} {:>+8.2}%  {} {:>4}%  {} {}",
                    marker,
                    format!("{:<10}", truncate_cn(&r.symbol_name, 10)),
                    format!("{:<10}", format!("{}.{}", r.symbol_code, r.market)),
                    r.current_price,
                    r.change_pct,
                    format!("{:<8}", signal_label_colored(r.signal.kind)),
                    r.signal.strength,
                    model_cell,
                    "-",
                );
            }
            None => println!(
                "{} {} {} {}",
                marker,
                format!("{:<10}", favorite.code),
                format!("{:<10}", favorite.market),
                "获取失败".dimmed()
            ),
        }
    }
    println!("  {}", "─".repeat(TABLE_WIDTH).dimmed());
    let model_hint = format!(
        "a 添加  d 删除  r 刷新  Enter 详情  Tab 翻页  m 切融合  M 切模型  q 退出  模型={}",
        state.current_model_label()
    );
    println!("  {}", model_hint.dimmed());

    // detail 面板
    if detail_expanded {
        if let Some(report) = reports.get(selected).and_then(|r| r.as_ref()) {
            render_detail_panel(report, detail_page);
        } else if favorites.is_empty() {
            println!();
            println!("  {}", "暂无自选股，按 a 添加".dimmed());
        } else {
            println!();
            println!(
                "  {}",
                "当前选中股票暂无行情，无法显示分析".dimmed()
            );
        }
    }
    let _ = io::stdout().flush();
}

/// 在终端底部渲染选中股票的 detail 面板
fn render_detail_panel(report: &AnalysisReport, page: DetailPage) {
    let mut stdout = io::stdout();
    write_detail_panel(&mut stdout, report, page).expect("write detail panel");
}

/// 把 detail 面板写入任意 `Write`，便于单测断言内容
fn write_detail_panel<W: Write>(
    w: &mut W,
    report: &AnalysisReport,
    page: DetailPage,
) -> io::Result<()> {
    writeln!(w)?;
    let title = format!(
        " {} {}.{} - 分析详情 [{}] ",
        report.symbol_name, report.symbol_code, report.market, page.label()
    );
    let title_w = display_width(&title);
    let border_w = TABLE_WIDTH.saturating_sub(title_w + 2);
    writeln!(w, "  ┌─{}{}┐", title.bold(), "─".repeat(border_w).dimmed())?;

    match page {
        DetailPage::Overview => write_overview(w, report)?,
        DetailPage::Indicators => write_indicators(w, report)?,
        DetailPage::Advice => write_advice(w, report)?,
        DetailPage::News => write_news(w, report)?,
    }

    writeln!(w, "  └{}┘", "─".repeat(TABLE_WIDTH - 2).dimmed())?;
    Ok(())
}

fn write_overview<W: Write>(w: &mut W, r: &AnalysisReport) -> io::Result<()> {
    let label = format!(" {} ", r.signal.kind.label());
    let signal_colored = match r.signal.kind {
        SignalKind::StrongBuy => label.on_red().white().bold().to_string(),
        SignalKind::Buy => label.red().bold().to_string(),
        SignalKind::Hold => label.yellow().bold().to_string(),
        SignalKind::Sell => label.green().bold().to_string(),
        SignalKind::StrongSell => label.on_green().white().bold().to_string(),
    };
    let factors = if r.key_factors.is_empty() {
        "-".to_string()
    } else {
        r.key_factors.join(" | ")
    };
    let line1 = format!(
        "操作建议: {}  强度 {}%   关键因子: {}",
        signal_colored,
        r.signal.strength,
        factors.cyan()
    );
    writeln!(w, "  │{}│", pad_cn_ansi(&line1, TABLE_WIDTH - 4))?;

    // 模型行（M3）
    if let Some(prob) = r.model_prob {
        let prob_pct = prob * 100.0;
        let prob_colored = if prob >= 0.6 {
            format!("{:>5.1}%", prob_pct).red().bold().to_string()
        } else if prob <= 0.4 {
            format!("{:>5.1}%", prob_pct).green().bold().to_string()
        } else {
            format!("{:>5.1}%", prob_pct).yellow().to_string()
        };
        let strat = r
            .fusion_strategy
            .map(|s| s.label())
            .unwrap_or("weighted");
        let model_line = format!(
            "模型置信: {}  融合: {}  规则={:+.1}  最终={:+.1}",
            prob_colored,
            strat.dimmed(),
            r.rule_score,
            r.final_score
        );
        writeln!(w, "  │{}│", pad_cn_ansi(&model_line, TABLE_WIDTH - 4))?;
    }

    // 支撑 / 压力
    let support_line = match r.support {
        Some(s) => {
            let dist = (r.current_price - s.price) / r.current_price * 100.0;
            format!(
                "支撑 {:>8.2} (距现价 {:>+5.2}%, {})",
                s.price,
                -dist,
                s.source.label()
            )
        }
        None => "支撑 (无足够数据)".to_string(),
    };
    let resistance_line = match r.resistance {
        Some(s) => {
            let up = (s.price - r.current_price) / r.current_price * 100.0;
            format!(
                "压力 {:>8.2} (距现价 {:>+5.2}%, {})",
                s.price,
                up,
                s.source.label()
            )
        }
        None => "压力 (无足够数据)".to_string(),
    };
    let half = (TABLE_WIDTH - 4) / 2;
    writeln!(
        w,
        "  │{}{}│",
        pad_cn_ansi(&support_line, half),
        pad_cn("", TABLE_WIDTH - 4 - half)
    )?;
    writeln!(
        w,
        "  │{}│",
        pad_cn_ansi(&resistance_line, TABLE_WIDTH - 4)
    )?;

    // 摘要
    let summary = format!(
        "现价 {:>8.2}  涨跌幅 {:>+6.2}%    (Tab 翻页: 指标 / 建议)",
        r.current_price, r.change_pct
    );
    writeln!(
        w,
        "  │{}│",
        pad_cn_ansi(&summary.dimmed().to_string(), TABLE_WIDTH - 4)
    )?;
    Ok(())
}

fn write_indicators<W: Write>(w: &mut W, r: &AnalysisReport) -> io::Result<()> {
    let ind = &r.indicators;
    let l1 = format!(
        "位置 {:>5.1}%   委比 {:>+6.1}%   量比 {:>5.2}   价量 {:>8.0}",
        ind.position * 100.0,
        ind.committee_ratio * 100.0,
        ind.volume_ratio,
        ind.pv_ratio
    );
    writeln!(w, "  │{}│", pad_cn(&l1, TABLE_WIDTH - 4))?;
    let l2 = format!(
        "趋势强度 {:>+5.2}   振幅 {}",
        ind.trend_strength,
        ind.amplitude
            .map(|a| format!("{:.2}%", a))
            .unwrap_or_else(|| "-".into())
    );
    writeln!(w, "  │{}│", pad_cn(&l2, TABLE_WIDTH - 4))?;
    let hint = "(Tab 翻页: 概览 / 建议)";
    writeln!(w, "  │{}│", pad_cn_ansi(&hint.dimmed().to_string(), TABLE_WIDTH - 4))?;
    Ok(())
}
// write_advice function
fn write_advice<W: Write>(w: &mut W, r: &AnalysisReport) -> io::Result<()> {
    let mut printed = 0usize;
    let max = DETAIL_MAX_LINES;
    for (i, line) in r.advice.iter().enumerate() {
        if printed >= max {
            break;
        }
        let colored = if i == 0 {
            line.bright_white().to_string()
        } else if line.starts_with("⚠") {
            line.red().to_string()
        } else if line.starts_with("  ") {
            line.dimmed().to_string()
        } else {
            line.to_string()
        };
        writeln!(w, "  │{}│", pad_cn_ansi(&colored, TABLE_WIDTH - 4))?;
        printed += 1;
    }
    if r.advice.len() > printed {
        let more = format!("...还有 {} 条建议", r.advice.len() - printed);
        writeln!(w, "  │{}│", pad_cn_ansi(&more.dimmed().to_string(), TABLE_WIDTH - 4))?;
    }
    let hint = "(Tab 翻页: 概览 / 指标 / 建议 / 资讯)";
    writeln!(w, "  │{}│", pad_cn_ansi(&hint.dimmed().to_string(), TABLE_WIDTH - 4))?;
    Ok(())
}

// write_news function
fn write_news<W: Write>(w: &mut W, r: &AnalysisReport) -> io::Result<()> {
    if let Some(ref news) = r.news {
        if !news.announcements.is_empty() {
            writeln!(w, "  │{}│", pad_cn_ansi(&"  公司公告".yellow().bold().to_string(), TABLE_WIDTH - 4))?;
            for ann in news.announcements.iter().take(3) {
                let line = format!("  │  [{}] {}│", ann.date, truncate_cn(&ann.title, TABLE_WIDTH - 12));
                writeln!(w, "{}", pad_cn_ansi(&line.dimmed().to_string(), TABLE_WIDTH))?;
            }
            if news.announcements.len() > 3 {
                writeln!(w, "  │{}│", pad_cn_ansi(&format!("  ...还有 {} 条公告", news.announcements.len() - 3).dimmed(), TABLE_WIDTH - 4))?;
            }
            writeln!(w)?;
        }
        if !news.news.is_empty() {
            writeln!(w, "  │{}│", pad_cn_ansi(&"  最新资讯/股评".cyan().bold().to_string(), TABLE_WIDTH - 4))?;
            for item in news.news.iter().take(3) {
                let time = item.datetime.format("%H:%M").to_string();
                let line = format!("  │  [{}] {}│", time, truncate_cn(&item.title, TABLE_WIDTH - 14));
                writeln!(w, "{}", pad_cn_ansi(&line.dimmed().to_string(), TABLE_WIDTH))?;
            }
            if news.news.len() > 3 {
                writeln!(w, "  │{}│", pad_cn_ansi(&format!("  ...还有 {} 条资讯", news.news.len() - 3).dimmed(), TABLE_WIDTH - 4))?;
            }
        }
        if news.is_empty() {
            writeln!(w, "  │{}│", pad_cn_ansi(&"  暂无资讯或公告".dimmed().to_string(), TABLE_WIDTH - 4))?;
        }
    } else {
        writeln!(w, "  │{}│", pad_cn_ansi(&"  资讯加载中（请等待网络）".dimmed().to_string(), TABLE_WIDTH - 4))?;
    }
    let hint = "(Tab 翻页: 概览/指标/建议/资讯)";
    writeln!(w, "  │{}│", pad_cn_ansi(&hint.dimmed().to_string(), TABLE_WIDTH - 4))?;
    Ok(())
}

fn signal_label_colored(kind: SignalKind) -> String {
    let s = kind.short();
    match kind {
        SignalKind::StrongBuy => s.red().bold().to_string(),
        SignalKind::Buy => s.red().to_string(),
        SignalKind::Hold => s.yellow().to_string(),
        SignalKind::Sell => s.green().to_string(),
        SignalKind::StrongSell => s.green().bold().to_string(),
    }
}

/// 把 model_prob 格式化成主表格里那列的字符串
///
/// - None：表示纯规则模式 -> "-"
/// - Some(p)：按概率上色（>60% 红、<40% 绿、中间黄）
fn format_model_prob(p: Option<f64>) -> String {
    match p {
        None => "-".dimmed().to_string(),
        Some(v) => {
            let pct = v * 100.0;
            let s = format!("{:>4.0}%", pct);
            if v >= 0.6 {
                s.red().bold().to_string()
            } else if v <= 0.4 {
                s.green().bold().to_string()
            } else {
                s.yellow().to_string()
            }
        }
    }
}

fn add_favorite(db: &Database) -> anyhow::Result<()> {
    disable_raw_mode()?;
    print!("\n添加股票代码: ");
    io::stdout().flush()?;
    let mut code = String::new();
    io::stdin().read_line(&mut code)?;
    enable_raw_mode()?;
    let symbol = Symbol::new(code.trim()).context("股票代码无效")?;
    FavoriteStore::new(db).add(&Favorite::new(symbol.code, symbol.market, None))?;
    Ok(())
}

// ---------- 纯函数工具（可单测） ----------

/// 按显示宽度截断字符串（用于名称过长时），不足时右补空格
fn truncate_cn(s: &str, max_width: usize) -> String {
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = if is_wide(c) { 2 } else { 1 };
        if w + cw > max_width {
            break;
        }
        out.push(c);
        w += cw;
    }
    if w < max_width {
        out.push_str(&" ".repeat(max_width - w));
    }
    out
}

/// 中文按显示宽度右补空格（不处理 ANSI；用于纯 ASCII 行）
fn pad_cn(s: &str, target: usize) -> String {
    let w = display_width(s);
    if w >= target {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(target - w))
    }
}

/// ANSI 颜色字符串按显示宽度右补空格
///
/// 简化处理：先去掉 ESC[...m 序列估算可见宽度，再补空格。
fn pad_cn_ansi(s: &str, target: usize) -> String {
    let mut stripped = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                while let Some(&nc) = chars.peek() {
                    chars.next();
                    if nc.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            stripped.push(c);
        }
    }
    let w = display_width(&stripped);
    if w >= target {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(target - w))
    }
}

/// 粗略的「显示宽度」：CJK 字符算 2，其它算 1
fn display_width(s: &str) -> usize {
    s.chars().map(|c| if is_wide(c) { 2 } else { 1 }).sum()
}

fn is_wide(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x115F
        | 0x2E80..=0x303E
        | 0x3041..=0x33FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xA000..=0xA4CF
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE4F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0x20000..=0x2FFFD
        | 0x30000..=0x3FFFD
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{Indicators, PriceLevel, PriceLevelSource, Signal, SignalKind};

    fn mk_report(signal_kind: SignalKind, name: &str) -> AnalysisReport {
        AnalysisReport {
            symbol_code: "600519".into(),
            symbol_name: name.into(),
            market: "SH".into(),
            current_price: 100.0,
            change_pct: 1.5,
            indicators: Indicators {
                position: 0.5,
                committee_ratio: 0.2,
                volume_ratio: 1.2,
                pv_ratio: 1.5e8,
                trend_strength: 0.3,
                amplitude: Some(3.5),
            },
            signal: Signal {
                kind: signal_kind,
                strength: 42,
            },
            support: Some(PriceLevel {
                price: 95.0,
                source: PriceLevelSource::DayLow,
            }),
            resistance: Some(PriceLevel {
                price: 108.0,
                source: PriceLevelSource::DayHigh,
            }),
            advice: vec![
                "综合得分: +12.3 / 100".into(),
                "委比正向，买盘强于卖盘".into(),
                "⚠ 风险提示: 仅供参考".into(),
            ],
            key_factors: vec!["涨跌幅".into(), "委比".into()],
            model_prob: None,
            fusion_strategy: None,
            rule_score: 12.3,
            final_score: 12.3,
            news: None,
        }
    }

    #[test]
    fn test_truncate_cn_ascii() {
        assert_eq!(truncate_cn("abcdef", 10), "abcdef    ");
        assert_eq!(truncate_cn("abcdef", 3), "abc");
    }

    #[test]
    fn test_truncate_cn_wide() {
        // "贵州茅台" 每个 2 宽，max_width=5 只能放 "贵"（2）+ "州"（2）= 4，补 1 空格
        assert_eq!(truncate_cn("贵州茅台", 5), "贵州 ");
    }

    #[test]
    fn test_pad_cn_to_width() {
        let s = pad_cn("hi", 6);
        assert_eq!(s, "hi    ");
        let s = pad_cn("中文", 6);
        // "中文" 显示宽度 4，补 2 空格
        assert_eq!(s, "中文  ");
    }

    #[test]
    fn test_display_width_mixed() {
        assert_eq!(display_width("abc"), 3);
        assert_eq!(display_width("中文"), 4);
        assert_eq!(display_width("a中b"), 4);
    }

    #[test]
    fn test_pad_cn_ansi_strips_esc() {
        let colored = "\x1b[31m红涨\x1b[0m";
        // 可见内容 "红涨" = 4 宽，target=8 应补 4 空格
        let padded = pad_cn_ansi(colored, 8);
        assert!(padded.starts_with(colored));
        assert!(padded.ends_with("    "));
    }

    #[test]
    fn test_detail_page_next_cycles() {
        assert_eq!(DetailPage::Overview.next(), DetailPage::Indicators);
        assert_eq!(DetailPage::Indicators.next(), DetailPage::Advice);
        assert_eq!(DetailPage::Advice.next(), DetailPage::News);
        assert_eq!(DetailPage::News.next(), DetailPage::Overview);
    }

    #[test]
    fn test_signal_label_colored_all_variants() {
        for k in [
            SignalKind::StrongBuy,
            SignalKind::Buy,
            SignalKind::Hold,
            SignalKind::Sell,
            SignalKind::StrongSell,
        ] {
            let s = signal_label_colored(k);
            assert!(!s.is_empty(), "label should not be empty for {:?}", k);
            assert!(s.contains(k.short()), "should contain short label {:?}", k);
        }
    }

    #[test]
    fn test_render_overview_doesnt_panic() {
        let r = mk_report(SignalKind::Buy, "贵州茅台");
        let mut buf = Vec::new();
        write_overview(&mut buf, &r).unwrap();
        let s = strip_ansi(&String::from_utf8(buf).unwrap());
        // 注意：股票名/代码在 write_detail_panel 的标题里，write_overview 本身不写。
        assert!(s.contains("操作建议"), "应包含「操作建议」: {}", s);
        assert!(s.contains("买入"), "应包含中文「买入」: {}", s);
        assert!(s.contains("强度"), "应包含「强度」: {}", s);
        assert!(s.contains("42%"), "应包含强度百分比: {}", s);
        assert!(s.contains("关键因子"), "应包含「关键因子」: {}", s);
        assert!(s.contains("涨跌幅"), "应包含 key_factors 中的「涨跌幅」: {}", s);
        assert!(s.contains("支撑"), "应包含「支撑」: {}", s);
        assert!(s.contains("压力"), "应包含「压力」: {}", s);
        assert!(s.contains("日内最低"), "应包含支撑来源「日内最低」: {}", s);
        assert!(s.contains("日内最高"), "应包含压力来源「日内最高」: {}", s);
        assert!(s.contains("100.00"), "应包含现价: {}", s);
        assert!(s.contains("+1.50"), "应包含涨跌幅: {}", s);
    }

    #[test]
    fn test_render_indicators_doesnt_panic() {
        let r = mk_report(SignalKind::Hold, "测试");
        let mut buf = Vec::new();
        write_indicators(&mut buf, &r).unwrap();
        let s = strip_ansi(&String::from_utf8(buf).unwrap());
        assert!(s.contains("位置"), "应包含「位置」: {}", s);
        assert!(s.contains("委比"), "应包含「委比」: {}", s);
        assert!(s.contains("量比"), "应包含「量比」: {}", s);
        assert!(s.contains("价量"), "应包含「价量」: {}", s);
        assert!(s.contains("趋势强度"), "应包含「趋势强度」: {}", s);
        assert!(s.contains("振幅"), "应包含「振幅」: {}", s);
        assert!(s.contains("3.50%"), "应包含振幅数值 3.50%: {}", s);
    }

    #[test]
    fn test_render_advice_doesnt_panic() {
        let r = mk_report(SignalKind::Sell, "ABC");
        let mut buf = Vec::new();
        write_advice(&mut buf, &r).unwrap();
        let s = strip_ansi(&String::from_utf8(buf).unwrap());
        assert!(s.contains("综合得分"), "应包含「综合得分」: {}", s);
        assert!(s.contains("+12.3"), "应包含综合得分数值: {}", s);
        assert!(s.contains("风险提示"), "应包含「风险提示」: {}", s);
        assert!(s.contains("(Tab 翻页"), "应包含翻页提示: {}", s);
    }

    #[test]
    fn test_render_advice_truncates_long() {
        let mut r = mk_report(SignalKind::StrongBuy, "Test");
        r.advice = (0..30).map(|i| format!("建议条目 #{}", i)).collect();
        let mut buf = Vec::new();
        write_advice(&mut buf, &r).unwrap();
        let s = strip_ansi(&String::from_utf8(buf).unwrap());
        assert!(s.contains("...还有"), "应包含「...还有 N 条建议」: {}", s);
        // 不会打印所有 30 条
        assert!(
            !s.contains("建议条目 #29"),
            "应截断，最后一条 #29 不应出现: {}",
            s
        );
    }

    fn mk_report_with_model(
        kind: SignalKind,
        model_prob: f64,
        fusion: FusionStrategy,
    ) -> AnalysisReport {
        let mut r = mk_report(kind, "贵州茅台");
        r.model_prob = Some(model_prob);
        r.fusion_strategy = Some(fusion);
        r.rule_score = 30.0;
        r.final_score = 50.0;
        r
    }

    #[test]
    fn test_overview_renders_model_line_when_model_prob_set() {
        let r = mk_report_with_model(SignalKind::Buy, 0.72, FusionStrategy::Weighted);
        let mut buf = Vec::new();
        write_overview(&mut buf, &r).unwrap();
        let s = strip_ansi(&String::from_utf8(buf).unwrap());
        assert!(s.contains("模型置信"), "应包含「模型置信」: {}", s);
        assert!(s.contains("72.0%"), "应包含概率 72.0%: {}", s);
        assert!(s.contains("weighted"), "应包含融合策略 weighted: {}", s);
        assert!(s.contains("规则=+30"), "应包含规则打分: {}", s);
        assert!(s.contains("最终=+50"), "应包含最终打分: {}", s);
    }

    #[test]
    fn test_overview_no_model_line_when_prob_is_none() {
        let r = mk_report(SignalKind::Hold, "贵州茅台");
        // model_prob = None
        let mut buf = Vec::new();
        write_overview(&mut buf, &r).unwrap();
        let s = strip_ansi(&String::from_utf8(buf).unwrap());
        assert!(
            !s.contains("模型置信"),
            "无 model_prob 时不应渲染模型行: {}",
            s
        );
    }

    #[test]
    fn test_tui_state_no_models_uses_rules_only() {
        use crate::analysis::InferenceContext;
        let s = TuiState::new(FusionStrategy::default(), 0.5, vec![]);
        assert!(s.model_idx.is_none());
        assert!(matches!(
            s.ctx.strategy,
            FusionStrategy::RulesOnly
        ));
        // 即便 fusion 设了 weighted，没模型就是用规则
        assert!(!s.ctx.uses_model());
        let _ = InferenceContext::rules_only();
    }

    #[test]
    fn test_tui_state_cycles_fusion() {
        let s = TuiState::new(FusionStrategy::RulesOnly, 0.5, vec![]);
        // 没模型：cycle_fusion 仍然切，但 ctx 不会使用模型
        let mut s = s;
        s.cycle_fusion();
        assert_eq!(s.fusion, FusionStrategy::ModelOnly);
        s.cycle_fusion();
        assert_eq!(s.fusion, FusionStrategy::Weighted);
        s.cycle_fusion();
        assert_eq!(s.fusion, FusionStrategy::RulesOnly);
    }

    #[test]
    fn test_tui_state_loads_default_model() {
        // 用已建立的 storage 测试 fixture：手动构造一个 LoadedModel
        use crate::model::ml::{
            DtNode, DtParams, LoadedModel, ModelKind, TrainReport, TrainedModel,
        };
        let model = TrainedModel::Dt(DtParams {
            root: DtNode::Leaf { prob: 0.6, n: 1 },
            n_classes: 2,
        });
        let report = TrainReport {
            kind: ModelKind::DecisionTree,
            hold_days: 5,
            threshold_pct: 0.5,
            trained_at: 1,
            train_samples: 100,
            n_features: 13,
            cv_accuracies: vec![0.6, 0.55],
            cv_mean: 0.575,
            cv_min: 0.55,
            cv_max: 0.6,
            train_accuracy: 0.7,
            train_codes: vec!["X".into()],
            feature_importance: None,
        };
        let loaded = LoadedModel {
            id: 1,
            kind: ModelKind::DecisionTree,
            hold_days: 5,
            threshold_pct: 0.5,
            model,
            report,
        };
        let mut s = TuiState::new(FusionStrategy::Weighted, 0.5, vec![loaded.clone()]);
        assert_eq!(s.model_idx, Some(0));
        assert!(s.ctx.uses_model());
        s.cycle_model();
        // 单个模型 -> 循环回 0
        assert_eq!(s.model_idx, Some(0));
        let mut s = TuiState::new(FusionStrategy::RulesOnly, 0.5, vec![loaded]);
        s.cycle_model();
        // 从 None -> 0
        assert_eq!(s.model_idx, Some(0));
        s.clear_model();
        assert!(s.model_idx.is_none());
    }

    #[test]
    fn test_tui_state_model_label() {
        use crate::model::ml::{
            DtNode, DtParams, LoadedModel, ModelKind, TrainReport, TrainedModel,
        };
        let mk = |id: i64, kind: ModelKind| ->LoadedModel {
            LoadedModel {
                id,
                kind,
                hold_days: 5,
                threshold_pct: 0.5,
                model: TrainedModel::Dt(DtParams {
                    root: DtNode::Leaf { prob: 0.5, n: 1 },
                    n_classes: 2,
                }),
                report: TrainReport {
                    kind,
                    hold_days: 5,
                    threshold_pct: 0.5,
                    trained_at: 0,
                    train_samples: 100,
                    n_features: 13,
                    cv_accuracies: vec![0.5],
                    cv_mean: 0.5,
                    cv_min: 0.5,
                    cv_max: 0.5,
                    train_accuracy: 0.5,
                    train_codes: vec![],
                    feature_importance: None,

                },
            }
        };
        let mut s = TuiState::new(FusionStrategy::Weighted, 0.5, vec![
            mk(7, ModelKind::LogisticRegression),
            mk(3, ModelKind::RandomForest),
        ]);
        assert_eq!(s.current_model_label(), "LR5#7(cv=50.0%)");
        s.cycle_model();
        assert_eq!(s.current_model_label(), "RF5#3(cv=50.0%)");
    }

    #[test]
    fn test_format_model_prob_color_thresholds() {
        // > 0.6 应该上色（用 ANSI escape 验证）
        let high = format_model_prob(Some(0.85));
        assert!(high.contains("85"), "应包含 85: {}", high);
        // strip 看看确实是字符串
        let stripped = high.chars().filter(|c| !c.is_control()).collect::<String>();
        assert!(stripped.contains("85"));

        // None -> "-"
        let none = format_model_prob(None);
        assert!(none.contains('-'));

        // 中间 0.5
        let mid = format_model_prob(Some(0.5));
        assert!(mid.contains("50"));

        // 低 < 0.4
        let low = format_model_prob(Some(0.2));
        assert!(low.contains("20"));
    }

    #[test]
    fn test_tui_state_model_label_includes_cv() {
        use crate::model::ml::{
            DtNode, DtParams, LoadedModel, ModelKind, TrainReport, TrainedModel,
        };
        let loaded = LoadedModel {
            id: 5,
            kind: ModelKind::RandomForest,
            hold_days: 3,
            threshold_pct: 0.5,
            model: TrainedModel::Dt(DtParams {
                root: DtNode::Leaf { prob: 0.5, n: 1 },
                n_classes: 2,
            }),
            report: TrainReport {
                kind: ModelKind::RandomForest,
                hold_days: 3,
                threshold_pct: 0.5,
                trained_at: 0,
                train_samples: 1000,
                n_features: 13,
                cv_accuracies: vec![0.58, 0.6, 0.6, 0.62],
                cv_mean: 0.6,
                cv_min: 0.58,
                cv_max: 0.62,
                train_accuracy: 0.7,
                train_codes: vec![],
                feature_importance: None,

            },
        };
        let s = TuiState::new(FusionStrategy::Weighted, 0.5, vec![loaded]);
        assert_eq!(s.current_model_label(), "RF3#5(cv=60.0%)");
    }

    #[test]
    fn test_render_detail_panel_three_pages() {
        let r = mk_report(SignalKind::Buy, "贵州茅台");
        for page in [
            DetailPage::Overview,
            DetailPage::Indicators,
            DetailPage::Advice,
        ] {
            let mut buf = Vec::new();
            write_detail_panel(&mut buf, &r, page).unwrap();
            let s = strip_ansi(&String::from_utf8(buf).unwrap());
            assert!(s.contains("贵州茅台"), "标题应包含股票名: {}", s);
            assert!(s.contains("600519"), "标题应包含代码: {}", s);
            assert!(s.contains(&format!("[{}]", page.label())), "应包含当前页标签: {}", s);
            assert!(s.contains("分析详情"), "应包含「分析详情」: {}", s);
            assert!(s.contains("┌─"), "应包含上边框: {}", s);
            assert!(s.contains("└"), "应包含下边框: {}", s);
        }
    }

    #[test]
    fn test_render_overview_empty_factors() {
        let mut r = mk_report(SignalKind::Hold, "X");
        r.key_factors.clear();
        let mut buf = Vec::new();
        write_overview(&mut buf, &r).unwrap();
        let s = strip_ansi(&String::from_utf8(buf).unwrap());
        // 没有关键因子时显示 "-"
        assert!(s.contains("关键因子: -"), "空因子应显示占位「-」: {}", s);
    }

    #[test]
    fn test_render_overview_no_levels() {
        let mut r = mk_report(SignalKind::StrongSell, "NoLevel");
        r.support = None;
        r.resistance = None;
        let mut buf = Vec::new();
        write_overview(&mut buf, &r).unwrap();
        let s = strip_ansi(&String::from_utf8(buf).unwrap());
        assert!(
            s.contains("支撑 (无足够数据)"),
            "无支撑时应显示占位: {}",
            s
        );
        assert!(
            s.contains("压力 (无足够数据)"),
            "无压力时应显示占位: {}",
            s
        );
    }

    /// 去掉 ANSI 转义码，便于断言可见内容
    fn strip_ansi(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\x1b' && chars.peek() == Some(&'[') {
                chars.next();
                while let Some(&nc) = chars.peek() {
                    chars.next();
                    if nc.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }
}





