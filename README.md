# gupiao — A 股实时行情命令行工具

一个用 Rust 编写的 A 股（沪市 / 深市 / 北交所）实时行情命令行查询工具。
数据源为腾讯财经公开行情接口（`qt.gtimg.cn`），**无需登录账号**。

## 特性

- ✅ 沪市 / 深市 / 创业板 / 科创板 / 北交所自动识别
- ✅ 显示最新价、涨跌、今开/最高/最低/昨收、成交量/成交额
- ✅ 显示换手率、市盈率(动)、市净率、总市值/流通市值、振幅
- ✅ 完整五档买卖盘
- ✅ 彩色输出：红涨绿跌（符合 A 股习惯）
- ✅ 自动定时刷新 `-i N`
- ✅ **自选股收藏**：`fav add/remove/list/clear`
- ✅ **自选股实时一览**：`watch` 一键查看所有收藏
- ✅ **键盘驱动 TUI**：`tui` 一键进入实时行情 + 分析面板
- ✅ **自动分析**：`analyze` 综合信号 + 关键因子 + 买/卖点预测
- ✅ GBK 编码自动处理，Windows 控制台中文显示正常
- ✅ **模块化架构**，便于扩展新的数据源和输出格式
- ✅ **SQLite 本地存储**（嵌入式，无需额外服务）

## 架构

```
┌─────────────────────────────────────────────────────────────┐
│  CLI (cli.rs)                  子命令解析（query/fav/watch）  │
├─────────────────────────────────────────────────────────────┤
│  Provider (provider/)          数据源抽象                      │
│     ├─ tencent  : 腾讯财经（默认）                            │
│     └─ eastmoney: 东方财富（可扩展）                          │
├─────────────────────────────────────────────────────────────┤
│  Display (display/)            输出格式抽象                    │
│     ├─ terminal: 彩色终端（单只股票详情，默认）                │
│     └─ watch    : 自选股列表式（多只股票一览）                │
├─────────────────────────────────────────────────────────────┤
│  Storage (storage/)            本地持久化（SQLite）            │
│     └─ favorites: 自选股 CRUD（后续可加历史/配置）            │
├─────────────────────────────────────────────────────────────┤
│  Model (model/)                核心数据类型                    │
│     Market / Symbol / Quote / Depth / Trend / Favorite      │
├─────────────────────────────────────────────────────────────┤
│  Platform / Error              跨平台兼容 / 统一错误           │
└─────────────────────────────────────────────────────────────┘
```

核心思想：通过 `QuoteProvider` 和 `QuoteDisplay` 两个 trait，
任何数据源（腾讯/东财/新浪）和任何输出格式（终端/JSON/CSV）都可以互不干扰地扩展。

## 安装与运行

```bash
cd e:\hzy\gupiao
cargo build --release
.\target\release\gupiao.exe 600519
```

## 使用

### 1. 单次查询（兼容旧版用法）

```bash
# 单次查询
gupiao 600519               # 贵州茅台（沪市）
gupiao 000001               # 平安银行（深市）
gupiao 300750               # 宁德时代（创业板）
gupiao 688981               # 中芯国际（科创板）
gupiao 830799               # 艾融软件（北交所）

# 多种代码格式都支持
gupiao sh600519
gupiao sz000001
gupiao 600519.SH
gupiao 000001.sz

# 自动刷新
gupiao 600519 -i 3          # 每 3 秒

# 不显示五档
gupiao 600519 --depth false

# 查询的同时加入自选股
gupiao 688981 -f
```

### 2. 自选股（收藏）管理

数据存储在 SQLite 中（跨平台位置）：
- Windows: `%APPDATA%\gupiao\gupiao.db`
- Linux: `~/.config/gupiao/gupiao.db`
- macOS: `~/Library/Application Support/gupiao/gupiao.db`

可通过环境变量 `GUPIAO_DB` 覆盖路径。

```bash
# 添加到自选股
gupiao fav add 600519
gupiao fav add 000001 -n "long-term"     # 带备注
gupiao fav add 300750 -n "watch until 400"

# 从自选股移除
gupiao fav remove 600519

# 列出所有自选股
gupiao fav list          # alias: gupiao fav ls

# 清空
gupiao fav clear
```

