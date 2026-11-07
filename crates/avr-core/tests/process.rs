//! 子进程回归测试使用本测试可执行文件作夹具，不依赖 shell、Python 或媒体工具。

use std::io::Write;
use std::path::Path;
use std::time::Duration;

use avr_core::error::ErrorCode;
use avr_core::process::{run, ProcessRequest};

const MAX_STDOUT: usize = 4 * 1024 * 1024;
const MAX_STDERR: usize = 1024 * 1024;

fn request(fixture: &str) -> ProcessRequest {
    ProcessRequest {
        program: std::env::current_exe().unwrap(),
        args: vec![
            "--exact".into(),
            fixture.into(),
            "--ignored".into(),
            "--nocapture".into(),
            "--test-threads=1".into(),
        ],
        cwd: None,
        timeout: Some(Duration::from_secs(10)),
        expected_outputs: vec![],
    }
}

fn write_bytes(mut output: impl Write, byte: u8, count: usize) {
    let chunk = [byte; 8192];
    let mut remaining = count;
    while remaining > 0 {
        let n = remaining.min(chunk.len());
        output.write_all(&chunk[..n]).unwrap();
        remaining -= n;
    }
    output.flush().unwrap();
}

fn fixture_requested(name: &str) -> bool {
    // 普通 cargo test -- --ignored 不应执行夹具的 exit 或文件写入。
    std::env::args().any(|arg| arg == name)
}

#[test]
#[ignore = "subprocess fixture; invoked by process regression tests"]
fn fixture_stderr_first() {
    if !fixture_requested("fixture_stderr_first") {
        return;
    }
    write_bytes(std::io::stderr().lock(), b'e', 256 * 1024);
    write_bytes(std::io::stdout().lock(), b'o', 16);
    std::process::exit(0);
}

#[test]
#[ignore = "subprocess fixture; invoked by process regression tests"]
fn fixture_over_both_limits() {
    if !fixture_requested("fixture_over_both_limits") {
        return;
    }
    write_bytes(std::io::stdout().lock(), b'o', MAX_STDOUT * 2);
    write_bytes(std::io::stderr().lock(), b'e', MAX_STDERR * 2);
    std::process::exit(0);
}

#[test]
#[ignore = "subprocess fixture; invoked by process regression tests"]
fn fixture_unicode_failure() {
    if !fixture_requested("fixture_unicode_failure") {
        return;
    }
    std::io::stderr()
        .write_all("错".repeat(200).as_bytes())
        .unwrap();
    std::process::exit(7);
}

#[test]
#[ignore = "subprocess fixture; invoked by process regression tests"]
fn fixture_invalid_utf8() {
    if !fixture_requested("fixture_invalid_utf8") {
        return;
    }
    std::io::stderr().write_all(b"ok\xff").unwrap();
    std::process::exit(0);
}

#[test]
#[ignore = "subprocess fixture; invoked by process regression tests"]
fn fixture_heartbeat() {
    if !fixture_requested("fixture_heartbeat") {
        return;
    }
    std::fs::write("pid", std::process::id().to_string()).unwrap();
    let mut heartbeat = std::fs::File::create("heartbeat").unwrap();
    // 有限生命周期，防止在回归失败时留下永久运行的测试进程。
    for _ in 0..200 {
        heartbeat.write_all(b".").unwrap();
        heartbeat.flush().unwrap();
        std::thread::sleep(Duration::from_millis(50));
    }
    std::process::exit(0);
}

#[tokio::test]
async fn drains_stderr_while_stdout_is_still_open() {
    let output = run(&request("fixture_stderr_first")).await.unwrap();
    assert_eq!(output.exit_code, 0);
    assert!(output.stdout.ends_with(&"o".repeat(16)));
    assert_eq!(output.stderr, "e".repeat(256 * 1024));
}

#[tokio::test]
async fn drains_discarded_bytes_after_both_retention_limits() {
    let output = run(&request("fixture_over_both_limits")).await.unwrap();
    assert_eq!(output.exit_code, 0);
    assert_eq!(output.stdout.len(), MAX_STDOUT);
    assert!(output.stdout.ends_with(&"o".repeat(8192)));
    assert_eq!(output.stderr, "e".repeat(MAX_STDERR));
}

#[tokio::test]
async fn nonzero_exit_with_unicode_stderr_is_an_error_not_a_panic() {
    let error = run(&request("fixture_unicode_failure")).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::ProcessFailed);
    assert!(error.message.contains("exit=7"));
    assert!(error.message.ends_with(&format!("{}...", "错".repeat(166))));
}

#[tokio::test]
async fn invalid_utf8_is_decoded_lossily() {
    let mut req = request("fixture_invalid_utf8");
    req.timeout = None;
    let output = tokio::time::timeout(Duration::from_secs(10), run(&req))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(output.stderr, "ok\u{fffd}");
}

async fn assert_child_stopped(dir: &Path) {
    let heartbeat = dir.join("heartbeat");
    // kill_on_drop 启动终止操作；给操作系统少量时间完成退出。
    tokio::time::sleep(Duration::from_millis(200)).await;
    let size = std::fs::metadata(&heartbeat).unwrap().len();
    assert!(size > 0, "fixture must have started before it was stopped");
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(std::fs::metadata(&heartbeat).unwrap().len(), size);

    #[cfg(target_os = "linux")]
    {
        let pid = std::fs::read_to_string(dir.join("pid")).unwrap();
        let proc = std::path::PathBuf::from(format!("/proc/{pid}"));
        tokio::time::timeout(Duration::from_secs(5), async {
            while proc.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("child must be reaped, not left as a zombie");
    }
}

#[tokio::test]
async fn timeout_terminates_and_reaps_child() {
    let dir = tempfile::tempdir().unwrap();
    let mut req = request("fixture_heartbeat");
    req.cwd = Some(dir.path().to_path_buf());
    req.timeout = Some(Duration::from_secs(2));
    let error = run(&req).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::ProcessFailed);
    assert_eq!(error.message, "子进程超时");
    assert_child_stopped(dir.path()).await;
}

#[tokio::test]
async fn cancellation_terminates_child() {
    let dir = tempfile::tempdir().unwrap();
    let mut req = request("fixture_heartbeat");
    req.cwd = Some(dir.path().to_path_buf());
    let task = tokio::spawn(async move { run(&req).await });
    tokio::time::timeout(Duration::from_secs(5), async {
        while std::fs::metadata(dir.path().join("heartbeat"))
            .map(|m| m.len() == 0)
            .unwrap_or(true)
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("fixture must start");
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_child_stopped(dir.path()).await;
}
