//! 导出 Obsidian 素材包等。

use std::path::{Path, PathBuf};

use crate::error::{AvrError, ErrorCode};
use crate::models::Transcript;

pub fn export_obsidian_bundle(
    export_dir: &Path,
    transcript: &Transcript,
    title: &str,
) -> Result<PathBuf, AvrError> {
    std::fs::create_dir_all(export_dir).map_err(io_err)?;
    let images = export_dir.join("images");
    std::fs::create_dir_all(&images).map_err(io_err)?;

    let md = export_dir.join("summary.md");
    let body = format!("# {}\n\n{}", title, transcript.full_text());
    std::fs::write(&md, body).map_err(io_err)?;
    Ok(md)
}

fn io_err(e: std::io::Error) -> AvrError {
    AvrError::new(ErrorCode::IoError, e.to_string(), "检查导出目录权限。")
}