### 3. 一键查看自选股实时行情

```bash
# 单次拉取所有自选股的行情（表格视图）
gupiao watch

# 每 3 秒自动刷新
gupiao watch -i 3
```

### 4. 键盘驱动 TUI（含分析面板）

```bash
gupiao tui            # alias: gupiao ui
```

主表格每一行直接显示**信号 + 强度**：

```
> 名称       代码       现价      涨跌%      信号     强度  更新时间
  贵州茅台   600519.SH   1316.01   -1.05%     观望     42%   -
```

按 **Enter** 在底部展开/折叠当前选中股票的**分析详情面板**，
面板分 3 页可按 **Tab** 切换：

| 键 | 作用 |
|----|------|
| `↑` / `↓` | 切换自选股（折叠详情时） |
| `Enter` | 展开 / 折叠当前股票的分析详情 |
| `Tab` | 在详情面板三页（概览 / 指标 / 建议）间翻页 |
| `a` | 添加自选股 |
| `d` | 删除当前选中的自选股 |
| `r` | 立即刷新 |
| `q` / `Esc` / `Ctrl+C` | 退出 TUI |

### 5. 单只股票深度分析

```bash
gupiao analyze 600519              # 一次性分析（alias: gupiao a 600519）
gupiao analyze 600519 -i 10        # 每 10 秒自动刷新
gupiao analyze 600519 --json       # 输出 JSON（便于脚本消费）
```

输出包括：综合信号（强烈买入 / 买入 / 观望 / 卖出 / 强烈卖出）、
关键因子、价位预测（建议买点 / 卖点 + 依据）、详细建议列表。

输出示例：
```
fetching 3 favorites ...
  名称       代码     市场       现价       涨跌     涨跌幅             更新时间
  ────────────────────────────────────────────────────────────────────────────
  贵州茅台 600519.SH   SH   1316.01   -13.99  -1.05%  2026-09-07 16:14:58
  宁德时代 300750.SZ   SZ    348.20   -2.80  -0.80%  2026-09-07 16:14:21
  中芯国际 688981.SH   SH    124.12    +2.98  +2.46%  2026-09-07 16:14:41
```

### 6. 历史 K 线（数据源：新浪财经）

```bash
gupiao kline 600519                # 拉取最近 250 个交易日的日 K（alias: gupiao k）
gupiao kline 000001 --days 500     # 拉取更多
gupiao kline 600519 --refresh      # 强制刷新缓存
```

K 线自动缓存到 SQLite，重复运行不会重复请求。**仅基于公开日 K，不依赖任何账号**。

### 7. 算法准确度回测

```bash
gupiao backtest 600519                       # 默认持仓 5 日（alias: gupiao bt）
gupiao backtest 600519 --hold 10             # 持仓 10 日
gupiao backtest 600519 --all-signals         # 评估全部信号（含 Hold/Sell）
gupiao backtest 600519 --days 500 --refresh  # 拉更多历史
```

输出报告：胜率 / 平均收益 / 中位收益 / 最大回撤 / Profit Factor，
按信号类型分组、按强度分桶、抽样交易明细。

### 8. 批量回测自选股（汇总表 + CSV）

```bash
gupiao report                                  # 默认 5 只股票 × 5 个周期 (alias: gupiao re)
gupiao report --hold 1,3,5,10,20              # 自定义周期
gupiao report --days 500 --refresh            # 拉更多历史
gupiao report --csv my_report.csv             # 指定 CSV 输出路径
```

### 9. 算法权重优化（贡献度分析 + 网格搜索）

```bash
gupiao optimize                                # 跑贡献度分析 + 729 种权重组合穷举
gupiao optimize --top 20                      # 显示 Top-20
gupiao optimize --hold 1,3,5                  # 自定义评估周期
```

**优化结果**（基于 5 只自选股 × 250 日历史 × 5 个持仓周期）：

| 指标 | 基线（原算法权重） | 优化后（网格搜索） | 提升 |
|---|---|---|---|
| 平均胜率 | 46.12% | **59.30%** | **+13.19%** |
| 平均收益 | -0.074% | **+0.532%** | **+0.606%** |
| Profit Factor | 1.02 | **2.15** | **+111%** |

