//! 作业管理：锁、singleflight、原子写入、进度事件。

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use fs4::fs_std::FileExt;
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
    ///
    /// 进程内用 singleflight 合并并发调用；跨进程用操作系统建议锁（flock / LockFileEx）
    /// 互斥，进程崩溃时由操作系统自动释放，不会残留阻塞性的锁。
    /// leader 被取消或 panic 时，等待者收到 `JobCancelled` 错误，而不是误报成功。
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
        let rx = {
            let mut map = self.in_flight.lock();
            if let Some(waiters) = map.get_mut(media_key) {
                let (tx, rx) = oneshot::channel();
                waiters.push(tx);
                Some(rx)
            } else {
                map.insert(media_key.to_string(), Vec::new());
                None
            }
        };

        if let Some(rx) = rx {
            return rx.await.unwrap_or_else(|_| Err(leader_aborted()));
        }

        // 从这里开始，无论正常返回、出错、被取消还是 panic，InFlight 的 Drop 都会通知等待者。
        let mut in_flight = InFlight {
            mgr: self,
            key: media_key,
            result: None,
        };

        let result = match FileLock::try_acquire(&cache_dir.join(".lock")) {
            Ok(_guard) => work().await,
            Err(e) => Err(e),
        };
        in_flight.result = Some(result.clone());
        result
    }
}

/// singleflight 条目守卫：Drop 时移除条目并把结果（或取消错误）广播给等待者。
struct InFlight<'a> {
    mgr: &'a JobManager,
    key: &'a str,
    result: Option<Result<(), AvrError>>,
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        let result = self.result.take().unwrap_or_else(|| Err(leader_aborted()));
        let waiters = self.mgr.in_flight.lock().remove(self.key);
        for w in waiters.into_iter().flatten() {
            let _ = w.send(result.clone());
        }
    }
}

fn leader_aborted() -> AvrError {
    AvrError::new(
        ErrorCode::JobCancelled,
        "同一媒体的处理任务被中断",
        "重新发起分析即可。",
    )
}

/// 跨进程文件锁（RAII）。锁由操作系统持有，文件句柄关闭即释放；
/// 锁文件本身留在磁盘上无害，不再以“文件是否存在”判断占用。
pub struct FileLock {
    _file: fs::File,
}

impl FileLock {
    pub fn try_acquire(path: &Path) -> Result<Self, AvrError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io_err)?;
        }
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(io_err)?;
        let locked = FileExt::try_lock_exclusive(&file).map_err(io_err)?;
        if !locked {
            return Err(AvrError::new(
                ErrorCode::CacheError,
                "另一个任务正在处理同一媒体",
                "等待当前任务完成后再试。",
            ));
        }
        Ok(Self { _file: file })
    }
}

impl Default for JobManager {
    fn default() -> Self {
        Self::new()
    }
}

pub use crate::storage::atomic_write;

pub fn write_manifest(cache_dir: &Path, manifest: &JobManifest) -> Result<(), AvrError> {
    let path = cache_dir.join("manifest.json");
    let json = serde_json::to_string_pretty(manifest).map_err(|e| {
        AvrError::new(ErrorCode::IoError, "序列化作业清单失败", "检查磁盘权限。").with_source(e)
    })?;
    atomic_write(&path, json.as_bytes())?;
    Ok(())
}

