//! 作业管理：锁、singleflight、原子写入、进度事件。

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, oneshot};

use crate::error::{AvrError, ErrorCode};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobManifest {
    pub media_key: String,
    pub stage: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum JobProgress {
    Stage {
        media_key: String,
        stage: String,
        percent: u8,
    },
    Done {
        media_key: String,
    },
    Failed {
        media_key: String,
        message: String,
    },
}

type Waiters = Vec<oneshot::Sender<Result<(), AvrError>>>;

pub struct JobManager {
    in_flight: Mutex<HashMap<String, Waiters>>,
    progress_tx: broadcast::Sender<JobProgress>,
}

impl JobManager {
    pub fn new() -> Self {
        let (progress_tx, _) = broadcast::channel(64);
        Self {
            in_flight: Mutex::new(HashMap::new()),
            progress_tx,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<JobProgress> {
        self.progress_tx.subscribe()
    }

    pub fn emit(&self, event: JobProgress) {
        let _ = self.progress_tx.send(event);
    }

    /// 同一 media_key 只执行一次，后到者等待并复用结果。
    pub async fn run_once<F, Fut>(
        &self,
        media_key: &str,
        cache_dir: &Path,
        work: F,
    ) -> Result<(), AvrError>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<(), AvrError>> + Send,
    {
        let (rx, is_leader) = {
            let mut map = self.in_flight.lock();
            if let Some(waiters) = map.get_mut(media_key) {
                let (tx, rx) = oneshot::channel();
                waiters.push(tx);
                (Some(rx), false)
            } else {
                map.insert(media_key.to_string(), Vec::new());
                (None, true)
            }
        };

        if !is_leader {
            if let Some(rx) = rx {
                return rx.await.unwrap_or(Ok(()));
            }
        }

        let lock_path = cache_dir.join(".lock");
        acquire_file_lock(&lock_path)?;

        let result = work().await;

        release_file_lock(&lock_path);

        let send_result = match &result {
            Ok(()) => Ok(()),
            Err(e) => Err(e.clone()),
        };
        let mut map = self.in_flight.lock();
        if let Some(waiters) = map.remove(media_key) {
            for w in waiters {
                let _ = w.send(send_result.clone());
            }
        }

        result
    }
}

impl Default for JobManager {
    fn default() -> Self {
        Self::new()
    }
}

fn acquire_file_lock(path: &Path) -> Result<(), AvrError> {
    if path.exists() {
        return Err(AvrError::new(
            ErrorCode::CacheError,
            "另一个任务正在处理同一媒体",
            "等待当前任务完成后再试。",
        ));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io_err)?;
    }
    fs::write(path, "1").map_err(io_err)?;
    Ok(())
}

fn release_file_lock(path: &Path) {
    fs::remove_file(path).ok();
}

pub fn atomic_write(path: &Path, content: &[u8]) -> Result<(), AvrError> {
    let parent = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(io_err)?;
    let part = path.with_extension("part");
    fs::write(&part, content).map_err(io_err)?;
    fs::rename(&part, path).map_err(io_err)?;
    Ok(())
}

pub fn write_manifest(cache_dir: &Path, manifest: &JobManifest) -> Result<(), AvrError> {
    let path = cache_dir.join("manifest.json");
    let json = serde_json::to_string_pretty(manifest).map_err(|e| io_err_msg(&e.to_string()))?;
    atomic_write(&path, json.as_bytes())?;
    Ok(())
}

fn io_err(e: std::io::Error) -> AvrError {
    AvrError::new(ErrorCode::IoError, e.to_string(), "检查磁盘权限。")
}

fn io_err_msg(msg: &str) -> AvrError {
    AvrError::new(ErrorCode::IoError, msg.to_string(), "检查磁盘权限。")
}

pub fn shared_job_manager() -> Arc<JobManager> {
    static MGR: std::sync::OnceLock<Arc<JobManager>> = std::sync::OnceLock::new();
    MGR.get_or_init(|| Arc::new(JobManager::new())).clone()
}
