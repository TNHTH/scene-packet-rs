//! 工具路径统一 API。

use std::path::{Path, PathBuf};

use crate::config::AppConfig;
use crate::root::DataRoot;
use crate::error::{ffmpeg_missing, tool_not_ready, ytdlp_missing, AvrError};
use crate::tools::detect;

pub struct ToolRegistry {
    root: DataRoot,
    config: AppConfig,
}

impl ToolRegistry {
    pub fn new(root: &DataRoot, config: &AppConfig) -> Self {
        Self {
            root: root.clone(),
            config: config.clone(),
        }
    }

    pub fn ffmpeg_path(&self) -> Result<PathBuf, AvrError> {
        resolve_tool(
            &self.config.tools.ffmpeg_path,
            self.root.bin_dir().join("ffmpeg/ffmpeg.exe"),
        )
        .ok_or_else(ffmpeg_missing)
    }

    pub fn ffprobe_path(&self) -> Result<PathBuf, AvrError> {
        resolve_tool(
            &self.config.tools.ffprobe_path,
            self.root.bin_dir().join("ffmpeg/ffprobe.exe"),
        )
        .ok_or_else(ffmpeg_missing)
    }

    pub fn ytdlp_path(&self) -> Result<PathBuf, AvrError> {
        resolve_tool(
            &self.config.tools.ytdlp_path,
            self.root.bin_dir().join("yt-dlp/yt-dlp.exe"),
        )
        .ok_or_else(ytdlp_missing)
    }

    pub fn whisper_cli_path(&self) -> Result<PathBuf, AvrError> {
        resolve_tool(
            &self.config.tools.whisper_cli_path,
            self.root.bin_dir().join("whisper/whisper-cli.exe"),
        )
        .ok_or_else(|| tool_not_ready("whisper-cli"))
    }

    pub fn whisper_model_path(&self) -> PathBuf {
        if !self.config.tools.whisper_model_path.is_empty() {
            PathBuf::from(&self.config.tools.whisper_model_path)
        } else if !self.config.asr.model_path.is_empty() {
            PathBuf::from(&self.config.asr.model_path)
        } else {
            self.root.models_dir().join("ggml-large-v3-turbo.bin")
        }
    }

    pub fn tool_status(&self, id: &str) -> ToolStatus {
        let path_result = match id {
            "ffmpeg" => self.ffmpeg_path(),
            "ffprobe" => self.ffprobe_path(),
            "yt-dlp" => self.ytdlp_path(),
            "whisper" => self.whisper_cli_path(),
            _ => return ToolStatus {
                id: id.to_string(),
                ready: false,
                path: None,
                version: None,
            },
        };
        let ok = path_result.is_ok();
        let path = path_result.ok();
        let version = path
            .as_ref()
            .and_then(|p| detect::detect_version(p).ok());
        ToolStatus {
            id: id.to_string(),
            ready: ok,
            path,
            version,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ToolStatus {
    pub id: String,
    pub ready: bool,
    pub path: Option<PathBuf>,
    pub version: Option<String>,
}

fn resolve_tool(manual: &str, default: PathBuf) -> Option<PathBuf> {
    if !manual.is_empty() {
        let p = PathBuf::from(manual);
        if detect::file_exists(&p) {
            return Some(p);
        }
    }
    if detect::file_exists(&default) {
        return Some(default);
    }
    None
}
