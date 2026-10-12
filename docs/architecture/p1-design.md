# P1 架构设计

> 历史设计记录；当前职责和实现见 [工程边界](engineering.md)。

## 目标

- `avr-core` 对外只暴露一个分析入口：`analyze(ctx, src, opts, deps) -> Result<AnalyzeResult, AvrError>`。
- GUI / CLI / MCP 三个入口只做输入输出适配（参数解析 → `Source`/`AnalyzeOptions`；`AnalyzeResult` → 各自格式）。
- MCP 的 JSON 拼装（`compact_mcp_payload` / `full_mcp_payload`）从核心库移到 `desktop/src-tauri/src/mcp.rs`。

## 分步与验收

每步一个提交，通用验收：`cargo fmt --all -- --check` 干净；`cargo clippy -p avr-core --all-targets -- -D warnings` 无警告；`cargo test -p avr-core` 全过且测试数不减少；MSRV 1.88 跑测试；desktop 部分以 Windows CI 为准。

| 步骤 | 内容 | 额外验收 |
|---|---|---|
| P1-1 | 新增 `enum Source { Url(String), Local(PathBuf) }`、`AnalyzeOptions`、`Deps` 与统一接口 `analyze`；`analyze_url` / `transcribe_local` 改为 `#[deprecated]` 薄包装，行为不变 | 现有测试全过 |
| P1-2 | `JobManager` 经 `Deps` 注入，删除 `shared_job_manager()`；desktop 用 `app.manage(AppState)`，CLI / MCP 自建同一个 `AppState` | 全仓库搜不到 `shared_job_manager`；新增测试证明两个独立 `JobManager` 互不干扰 |
| P1-3 | `ToolId` 枚举替代硬编码工具名；`async_trait` 的 `Tools` trait，真实实现 `RealTools`，测试实现 `FakeTools` | 不装外部程序跑通一个 pipeline 测试 |
| P1-4 | `AvrError` 分层：thiserror、`#[non_exhaustive]`、`#[source]` 保留底层原因，不依赖 serde；Tauri 侧可序列化包装（`to_string` + 错误码） | 错误链测试 |
| P1-5 | payload 拼装移到 desktop `mcp.rs`；GUI / CLI / MCP 都走 `analyze` | 核心库不再含 MCP JSON；Windows CI 通过 |

## 只加不改的约束

- `analyze` 签名在 P1-1 一次定好，后续步骤不再修改。
- `Deps`：字段私有，构造只走 `Deps::new(jobs)`；P1-3 增加 `with_tools(...)` builder；测试用 `Deps::for_test()`（`cfg(test)` 门控，带默认假实现）。
- `AnalyzeOptions`：`#[non_exhaustive]` + 私有字段 + builder，实现 `Default`。
- 这样新增依赖或选项都只是新增方法，不破坏调用方。

## 错误分层

- 核心库：`thiserror` 定义 `AvrError`，`#[non_exhaustive]`，底层错误用 `#[source]` 挂载，`Display` 不重复 source 内容；不依赖 serde。
- 入口层（CLI / MCP）：用 `anyhow` 的 `.context()` 补上下文，打印时用 `{:#}` 输出整条错误链。
- Tauri 命令：返回可 `Serialize` 的包装类型，输出 `to_string()` 与错误码。

## 依赖注入

- Tauri：启动时 `app.manage(AppState)`，命令里用 `State<'_, AppState>` 取 `Deps`。
- CLI / MCP：不经过 Tauri，自己构造同一个 `AppState` 并传给 `analyze`，三个入口用同一套依赖。
