# CLAUDE.md — gupiao 项目开发笔记

> ⚠️ **Cline 用户注意**：本文件**不会被 Cline 自动读取**（与 `.clinerules` 不同）。
> **简短、可执行的硬规则**放在仓库根目录的 `.clinerules`（Cline 每次会话自动加载）。
> 本文件是**详细文档 + 背景解释**，需要时手动 `read_files` 查阅。

给后续接手 / AI 助手看的「踩坑记录 + 项目约定」。
**优先于 README**：这里的规则都是实战验证过的，不要重复犯。

## 文件分工

| 文件 | 谁会读 | 内容 |
|---|---|---|
| `.clinerules` | **Cline 每次会话自动加载** | 12 条短规则，全是可执行项 |
| `CLAUDE.md`（本文件）| 手动查阅 | 详细背景、调试清单、待办、变更日志 |
| `README.md` | 用户 | 功能介绍、使用方法 |

---

## 0. TL;DR — 三条最容易犯的错

1. **永远不要用 `Add-Content` / `Set-Content` 写中文到 `.rs` 文件**——会被 GBK 编码，
   导致 cargo 报 `stream did not contain valid UTF-8`。**用 `editor` 工具写。**
2. **PowerShell 下 `cargo build` 即便成功也会返回非零退出码**（stderr 输出被当错误）。
   看是否成功要看 `Finished` / `error:`，而不是 `$LASTEXITCODE`。
3. **clap 子命令里无法识别顶层 `-V` / `--help`**——需要手动检测
   `argv` 中是否只有全局标志，绕过「默认子命令」包装。

---

## 1. Windows / PowerShell 编码陷阱

### 1.1 `Add-Content` / `Set-Content` 会把中文写成 GBK

**症状：**

```
error: couldn't read `src\xxx.rs`: stream did not contain valid UTF-8
note: byte `179` is not valid utf-8
```

**根因：** Windows PowerShell 默认 `OutputEncoding` / `Default` 是 GBK（系统代码页 936）。
`Add-Content` 写入字符串时按本机代码页编码，不会自动转 UTF-8。

**解决：**

| 操作 | 工具 |
|---|---|
| 创建或重写整个文件（含中文） | `editor` 工具 |
| 修改已有文件（局部替换） | `editor` 工具（`old_text` / `new_text`）|
| 写入**纯 ASCII** 文本 | `Add-Content` 可以 |
| 写入**中文**内容 | 绝对不能用 `Add-Content` |

**如果已经写坏了：** 整个文件用 `editor` 工具重写——不要尝试
`[System.IO.File]::WriteAllText` 之类的修复，原字节已经丢失。

**检测方式：**

```powershell
$bytes = [System.IO.File]::ReadAllBytes("e:\path\to\file.rs")
# 看首字节：UTF-8 BOM = EF BB BF，UTF-8 无 BOM = ASCII 字母，GBK = 高位字节
```

### 1.2 测试输出在 PowerShell 捕获时显示为乱码

**症状：** 直接执行 `gupiao.exe 600519` 终端显示正常，但
`gupiao.exe 600519 2>&1 | Out-String` 看到 `璐靛窞鑼呭彴 ...`。

**根因：** 不是程序问题，是 PowerShell 的 `Out-String` 把 stdout 按 GBK 重新编码。
**解决：** 直接打印到终端即可，不要 `Out-String` 捕获。

### 1.3 必须 `enable_utf8_console()`

`src/platform.rs` 已经处理：Windows 上 `SetConsoleOutputCP(65001)`。
**别忘了在 `main()` 最开始调用**，否则 GBK 响应解码后输出的中文会乱码。

---

## 2. PowerShell 下 cargo 的退出码问题

**症状：** `cargo build` 成功后单独看 `$LASTEXITCODE` 是 0，
但 `cargo build; ...` 的整个管道返回非零。

**根因：** PowerShell 把任何带 stderr 输出的命令视为「NativeCommandError」，
即便 cargo 本身返回 0。

**判断 cargo 是否成功的正确方法：**

```powershell
cargo build 2>&1 | Tee-Object -FilePath build.log
# 检查 build.log 中是否包含 "Finished" 或 "error:"
if (Select-String -Path build.log -Pattern "error:") { ...失败... } else { ...成功... }
```

或者只关心 `Finished` 关键字是否出现：

```powershell
cargo build 2>&1 | Select-String "Finished|error\[E"
```

---

## 3. clap 子命令架构 — 「默认子命令」模式

### 3.1 需求

