# ADR 0005：P1 架构——统一入口、依赖注入、工具 trait、错误分层
状态：提议（2026-10-10，架构师）

决策（按提交拆分）：
1. P1-1 `enum Source { Url, Local }` + `analyze(ctx, src, opts, deps)`；旧接口保留为 deprecated 薄包装。
2. P1-2 `JobManager` 经 `Deps` 注入，删除 `shared_job_manager()`；desktop 用 `app.manage(AppState)`，CLI/MCP 自建同一 `AppState`。
3. P1-3 `ToolId` 枚举 + `async_trait` 的 `Tools` trait（`RealTools` / `FakeTools`），不装外部程序可跑 pipeline 测试。
4. P1-4 核心库 `thiserror`：按调用方处理方式分变体、`#[source]`、`#[non_exhaustive]`、不依赖 serde；入口层 `anyhow` + `.context()`；Tauri 用可序列化包装类型返回错误。
5. P1-5 MCP payload 拼装移到 `desktop/src-tauri/src/mcp.rs`，三入口都走 `analyze`（仅 Windows CI 可验证，放最后）。

依据：研究员关于 thiserror/anyhow 分层与 Tauri 状态管理的调研（`docs/research/`）。
