//! 命令行入口

use std::io::{self, stdout, Write};
use std::time::Duration;
use anyhow::Context;
use colored::*;

use gupiao::cli::{self, Command, FavCommand};
use gupiao::display::AnalysisDisplay;
use gupiao::model::{Favorite, KlineSeries, Symbol, Trend};
use gupiao::platform::enable_utf8_console;
use gupiao::provider::{EastMoneyNewsProvider, KlineProvider, QuoteProvider, SinaKlineProvider, TencentProvider};
use gupiao::storage::{Database, FavoriteStore, KlineStore, ModelStore};
use gupiao::analysis::{ScoreWeights, FusionStrategy, InferenceContext};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    enable_utf8_console();
    let cli = cli::parse_cli();
    let command = cli.command.unwrap_or(Command::Query(gupiao::cli::QueryArgs {
        code: String::new(), interval: 0, depth: true, fav: false,
    }));

    match command {
        Command::Tui => gupiao::tui::run().await,
        Command::Query(args) => { if args.code.is_empty() { anyhow::bail!("missing stock code"); } cmd_query(args).await }
        Command::Fav(fav) => cmd_fav(fav),
        Command::Watch(args) => cmd_watch(args).await,
        Command::Analyze(args) => cmd_analyze(args).await,
        Command::Kline(args) => cmd_kline(args).await,
        Command::Backtest(args) => cmd_backtest(args).await,
        Command::Report(args) => cmd_report(args).await,
        Command::Optimize(args) => cmd_optimize(args).await,
        Command::Train(args) => cmd_train(args).await,
        Command::Model(args) => cmd_model(args),
    }
}

async fn cmd_query(args: gupiao::cli::QueryArgs) -> anyhow::Result<()> {
    let symbol = Symbol::new(&args.code).context("parse stock code failed")?;
    if args.fav {
        let db = Database::open().context("open db failed")?;
        let store = FavoriteStore::new(&db);
        let existed = store.contains(&symbol.code, symbol.market)?;
        let fav = Favorite::new(symbol.code.clone(), symbol.market, None);
        store.add(&fav).context("add favorite failed")?;
        println!("{} {}.{}", if existed { "Updated" } else { "Added" }.green().bold(), symbol.code, symbol.market);
    }
    let provider = TencentProvider::new();
    if args.interval == 0 {
        let quote = provider.fetch(&symbol).await?;
        println!("{} {}.{}: {} ({:+.2}%)", quote.name, symbol.code, symbol.market, quote.price, quote.change_pct);
    } else {
        loop {
            match provider.fetch(&symbol).await {
                Ok(quote) => println!("{} {}.{}: {} ({:+.2}%)", quote.name, symbol.code, symbol.market, quote.price, quote.change_pct),
                Err(e) => eprintln!("fetch failed: {}", e),
            }
            tokio::time::sleep(Duration::from_secs(args.interval)).await;
        }
    }
    Ok(())
}

fn cmd_fav(fav: FavCommand) -> anyhow::Result<()> {
    let db = Database::open().context("open db failed")?;
    let store = FavoriteStore::new(&db);
    match fav {
        FavCommand::Add(args) => {
            let symbol = Symbol::new(&args.code).context("parse stock code failed")?;
            let existed = store.contains(&symbol.code, symbol.market)?;
            let fav = Favorite::new(symbol.code.clone(), symbol.market, args.note.clone());
            store.add(&fav).context("add favorite failed")?;
            println!("{} {}.{}", if existed { "Updated" } else { "Added" }.green().bold(), symbol.code, symbol.market);
        }
        FavCommand::Remove(args) => {
            let symbol = Symbol::new(&args.code).context("parse stock code failed")?;
            if store.remove(&symbol.code, symbol.market)? { println!("{}", "Removed".red().bold()); }
            else { println!("{}", "Not found".yellow()); }
        }
        FavCommand::List => {
            let list = store.list()?;
            if list.is_empty() { println!("(empty)"); }
            else { for fav in list { println!("{}.{} {:?}", fav.code, fav.market, fav.note); } }
        }
        FavCommand::Clear => println!("(not implemented)"),
    }
    Ok(())
}

async fn cmd_watch(_args: gupiao::cli::WatchArgs) -> anyhow::Result<()> { println!("(use gupiao tui)"); Ok(()) }

async fn cmd_analyze(args: gupiao::cli::AnalyzeArgs) -> anyhow::Result<()> {
    let symbol = Symbol::new(&args.code).context("parse stock code failed")?;
    let provider = TencentProvider::new();
    let display = AnalysisDisplay;
    let weights = match args.weights.as_str() {
        "optimal" => ScoreWeights::optimal(),
        "default" => ScoreWeights::default(),
        other => anyhow::bail!("unknown weights: {}", other),
    };
    let fusion_strategy = FusionStrategy::from_str(&args.fusion)
        .ok_or_else(|| anyhow::anyhow!("unknown fusion: {}", args.fusion))?;
    let ctx = load_inference_ctx(&fusion_strategy, args.alpha)?;
    run_analyze_once(&provider, &display, &symbol, args.json, &weights, &ctx).await
}

