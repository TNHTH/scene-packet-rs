# ADR 0003：Tauri crate 由 Windows CI 验证
状态：已采纳（2026-10-10）

背景：Linux 开发机缺 GTK/WebKit 依赖且安装失败；`generate_context!` 编译期需要 `ui/dist`。
决策：Linux 只验证 avr-core；desktop 在 Windows CI 上先 `npm ci && npm run build` 再跑 clippy 和测试。
后果：Tauri 改动的反馈回路变长；若后续本机装好 GTK，可补 Linux desktop job。
