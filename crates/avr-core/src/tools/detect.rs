//! 工具本地探测。

use std::path::Path;
use std::time::Duration;

use crate::error::{process_failed, AvrError};
use crate::process::{ProcessRequest, run_blocking};

pub fn detect_version(program: &Path) -> Result<String, AvrError> {
    let req = ProcessRequest {
        program: program.to_path_buf(),
        args: vec!["-version".into()],
        cwd: None,
        timeout: Some(Duration::from_secs(15)),
        expected_outputs: vec![],
    };
    let out = run_blocking(&req)?;
    let first = out
        .stdout
        .lines()
        .next()
        .or_else(|| out.stderr.lines().next())
        .unwrap_or("unknown")
        .to_string();
    if out.exit_code != 0 && first == "unknown" {
        return Err(process_failed(&out.stderr));
    }
    Ok(first)
}

pub fn file_exists(path: &Path) -> bool {
    path.is_file()
}
