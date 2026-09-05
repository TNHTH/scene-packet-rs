//! 共享应用状态。

use avr_core::config::AppConfig;
use avr_core::pipeline::PipelineContext;
use avr_core::root::DataRoot;
use parking_lot::Mutex;
use std::sync::Arc;

pub struct AppState {
    pub root: Mutex<DataRoot>,
    pub config: Mutex<AppConfig>,
}

impl AppState {
    pub fn pipeline_ctx(&self) -> PipelineContext {
        PipelineContext {
            root: self.root.lock().clone(),
            config: self.config.lock().clone(),
        }
    }
}

pub type SharedState = Arc<AppState>;
