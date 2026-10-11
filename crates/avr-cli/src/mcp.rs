//! Headless MCP 入口。协议实现仍在 `avr-mcp`，这里只负责启动。

use std::sync::Arc;

use avr_core::error::AvrError;
use avr_core::jobs::JobManager;
use avr_core::pipeline::Deps;

pub fn run_mcp() -> Result<(), AvrError> {
    let ctx = crate::context()?;
    let deps = Deps::new(Arc::new(JobManager::new()));
    avr_mcp::server::run_stdio(ctx, deps)
}