希望同时支持：
- `gupiao 600519`（旧式，单只查询）
- `gupiao query 600519`（新式，显式子命令）
- `gupiao fav add 600519`
- `gupiao watch`
- `gupiao -V` / `gupiao --help`（顶层）

### 3.2 实现要点（见 `src/cli.rs::parse_cli`）

```rust
pub fn parse_cli() -> Cli {
    let raw: Vec<String> = std::env::args().collect();
    let known_subs = ["query", "q", "fav", "watch", "w"];

    let first_positional = raw.iter().skip(1).find(|a| !a.starts_with('-'));

    match first_positional {
        // 显式子命令
        Some(p) if known_subs.iter().any(|s| s == p) => Cli::parse(),
        // 只有全局标志 → 让 clap 打印顶层 help/version
        None => Cli::parse(),
        // 有位置参数但不是子命令 → 视为旧式 query
        Some(_) => {
            let mut new_argv = vec![raw[0].clone(), "query".to_string()];
            new_argv.extend_from_slice(&raw[1..]);
            Cli::parse_from(new_argv)
        }
    }
}
```

### 3.3 坑：`Cli::command` 必须是 `Option<Command>`

```rust
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,   // ← 必须是 Option
}
```

否则无子命令时会编译失败（clap 要求至少一个子命令或允许空）。

### 3.4 坑：子命令里接受「股票代码」位置参数

```rust
#[derive(Parser, Debug)]
pub struct QueryArgs {
    pub code: String,    // 位置参数，不要加默认值
    ...
}
```

如果加了 `default_value`，用户输入非法代码时 clap 会接受空字符串，导致后面
`Symbol::new("")` 失败但报错信息不直观。**保持必填**，让 clap 自己报「missing field」。

---

## 4. SQLite + WAL 模式的测试陷阱

### 4.1 临时文件残留导致测试串扰

**症状：** 单个测试通过，多个测试一起跑时失败：

```
test test_remove ... FAILED
  left: 3
 right: 0     # 期望删完后剩 0 条，实际有 3 条
```

**根因：** `Database::open_at(path)` 用 `std::process::id()` 命名临时目录，
但**同一测试进程**内所有测试共享 PID，残留的 WAL/SHM 文件导致计数累加。

**解决：** 用原子计数器为每个测试生成唯一目录，且用 `remove_dir_all` 而非 `remove_file`：

```rust
static COUNTER: AtomicU64 = AtomicU64::new(0);

fn fresh_db() -> Database {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "gupiao_fav_test_{}_{}", std::process::id(), n
    ));
    let _ = std::fs::remove_dir_all(&dir);   // ← 必须删整个目录（含 .db-wal / .db-shm）
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("fav.db");
    Database::open_at(&path).unwrap()
}
```

### 4.2 不要启用 `rusqlite` 的 `chrono` 特性

当前 `Cargo.toml`：

```toml
rusqlite = { version = "0.32", features = ["bundled"] }
# 注意：不要加 "chrono" 特性。我们用 i64 Unix 时间戳自行转换，避免版本冲突。
```

时间戳统一用 `chrono::Utc::now().timestamp()` 存为 i64，
读取时 `chrono::DateTime::from_timestamp(secs, 0)` 转回。

---

## 5. 跨平台配置目录

用 `dirs` crate：

```rust
let base = dirs::config_dir()
    .or_else(dirs::data_dir)
    .ok_or(StorageError::NoConfigDir)?;
```

