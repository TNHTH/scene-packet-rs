//! 子进程参数直传，并发排空输出，取消和超时回收进程。
use crate::error::{process_failed, AvrError};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
const MAX_STDOUT: usize = 4 * 1024 * 1024;
const MAX_STDERR: usize = 1024 * 1024;
#[derive(Debug, Clone)]
pub struct ProcessRequest {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub timeout: Option<Duration>,
    pub expected_outputs: Vec<PathBuf>,
}
#[derive(Debug, Clone)]
pub struct ProcessOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}
async fn drain(mut stream: impl AsyncRead + Unpin, limit: usize) -> std::io::Result<Vec<u8>> {
    let mut retained = Vec::new();
    let mut buf = [0; 8192];
    loop {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            return Ok(retained);
        }
        let keep = n.min(limit.saturating_sub(retained.len()));
        retained.extend_from_slice(&buf[..keep]);
        // 超过保留上限仍排空，避免子进程因管道满而无法退出。
    }
}
struct ProcessTree(u32);
impl Drop for ProcessTree {
    fn drop(&mut self) {
        #[cfg(unix)]
        // 子进程独占进程组；只向本次创建的组发送信号。
        unsafe {
            libc::kill(-(self.0 as i32), libc::SIGKILL);
        }
        #[cfg(windows)]
        {
            let _ = std::process::Command::new("taskkill")
                .args(["/F", "/T", "/PID", &self.0.to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}
pub async fn run(req: &ProcessRequest) -> Result<ProcessOutput, AvrError> {
    let mut command = Command::new(&req.program);
    command
        .args(&req.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    if let Some(cwd) = &req.cwd {
        command.current_dir(cwd);
    }
    let mut child = command
        .spawn()
        .map_err(|e| process_failed(&format!("无法启动 {}: {e}", req.program.display())))?;
    let tree = ProcessTree(child.id().expect("spawned child has pid"));
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let work = async {
        tokio::try_join!(
            drain(stdout, MAX_STDOUT),
            drain(stderr, MAX_STDERR),
            child.wait()
        )
    };
    let result = match req.timeout {
        Some(duration) => match tokio::time::timeout(duration, work).await {
            Ok(result) => result,
            Err(_) => {
                drop(tree);
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Err(process_failed("子进程超时，已终止并回收"));
            }
        },
        None => work.await,
    };
    drop(tree);
    let (stdout, stderr, status) = result.map_err(|e| process_failed(&e.to_string()))?;
    let output = ProcessOutput {
        exit_code: status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    };
    if !status.success() {
        return Err(process_failed(&format!(
            "exit={} stderr={}",
            output.exit_code,
            crate::models::truncate_utf8(&output.stderr, 2000)
        )));
    }
    for path in &req.expected_outputs {
        if !path.is_file() {
            return Err(process_failed(&format!(
                "预期文件不存在: {}",
                path.display()
            )));
        }
    }
    Ok(output)
}
pub fn run_blocking(req: &ProcessRequest) -> Result<ProcessOutput, AvrError> {
    let execute = || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| process_failed(&e.to_string()))?
            .block_on(run(req))
    };
    if tokio::runtime::Handle::try_current().is_ok() {
        // 兼容同步公共 API；异步消费者应直接 await run，避免阻塞宿主。
        std::thread::scope(|scope| {
            scope
                .spawn(execute)
                .join()
                .map_err(|_| process_failed("同步子进程执行线程异常"))?
        })
    } else {
        execute()
    }
}
