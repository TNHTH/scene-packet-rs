# ADR 0007：MCP 输出改为摘要索引 + 按需读取
状态：提议（2026-10-11，架构师；研究员补充做法）

背景：仓库目的是把视频变成 AI 可用的资料包，MCP 输出的质量与体积是最终效果。
决策：
- P3-1 基线：ffmpeg `testsrc2` + `sine` 现生成测试视频；转写用 `FakeTools` 固定内容，关键帧与时间轴走真实 ffmpeg；`insta` 快照只记结构（帧数范围、时间戳落在对应分段），不记具体帧时间与图片哈希；记录 compact payload 字节数与 `o200k_base` token 参考值。CI 显式装 ffmpeg，本机无 ffmpeg 时跳过而非失败。`insta`、`tiktoken-rs` 只进 `dev-dependencies`。
- P3-2：`analyze` 只返回摘要索引（分段、时间范围、一句话概要、帧编号）；新增按时间范围分页取转写（`cursor`/`nextCursor`）、按编号取帧两个只读工具；帧同时以 resource `avr://job/{id}/frame/{n}` 暴露、工具版兜底；声明 `outputSchema` 并返回 `structuredContent` 与文本 `content`；`transcribe_and_save` 参数收成 `StageCtx`。
验收：首次返回 token 明显减少；往返测试证明索引 + 两个工具可还原完整原始 payload。
