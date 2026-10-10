//! 共享应用状态：GUI 通过 `app.manage` 注入，CLI / MCP 自建同一个 AppState。

use avr_core::config::{load_config, AppConfig};
use avr_core::error::AvrError;
use avr_core::jobs::JobManager;
use avr_core::pipeline::{Deps, PipelineContext};
use avr_core::root::{resolve_data_root, DataRoot};
use parking_lot::Mutex;
use std::sync::Arc;

pub struct AppState {
    pub root: Mutex<DataRoot>,
    pub config: Mutex<AppConfig>,
    deps: Deps,
}

impl AppState {
    pub fn new(root: DataRoot, config: AppConfig) -> Self {
        Self {
            root: Mutex::new(root),
            config: Mutex::new(config),
            deps: Deps::new(Arc::new(JobManager::new())),
        }
    }

    /// 按 exe 所在目录解析数据根、建目录、读配置。三个入口共用。
    pub fn load() -> Result<Self, AvrError> {
        let exe = std::env::current_exe().ok();
        let exe_dir = exe.as_ref().and_then(|p| p.parent());
        let root = resolve_data_root(exe_dir)?;
        root.ensure_layout()?;
        let config = load_config(&root)?;
        Ok(Self::new(root, config))
    }

    pub fn pipeline_ctx(&self) -> PipelineContext {
        PipelineContext {
            root: self.root.lock().clone(),
            config: self.config.lock().clone(),
        }
    }

    pub fn deps(&self) -> &Deps {
        &self.deps
    }
}

pub type SharedState = Arc<AppState>;
