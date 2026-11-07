//! 统一子进程执行器（唯一允许 spawn 的模块）。

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio::time::timeout;
use tracing::debug;

use crate::error::{process_failed, AvrError, ErrorCode};

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

/// 执行子进程；参数数组直传，禁止 shell 拼接。
pub async fn run(req: &ProcessRequest) -> Result<ProcessOutput, AvrError> {
    debug!("spawn: {} {}", req.program.display(), req.args.join(" "));

    let mut cmd = Command::new(&req.program);
    cmd.args(&req.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    if let Some(cwd) = &req.cwd {
        cmd.current_dir(cwd);
    }

    #[cfg(windows)]
    assign_job_object(&mut cmd);

    let mut child = cmd
        .spawn()
        .map_err(|e| process_failed(&format!("无法启动 {}: {}", req.program.display(), e)))?;

    let stdout = child.stdout.take().expect("stdout");
    let stderr = child.stderr.take().expect("stderr");
    let run_inner = async {
        // 两条管道与 wait 同时推进；达到保留上限后仍须读到 EOF，
        // 否则子进程可能阻塞在写满的管道上，永远无法退出。
        tokio::try_join!(
            drain_bounded(stdout, MAX_STDOUT),
            drain_bounded(stderr, MAX_STDERR),
            child.wait(),
        )
        .map_err(|e| process_failed(&e.to_string()))
    };

    let result = if let Some(dur) = req.timeout {
        match timeout(dur, run_inner).await {
            Ok(r) => r,
            Err(_) => Err(AvrError::new(
                ErrorCode::ProcessFailed,
                "子进程超时",
                "可尝试缩短输入或增加超时。",
            )),
        }
    } else {
        run_inner.await
    };

    let (out_buf, err_buf, status) = match result {
        Ok(output) => output,
        Err(error) => {
            #[cfg(windows)]
            if let Some(p) = child.id() {
                kill_process_tree(p);
            }
            // kill().await 同时等待回收；取消 run future 则由 kill_on_drop 兜底。
            let _ = child.kill().await;
            return Err(error);
        }
    };

    for expected in &req.expected_outputs {
        if !expected.exists() {
            return Err(process_failed(&format!(
                "预期产物不存在: {}",
                expected.display()
            )));
        }
    }

    let output = ProcessOutput {
        exit_code: status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out_buf).into_owned(),
        stderr: String::from_utf8_lossy(&err_buf).into_owned(),
    };

    if output.exit_code != 0 {
        return Err(process_failed(&format!(
            "exit={} stderr={}",
            output.exit_code,
            truncate(&output.stderr, 500)
        )));
    }

    Ok(output)
}

async fn drain_bounded(mut reader: impl AsyncRead + Unpin, max: usize) -> std::io::Result<Vec<u8>> {
    let mut retained = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            return Ok(retained);
        }
        let keep = n.min(max - retained.len());
        retained.extend_from_slice(&buf[..keep]);
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut end = max;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

#[cfg(windows)]
fn assign_job_object(cmd: &mut Command) {
    // Job Object 在 spawn 后通过子进程继承较难做到跨 tokio；
    // 超时/读取失败时使用 kill_process_tree 清理整棵树；取消时杀掉直接子进程。
    let _ = cmd;
}

#[cfg(windows)]
fn kill_process_tree(pid: u32) {
    let _ = std::process::Command::new("taskkill")
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .status();
}

/// 同步包装（供非 async 上下文测试）。
pub fn run_blocking(req: &ProcessRequest) -> Result<ProcessOutput, AvrError> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| process_failed(&format!("无法创建 tokio 运行时: {e}")))?
        .block_on(run(req))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use tokio::io::ReadBuf;

    #[tokio::test]
    async fn retention_limit_does_not_stop_draining() {
        let input = vec![b'x'; 20_000];
        for limit in [0, 1, 8193, input.len(), input.len() + 1] {
            let mut reader = input.as_slice();
            let output = drain_bounded(&mut reader, limit).await.unwrap();
            assert_eq!(output, input[..limit.min(input.len())]);
            assert!(reader.is_empty(), "must drain beyond the retention limit");
        }
    }

    #[tokio::test]
    async fn read_errors_are_not_treated_as_eof() {
        struct FailingReader;
        impl AsyncRead for FailingReader {
            fn poll_read(
                self: Pin<&mut Self>,
                _: &mut Context<'_>,
                _: &mut ReadBuf<'_>,
            ) -> Poll<std::io::Result<()>> {
                Poll::Ready(Err(std::io::Error::other("read failed")))
            }
        }
        let error = drain_bounded(FailingReader, 10).await.unwrap_err();
        assert_eq!(error.to_string(), "read failed");
    }

    #[test]
    fn truncation_respects_utf8_boundaries() {
        assert_eq!(truncate("", 0), "");
        assert_eq!(truncate("abc", 3), "abc");
        assert_eq!(truncate("abc", 2), "ab...");
        assert_eq!(truncate("中文错误", 0), "...");
        assert_eq!(truncate("中文错误", 1), "...");
        assert_eq!(truncate("中文错误", 5), "中...");
        assert_eq!(truncate("a😀b", 4), "a...");
        assert_eq!(
            truncate(&"错".repeat(200), 500),
            format!("{}...", "错".repeat(166))
        );
    }
}
