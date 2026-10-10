//! MCP stdio 入口：构造依赖后交给 avr-mcp 的服务实现。

use crate::state::AppState;
use avr_core::error::AvrError;
pub use avr_mcp::payload::compact_mcp_payload;

pub fn run_mcp() -> Result<(), AvrError> {
    let state = AppState::load()?;
    avr_mcp::server::run_stdio(state.pipeline_ctx(), state.deps().clone())
}
