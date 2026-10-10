//! 统一子进程执行器（唯一允许 spawn 的模块）。

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::timeout;
use tracing::debug;

use crate::error::{process_failed, AvrError, ErrorCode};

const MAX_STDOUT: usize = 4 * 1024 * 1024;
const MAX_STDERR: usize = 1 * 1024 * 1024;

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
        .stderr(Stdio::piped());

    if let Some(cwd) = &req.cwd {
        cmd.current_dir(cwd);
    }

    #[cfg(windows)]
    assign_job_object(&mut cmd);

    let child = cmd
        .spawn()
        .map_err(|e| process_failed(&format!("无法启动 {}: {}", req.program.display(), e)))?;

    let pid = child.id();

    let run_inner = async {
        let mut child = child;
        let mut stdout = child.stdout.take().expect("stdout");
        let mut stderr = child.stderr.take().expect("stderr");

        let (mut out_buf, mut err_buf) = (Vec::new(), Vec::new());
        let (mut out_tmp, mut err_tmp) = ([0u8; 8192], [0u8; 8192]);

        while out_buf.len() < MAX_STDOUT {
            let n = stdout.read(&mut out_tmp).await.unwrap_or(0);
            if n == 0 {
                break;
            }
            out_buf.extend_from_slice(&out_tmp[..n]);
        }
        while err_buf.len() < MAX_STDERR {
            let n = stderr.read(&mut err_tmp).await.unwrap_or(0);
            if n == 0 {
                break;
            }
            err_buf.extend_from_slice(&err_tmp[..n]);
        }

        let status = child
            .wait()
            .await
            .map_err(|e| process_failed(&e.to_string()))?;
        let code = status.code().unwrap_or(-1);

        for expected in &req.expected_outputs {
            if !expected.exists() {
                return Err(process_failed(&format!(
                    "预期产物不存在: {}",
                    expected.display()
                )));
            }
        }

        Ok(ProcessOutput {
            exit_code: code,
            stdout: String::from_utf8_lossy(&out_buf).to_string(),
            stderr: String::from_utf8_lossy(&err_buf).to_string(),
        })
    };

    let output = if let Some(dur) = req.timeout {
        match timeout(dur, run_inner).await {
            Ok(r) => r?,
            Err(_) => {
                #[cfg(windows)]
                if let Some(p) = pid {
                    kill_process_tree(p);
                }
                return Err(AvrError::new(
                    ErrorCode::ProcessFailed,
                    "子进程超时",
                    "可尝试缩短输入或增加超时。",
                ));
            }
        }
    } else {
        run_inner.await?
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

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

#[cfg(windows)]
fn assign_job_object(cmd: &mut Command) {
    // Job Object 在 spawn 后通过子进程继承较难做到跨 tokio；
    // 超时/取消时使用 kill_process_tree 清理整棵树。
    let _ = cmd;
}

#[cfg(windows)]
fn kill_process_tree(pid: u32) {
    let _ = std::process::Command::new("taskkill")
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .status();
}

#[cfg(not(windows))]
fn kill_process_tree(_pid: u32) {}

/// 同步包装（供非 async 上下文测试）。
pub fn run_blocking(req: &ProcessRequest) -> Result<ProcessOutput, AvrError> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(run(req))
}