| OS | 路径 |
|---|---|
| Windows | `%APPDATA%\gupiao\gupiao.db` (即 `C:\Users\<user>\AppData\Roaming\gupiao\`) |
| Linux | `~/.config/gupiao/gupiao.db` |
| macOS | `~/Library/Application Support/gupiao/gupiao.db` |

测试覆盖路径用 `GUPIAO_DB` 环境变量覆盖。

---

## 6. 项目架构约定

### 6.1 模块边界

```
src/
├── cli.rs         # 命令行参数（clap derive）
├── main.rs        # 入口：分发到子命令
├── lib.rs         # 模块声明 + 公开 API
├── error.rs       # 库错误类型（thiserror）
├── platform.rs    # 跨平台兼容（目前只 Windows UTF-8）
├── model/         # 纯数据，不依赖 IO
│   ├── market.rs  # Market / Symbol（代码归一化）
│   ├── quote.rs   # Quote / Depth / Trend
│   └── favorite.rs # ← 新增
├── provider/      # 数据源 trait + 实现
│   ├── mod.rs     # QuoteProvider trait
│   └── tencent.rs # 腾讯财经实现
├── display/       # 输出格式 trait + 实现
│   ├── mod.rs     # QuoteDisplay trait
│   ├── terminal.rs # 彩色终端
│   └── watch.rs   # ← 新增：自选股列表
└── storage/       # ← 新增：本地持久化
    ├── mod.rs     # 错误类型
    ├── db.rs      # 连接管理 + schema 迁移
    └── favorite.rs # 收藏 DAO
```

**依赖方向（严格）：** `cli` → `main` → {`provider`, `display`, `storage`, `model`}
`storage` → `model`；`provider` / `display` / `storage` 之间互不依赖。

### 6.2 新增数据源 / 输出格式 / 存储表的步骤

| 新增类型 | 文件 | 改动点 |
|---|---|---|
| 数据源 | `provider/xxx.rs` | 实现 `QuoteProvider` |
| 输出格式 | `display/xxx.rs` | 实现 `QuoteDisplay` |
| 存储表 | `storage/xxx.rs` + `storage/db.rs::migrate()` | 递增 `CURRENT_SCHEMA_VERSION`，加 `if from < N { ... }` |

### 6.3 测试约定

- **库单测**：放在每个文件的 `#[cfg(test)] mod tests`
- **集成测试**：未来如果要端到端测试，放 `tests/` 目录
- **临时文件**：用 `std::env::temp_dir()` + 原子计数器（见 §4.1）
- **不要引入 `tempfile` 等 dev-dependency**——保持依赖最小

---

## 7. 容易踩的 Rust / 工具坑

### 7.1 `Cargo.lock` 不要手动编辑

之前手动编辑 `Cargo.lock` 加 `futures` 那次是没必要的——`Cargo.toml` 加依赖后
`cargo build` 会自动更新 lock。

### 7.2 `editor` 工具的 `old_text` 必须精确匹配

包括空格、换行、标点。如果匹配失败，会返回「text not found」错误。
**不要靠记忆改文件**——先 `read_files` 看清楚当前内容。

### 7.3 `editor` 工具的 `new_text` 大小限制

单次编辑建议 < 6000 字符（实际限制更高但建议遵守）。
**大文件分多次编辑**——先改头部，再改中段，最后追加。
或者用 `insert_line` 在指定行号处插入新内容。

### 7.4 `run_commands` 中不能用 `printf`

PowerShell 没有 `printf`，要用 `Write-Host -NoNewline ("{0:X2} " -f $byte)`。

### 7.5 `Default::default()` 对包含 chrono 字段的结构不友好

`Quote::depth: Default::default()` 是可以的（`Depth` 派生了 `Default`）。
但 `update_time: DateTime<Local>` 必须显式给 `Local::now()`——别图省事用 `Default`，
否则会得到 epoch 时间。

---

## 8. 调试清单（速查）

| 现象 | 第一步检查 |
|---|---|
| cargo 报 `stream did not contain valid UTF-8` | §1.1——文件是否被 GBK 写过 |
| cargo 报 `couldn't read XXX.rs` | 文件是否还在（`Get-ChildItem`）|
| 测试失败 `left: N, right: M` 且数字偏大 | §4.1——临时目录隔离 |
| `gupiao -V` 报 `unexpected argument` | §3.2——`parse_cli` 是否走顶层分支 |
| 中文输出乱码（终端） | §1.3——`enable_utf8_console()` 是否调用 |
| 中文输出乱码（捕获） | §1.2——不要用 `Out-String` |
| `cargo build` exit code 非 0 | §2——看 stderr 是否含 `error:` |
| `linker link.exe not found` | 安装 Visual Studio Build Tools |
| `could not find native static library sqlite3` | 已经用 `bundled` 特性，不会出现；如出现检查 `Cargo.toml` |

---

## 9. 后续待办

- [ ] 集成测试（端到端 `assert_cmd`）
- [ ] 历史 K 线缓存表
- [ ] 价格提醒功能
- [ ] 自选股分组（多 group）
- [ ] JSON 输出格式
- [ ] 东财数据源
- [ ] 配置文件（`~/.config/gupiao/config.toml`）

---

## 10. 变更日志（本文件）

- **2026-09-07 初版**：记录 fav/watch 功能开发中踩的坑。
- **2026-09-08 TUI + Analyze**：
  - 新增 `analyze` 子命令（`gupiao analyze 600519`）和 `tui` 子命令（`gupiao tui`）
  - TUI 主表格新增「信号 / 强度」列，按 Enter 展开 detail 面板（3 页可 Tab 翻页）
  - 详情面板渲染抽成 `write_<page>(W: Write)` 形式，方便单测对内容做断言
  - `truncate_cn` / `pad_cn` / `pad_cn_ansi` 等宽度工具独立出来，统一项目里 CJK 显示宽度处理

