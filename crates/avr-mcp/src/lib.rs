//! ScenePacket 的 MCP 层：payload 拼装（纯函数）与 rmcp 服务实现。
//!
//! 独立成 crate 是为了不依赖 Tauri / GTK，在 Linux CI 上也能测试（含进程内 MCP 集成测试），
//! 同时保持 avr-core 不关心输出格式（P1-5 的约束）。desktop 只负责在 stdio 上启动服务。

pub mod index;
pub mod payload;
pub mod server;
