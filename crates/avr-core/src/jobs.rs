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

    /// 同一缓存目录只执行一次；不同数据根不共享等待状态。
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
        let key = format!("{}:{}", cache_dir.display(), media_key);
        let waiter = {
            let mut map = self.in_flight.lock();
            if let Some(waiters) = map.get_mut(&key) {
                let (tx, rx) = oneshot::channel();
                waiters.push(tx);
                Some(rx)
            } else {
                map.insert(key.clone(), Vec::new());
                None
            }
        };
        if let Some(rx) = waiter {
            return rx.await.unwrap_or_else(|_| Err(cancelled()));
        }
        let mut guard = CompletionGuard {
            manager: self,
            key,
            result: Err(cancelled()),
        };
        let result = async {
            fs::create_dir_all(cache_dir).map_err(io_err)?;
            let lock = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(cache_dir.join(".lock"))
                .map_err(io_err)?;
            fs2::FileExt::try_lock_exclusive(&lock).map_err(|e| {
                AvrError::new(
                    ErrorCode::CacheError,
                    format!("缓存被其他进程占用: {e}"),
                    "等待持有者结束后重试。",
                )
            })?;
            // 文件锁随句柄释放，崩溃/取消不留下不可恢复的存在性锁。
            let result = work().await;
            drop(lock);
            result
        }
        .await;
        guard.result = result.clone();
        result
    }
}
impl Default for JobManager {
    fn default() -> Self {
        Self::new()
    }
}
fn cancelled() -> AvrError {
    AvrError::new(
        ErrorCode::JobCancelled,
        "任务已取消或执行者退出",
        "可以重新执行。",
    )
}
struct CompletionGuard<'a> {
    manager: &'a JobManager,
    key: String,
    result: Result<(), AvrError>,
}
impl Drop for CompletionGuard<'_> {
    fn drop(&mut self) {
        if let Some(waiters) = self.manager.in_flight.lock().remove(&self.key) {
            for waiter in waiters {
                let _ = waiter.send(self.result.clone());
            }
        }
    }
}

pub fn atomic_write(path: &Path, content: &[u8]) -> Result<(), AvrError> {
    let parent = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(io_err)?;
    let part = parent.join(format!(".{}.part", uuid::Uuid::new_v4()));
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
