//! ScenePacket 的 MCP 输出层：把 `avr_core::AnalyzeResult` 拼成 MCP 工具返回的 JSON。
//!
//! 独立成 crate 是为了让它不依赖 rmcp / Tauri / GTK，在 Linux CI 上也能测试，
//! 同时保持 avr-core 不关心输出格式（P1-5 的约束）。

pub mod index;
pub mod payload;