**贡献度分析**（剔除后胜率变化）：

| 维度 | 剔除后胜率变化 | 结论 |
|---|---|---|
| 量能 | **-7.84%** | ✅ 强正贡献（最重要） |
| 价量配合 | **-7.84%** | ✅ 强正贡献（最重要） |
| 趋势强度 | +1.11% | ❌ 负贡献（剔除后更好） |
| 涨跌幅 | +0.63% | 🟡 弱负贡献 |
| 价格位置 | +0.44% | 🟡 弱负贡献 |
| 委比 | +0.00% | ⚪ 中性（深市无盘口） |

**最优配置**：`pct=25, pos=0, comm=0, vol=0, pv=0, trend=50`
（保留：涨跌幅、趋势强度；剔除：价格位置、委比、量能、价量配合）

可通过 `gupiao analyze 600519 --weights optimal` 启用最优配置。

### 10. 用最优权重实时分析

```bash
gupiao analyze 600519 --weights default        # 默认算法权重
gupiao analyze 600519 --weights optimal        # 网格搜索最优权重（推荐）
gupiao analyze 600519 -i 10 --weights optimal  # 每 10 秒刷新
```

### 4. 完整 help

```bash
gupiao --help              # 顶层
gupiao query --help        # 单只查询
gupiao fav --help          # 自选股管理
gupiao watch --help        # 自选股一览
gupiao tui --help          # 键盘驱动 TUI
gupiao analyze --help      # 单只股票深度分析
```

## 作为库使用

```rust
use gupiao::model::Symbol;
use gupiao::provider::{QuoteProvider, TencentProvider};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let sym = Symbol::new("600519")?;
    let quote = TencentProvider::new().fetch(&sym).await?;
    println!("{} 最新价: {} 元", quote.name, quote.price);
    Ok(())
}
```

直接操作 SQLite：

```rust
use gupiao::storage::{Database, FavoriteStore};
use gupiao::model::{Favorite, Market};

let db = Database::open()?;
let store = FavoriteStore::new(&db);
store.add(&Favorite::new("600519".into(), Market::Shanghai, None))?;
for fav in store.list()? {
    println!("{:>6}.{:} {:?}", fav.code, fav.market, fav.note);
}
```

## 数据库 Schema

当前版本 1，初始化时会自动建表：

```sql
CREATE TABLE schema_version (version INTEGER PRIMARY KEY);

CREATE TABLE favorites (
    code         TEXT    NOT NULL,
    market       TEXT    NOT NULL,    -- 'SH' / 'SZ' / 'BJ'
    note         TEXT,
    added_at     INTEGER NOT NULL,    -- Unix 秒
    sort_order   INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (code, market)
);

CREATE INDEX idx_favorites_added_at ON favorites (added_at);
```

后续扩展（如历史 K 线、价格提醒、自选股分组）只需在 `storage/db.rs::migrate()`
里追加 `if from < N { ... }` 分支。

## 扩展指南

### 添加新的数据源（如东方财富）

1. 在 `src/provider/` 下新建 `eastmoney.rs`
2. 实现 `QuoteProvider` trait
3. 在 `src/main.rs` 中加 `--source` 参数切换

### 添加新的输出格式（如 JSON）

1. 在 `src/display/` 下新建 `json.rs`
2. 实现 `QuoteDisplay` trait
3. 在 `src/main.rs` 加 `--format json` 参数切换

### 添加新的存储表（如价格提醒）

1. 在 `src/storage/` 下新建 `alert.rs`
2. 在 `db.rs::migrate` 里追加 `if from < 2 { ... }` 分支
3. 在 `db.rs` 中把 `CURRENT_SCHEMA_VERSION` 改为 2

### 运行测试

```bash
cargo test --lib
```

## 注意事项

- 数据延迟约 3 秒，仅供个人参考，**不构成任何投资建议**。
- 仅支持 A 股。
- 仅在交易时段（工作日 9:30-11:30、13:00-15:00）有实时变动。
