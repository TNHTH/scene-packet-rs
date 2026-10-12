//! 工具本地探测。

use std::path::Path;
use std::time::Duration;

use super::ToolId;
use crate::error::{process_failed, AvrError};
use crate::process::{run, run_blocking, ProcessOutput, ProcessRequest};

pub fn detect_version(program: &Path) -> Result<String, AvrError> {
    version(run_blocking(&request(program, ToolId::Ffmpeg))?)
}

fn request(program: &Path, id: ToolId) -> ProcessRequest {
    ProcessRequest {
        program: program.to_path_buf(),
        args: vec![match id {
            ToolId::YtDlp => "--version",
            ToolId::Whisper => "--help",
            _ => "-version",
        }
        .into()],
        cwd: None,
        timeout: Some(Duration::from_secs(15)),
        expected_outputs: vec![],
    }
}

pub fn detect_tool_version(program: &Path, id: ToolId) -> Result<String, AvrError> {
    version(run_blocking(&request(program, id))?)
}

pub async fn detect_tool_version_async(program: &Path, id: ToolId) -> Result<String, AvrError> {
    version(run(&request(program, id)).await?)
}

fn version(out: ProcessOutput) -> Result<String, AvrError> {
    let first = out
        .stdout
        .lines()
        .chain(out.stderr.lines())
        .find(|s| !s.trim().is_empty())
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