fn load_inference_ctx(strategy: &FusionStrategy, alpha: f64) -> anyhow::Result<InferenceContext> {
    if matches!(strategy, FusionStrategy::RulesOnly) { return Ok(InferenceContext::rules_only()); }
    let db = match Database::open() { Ok(d) => d, Err(_) => return Ok(InferenceContext::rules_only()) };
    let store = ModelStore::new(&db);
    match store.load_default() {
        Ok(Some(loaded)) => {
            println!("OK model loaded (id={}, kind={}, cv={:.1}%)", loaded.id, loaded.kind.label(), loaded.report.cv_mean * 100.0);
            Ok(InferenceContext::with_model(loaded, *strategy, alpha))
        }
        Ok(None) => { println!("WARN no model, using rules only"); Ok(InferenceContext::rules_only()) }
        Err(e) => { println!("WARN model load failed: {}, using rules", e); Ok(InferenceContext::rules_only()) }
    }
}

async fn run_analyze_once<P: QuoteProvider>(provider: &P, display: &AnalysisDisplay, symbol: &Symbol, as_json: bool, weights: &ScoreWeights, ctx: &InferenceContext) -> anyhow::Result<()> {
    let quote_fut = provider.fetch(symbol);
    let news_provider = EastMoneyNewsProvider::new();
    let news_fut = news_provider.fetch_all(symbol);
    let (quote_result, news_result) = futures::future::join(quote_fut, news_fut).await;

    let quote = quote_result?;
    let base_report = if ctx.uses_model() {
        match load_klines_for(symbol).await {
            Ok(series) => gupiao::analysis::analyze_with_model(&quote, weights, &Default::default(), &series, ctx),
            Err(_) => gupiao::analysis::analyze_with(&quote, weights, &Default::default()),
        }
    } else {
        gupiao::analysis::analyze_with(&quote, weights, &Default::default())
    };

    let report = match news_result {
        Ok(news) if !news.announcements.is_empty() || !news.news.is_empty() => base_report.with_news(news),
        Ok(_) => base_report,
        Err(e) => { eprintln!("WARN fetch news failed: {}", e); base_report }
    };

    let mut out = stdout();
    if as_json { write_json(&mut out, &report)?; } else { display.write(&mut out, &report)?; }
    out.flush()?;
    Ok(())
}

async fn load_klines_for(symbol: &Symbol) -> anyhow::Result<KlineSeries> {
    let db = Database::open()?;
    let store = KlineStore::new(&db);
    let cached = store.count(&symbol.code, symbol.market)?;
    if cached >= 60 { return Ok(store.load_range(&symbol.code, symbol.market, None, None)?); }
    let provider = SinaKlineProvider::new();
    let series = provider.fetch(symbol, 250).await?;
    let _ = store.upsert_series(&symbol.code, symbol.market, &series)?;
    Ok(series)
}

fn write_json<W: Write>(w: &mut W, report: &gupiao::analysis::AnalysisReport) -> io::Result<()> {
    use serde::Serialize;
    #[derive(Serialize)]
    struct JsonOut<'a> {
        code: &'a str, name: &'a str, market: &'a str, current_price: f64, change_pct: f64,
        signal: &'a str, strength: u8, advice: Vec<&'a str>, key_factors: Vec<&'a str>,
        support: Option<f64>, resistance: Option<f64>, model_prob: Option<f64>, rule_score: f64, final_score: f64,
    }
    let out = JsonOut {
        code: &report.symbol_code, name: &report.symbol_name, market: &report.market,
        current_price: report.current_price, change_pct: report.change_pct,
        signal: report.signal.kind.label(), strength: report.signal.strength,
        advice: report.advice.iter().map(|s| s.as_str()).collect(),
        key_factors: report.key_factors.iter().map(|s| s.as_str()).collect(),
        support: report.support.map(|s| s.price), resistance: report.resistance.map(|s| s.price),
        model_prob: report.model_prob, rule_score: report.rule_score, final_score: report.final_score,
    };
    serde_json::to_writer(w, &out).map_err(|e| io::Error::new(io::ErrorKind::Other, e))
}

async fn cmd_kline(_args: gupiao::cli::KlineArgs) -> anyhow::Result<()> { println!("(not impl)"); Ok(()) }
async fn cmd_backtest(_args: gupiao::cli::BacktestArgs) -> anyhow::Result<()> { println!("(not impl)"); Ok(()) }
async fn cmd_report(_args: gupiao::cli::ReportArgs) -> anyhow::Result<()> { println!("(not impl)"); Ok(()) }
async fn cmd_optimize(_args: gupiao::cli::OptimizeArgs) -> anyhow::Result<()> { println!("(not impl)"); Ok(()) }
async fn cmd_train(args: gupiao::cli::TrainArgs) -> anyhow::Result<()> { println!("train kind={}", args.kind); Ok(()) }

fn cmd_model(args: gupiao::cli::ModelCommand) -> anyhow::Result<()> {
    let db = Database::open()?;
    let store = ModelStore::new(&db);
    use gupiao::cli::ModelCommand as MC;
    match args {
        MC::List => { let list = store.list()?; if list.is_empty() { println!("(no models)"); return Ok(()); } for m in &list { println!("{}: {} hold={} cv={:.1}%", m.id, m.kind.label(), m.hold_days, m.report.cv_mean * 100.0); } }
        MC::Show { id } => { let loaded = store.load(id)?.ok_or_else(|| anyhow::anyhow!("model {} not found", id))?; println!("Model id={}", loaded.id); println!("  kind={} hold={}", loaded.kind.label(), loaded.hold_days); }
        MC::Use { id } => { store.set_default(id)?; println!("OK model {} set as default", id); }
        MC::Delete { id } => { let n = store.delete(id)?; if n == 0 { anyhow::bail!("model {} not found", id); } println!("OK model {} deleted", id); }
    }
    Ok(())
}

#[allow(dead_code)]
fn _force_use_trend(t: Trend) -> Trend { t }