fn io_err(e: std::io::Error) -> AvrError {
    AvrError::io(e, "检查磁盘权限。")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;
    use tempfile::TempDir;

    /// 回归：崩溃残留的 .lock 文件不再阻塞后续任务。
    #[tokio::test]
    async fn stale_lock_file_is_taken_over() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join(".lock"), "1").unwrap();
        let mgr = JobManager::new();
        let r = mgr.run_once("k", tmp.path(), || async { Ok(()) }).await;
        assert!(r.is_ok(), "{r:?}");
    }

    #[test]
    fn file_lock_is_exclusive_and_released_on_drop() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".lock");
        let first = FileLock::try_acquire(&path).unwrap();
        assert!(FileLock::try_acquire(&path).is_err());
        drop(first);
        assert!(FileLock::try_acquire(&path).is_ok());
    }

    #[tokio::test]
    async fn concurrent_calls_run_work_once() {
        let tmp = TempDir::new().unwrap();
        let mgr = Arc::new(JobManager::new());
        let runs = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for _ in 0..4 {
            let (mgr, runs, dir) = (mgr.clone(), runs.clone(), tmp.path().to_path_buf());
            handles.push(tokio::spawn(async move {
                mgr.run_once("k", &dir, move || async move {
                    runs.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    Ok(())
                })
                .await
            }));
        }
        for h in handles {
            assert!(h.await.unwrap().is_ok());
        }
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    /// 回归：leader 被取消时，等待者必须收到错误而不是误报成功。
    #[tokio::test]
    async fn waiter_gets_error_when_leader_cancelled() {
        let tmp = TempDir::new().unwrap();
        let mgr = Arc::new(JobManager::new());
        let dir = tmp.path().to_path_buf();
        let leader = {
            let (mgr, dir) = (mgr.clone(), dir.clone());
            tokio::spawn(async move {
                mgr.run_once("k", &dir, || async {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    Ok(())
                })
                .await
            })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        let waiter = {
            let (mgr, dir) = (mgr.clone(), dir.clone());
            tokio::spawn(async move { mgr.run_once("k", &dir, || async { Ok(()) }).await })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        leader.abort();
        let r = waiter.await.unwrap();
        assert!(
            matches!(r, Err(ref e) if e.code == ErrorCode::JobCancelled),
            "{r:?}"
        );
        // 条目和锁都已释放，可以重新执行。
        assert!(mgr.run_once("k", &dir, || async { Ok(()) }).await.is_ok());
    }

    #[tokio::test]
    async fn waiter_receives_leader_error() {
        let tmp = TempDir::new().unwrap();
        let mgr = Arc::new(JobManager::new());
        let dir = tmp.path().to_path_buf();
        let leader = {
            let (mgr, dir) = (mgr.clone(), dir.clone());
            tokio::spawn(async move {
                mgr.run_once("k", &dir, || async {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    Err(AvrError::new(ErrorCode::ProcessFailed, "boom", ""))
                })
                .await
            })
        };
        tokio::time::sleep(Duration::from_millis(10)).await;
        let w = mgr.run_once("k", &dir, || async { Ok(()) }).await;
        assert!(leader.await.unwrap().is_err());
        assert!(matches!(w, Err(ref e) if e.code == ErrorCode::ProcessFailed));
    }

    /// P1-2：两个独立注入的 JobManager 互不干扰（同 key 各自执行、事件互不串台）。
    #[tokio::test]
    async fn independent_managers_do_not_interfere() {
        let (a_dir, b_dir) = (TempDir::new().unwrap(), TempDir::new().unwrap());
        let (a, b) = (Arc::new(JobManager::new()), Arc::new(JobManager::new()));
        let mut a_rx = a.subscribe();
        let runs = Arc::new(AtomicUsize::new(0));
        let spawn = |mgr: Arc<JobManager>, dir: std::path::PathBuf| {
            let runs = runs.clone();
            tokio::spawn(async move {
                mgr.run_once("same-key", &dir, move || async move {
                    runs.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    Ok(())
                })
                .await
            })
        };
        let ha = spawn(a.clone(), a_dir.path().to_path_buf());
        let hb = spawn(b.clone(), b_dir.path().to_path_buf());
        assert!(ha.await.unwrap().is_ok());
        assert!(hb.await.unwrap().is_ok());
        assert_eq!(runs.load(Ordering::SeqCst), 2, "每个实例各自执行一次");
        b.emit(JobProgress::Done {
            media_key: "b".into(),
        });
        assert!(a_rx.try_recv().is_err(), "a 收不到 b 的进度事件");
    }
}
