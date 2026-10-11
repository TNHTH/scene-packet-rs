# ADR 0005：P1 架构——统一入口、依赖注入、工具 trait、错误分层
状态：已采纳（2026-10-11，P1-1～P1-5 已验收，最新 `00c75a4`）

决策（按提交拆分）：
1. P1-1 `enum Source { Url, Local }` + `analyze(ctx, src, opts, deps)`；旧接口保留为 deprecated 薄包装。
2. P1-2 `JobManager` 经 `Deps` 注入，删除 `shared_job_manager()`；desktop 用 `app.manage(AppState)`，CLI/MCP 自建同一 `AppState`。
3. P1-3 `ToolId` 枚举 + `async_trait` 的 `Tools` trait（`RealTools` / `FakeTools`），不装外部程序可跑 pipeline 测试。
4. P1-4 核心库 `thiserror`：按调用方处理方式分变体、`#[source]`、`#[non_exhaustive]`、不依赖 serde；入口层 `anyhow` + `.context()`；Tauri 用可序列化包装类型返回错误。
5. P1-5 MCP payload 拼装移到 `desktop/src-tauri/src/mcp.rs`，三入口都走 `analyze`（仅 Windows CI 可验证，放最后）。

依据：研究员关于 thiserror/anyhow 分层与 Tauri 状态管理的调研（`docs/research/`）。

API 稳定性约束（架构师、研究员补充）：`analyze` 签名在 P1-1 一次定死；`Deps`、`Options` 字段私有，`Deps::new(jobs)` 构造、`with_tools(...)` 等 builder 扩展、测试用 `Deps::for_test()`；`Options` 实现 `Default`。

## 实施记录
- P1-1 `34ea752`、P1-2 `8920c25`、P1-3 `2a67672`、CI 修复 `7db85d6`、P1-4 `a366f8e`、P1-5 `f17a0e7`；删旧接口 `316dd46` + fmt `00c75a4`。

## 修订
- **作废**：原第 4 点"入口层用 anyhow + `.context()`"。原因：CLI 退出码依赖 `e.code`，anyhow 会丢掉错误码；`AvrError` 已带 `#[source]` 原因链与 `format_user()`。错误统一为 `AvrError` + `ErrorCode` 单层。
- **偏离**：deprecated 薄包装 `analyze_url`/`transcribe_local` 未保留，直接删除（每次新建 JobManager，并发时报"正在处理"，且仅仓库内部使用）。
