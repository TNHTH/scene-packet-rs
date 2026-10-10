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
        let root = self.root.lock();
        PipelineContext {
            root: root.clone(),
            config: self.config.lock().clone(),
        }
    }
}

pub type SharedState = Arc<AppState>;
