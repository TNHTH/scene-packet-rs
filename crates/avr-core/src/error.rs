//! 业务错误（1:1 移植 V1 errors.py）。

use std::error::Error as StdError;
use std::sync::Arc;

use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorCode {
    UnsupportedUrl,
    ExtractFailed,
    CookiesRequired,
    NoTranscriptAsrNotConfigured,
    AsrModelMissing,
    AsrFailed,
    AsrBackendMissing,
    FfmpegMissing,
    YtdlpMissing,
    ToolNotReady,
    JobCancelled,
    ProcessFailed,
    ConfigError,
    IoError,
    CacheError,
    VisualProxyFailed,
    NetworkError,
}

impl ErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::UnsupportedUrl => "UNSUPPORTED_URL",
            Self::ExtractFailed => "EXTRACT_FAILED",
            Self::CookiesRequired => "COOKIES_REQUIRED",
            Self::NoTranscriptAsrNotConfigured => "NO_TRANSCRIPT_ASR_NOT_CONFIGURED",
            Self::AsrModelMissing => "ASR_MODEL_MISSING",
            Self::AsrFailed => "ASR_FAILED",
            Self::AsrBackendMissing => "ASR_BACKEND_MISSING",
            Self::FfmpegMissing => "FFMPEG_MISSING",
            Self::YtdlpMissing => "YTDLP_MISSING",
            Self::ToolNotReady => "TOOL_NOT_READY",
            Self::JobCancelled => "JOB_CANCELLED",
            Self::ProcessFailed => "PROCESS_FAILED",
            Self::ConfigError => "CONFIG_ERROR",
            Self::IoError => "IO_ERROR",
            Self::CacheError => "CACHE_ERROR",
            Self::VisualProxyFailed => "VISUAL_PROXY_FAILED",
            Self::NetworkError => "NETWORK_ERROR",
        }
    }
}

/// 底层错误，用 Arc 包装以便 `AvrError` 可 Clone（singleflight 需要把结果广播给多个等待者）。
pub type BoxedSource = Arc<dyn StdError + Send + Sync + 'static>;

/// 核心库错误。
///
/// - `code`：稳定的机器可读错误码，入口层据此映射退出码 / JSON / Tauri 错误。
/// - `message` / `hint`：面向用户的说明与下一步建议。
/// - `source`：底层原因（io / serde / reqwest / zip 等），通过 `Error::source()` 暴露，
///   `Display` 不重复输出它，避免错误链打印两遍。
#[derive(Debug, Error, Clone)]
#[non_exhaustive]
#[error("[{}] {}", code.as_str(), message)]
pub struct AvrError {
    pub code: ErrorCode,
    pub message: String,
    pub hint: String,
    #[source]
    source: Option<BoxedSource>,
}

impl AvrError {
    pub fn new(code: ErrorCode, message: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            hint: hint.into(),
            source: None,
        }
    }

    /// 挂上底层原因。
    pub fn with_source<E>(mut self, source: E) -> Self
    where
        E: StdError + Send + Sync + 'static,
    {
        self.source = Some(Arc::new(source));
        self
    }

    /// I/O 错误的统一构造：保留 `std::io::Error` 作为 source。
    pub fn io(err: std::io::Error, hint: impl Into<String>) -> Self {
        Self::new(ErrorCode::IoError, "文件读写失败", hint).with_source(err)
    }

    /// 面向用户的完整文本：错误码、说明、底层原因（若有）与下一步建议。
    pub fn format_user(&self) -> String {
        let mut s = format!("[{}] {}", self.code.as_str(), self.message);
        let mut cause = StdError::source(self);
        while let Some(c) = cause {
            s.push_str(&format!("\n原因: {c}"));
            cause = c.source();
        }
        s.push_str(&format!("\n下一步: {}", self.hint));
        s
    }
}

pub fn unsupported_url() -> AvrError {
    AvrError::new(
        ErrorCode::UnsupportedUrl,
        "当前仅支持哔哩哔哩与抖音链接。",
        "请换成 B 站或抖音 URL。",
    )
}

pub fn extract_failed() -> AvrError {
    AvrError::new(
        ErrorCode::ExtractFailed,
        "无法解析该视频。",
        "检查网络；抖音可在设置里把 Cookies 改为 chrome 或 edge（需已登录）。",
    )
}

pub fn cookies_required() -> AvrError {
    AvrError::new(
        ErrorCode::CookiesRequired,
        "抖音需要登录 Cookie 才能解析。",
        "打开设置，将「浏览器 Cookies」选为 chrome 或 edge，并确保该浏览器已登录抖音。",
    )
}

pub fn no_transcript_asr_not_configured() -> AvrError {
    AvrError::new(
        ErrorCode::NoTranscriptAsrNotConfigured,
        "没有字幕，且未配置本地语音识别模型。",
        "打开 ScenePacket 设置，配置 whisper.cpp 可执行文件与模型路径后再试。",
    )
}

pub fn asr_model_missing() -> AvrError {
    AvrError::new(
        ErrorCode::AsrModelMissing,
        "模型目录无效或不存在。",
        "在设置里重新选择本地模型文件夹，不要填写网上下载地址。",
    )
}

pub fn asr_failed(detail: &str) -> AvrError {
    let msg = if detail.is_empty() {
        "语音识别失败。".to_string()
    } else {
        format!("语音识别失败。 ({detail})")
    };
    AvrError::new(ErrorCode::AsrFailed, msg, "查看日志；确认模型与音频完整。")
}

pub fn ffmpeg_missing() -> AvrError {
    AvrError::new(
        ErrorCode::FfmpegMissing,
        "未找到 ffmpeg，无法抽音频。",
        "在「工具与模型」页安装或指定 ffmpeg 路径。",
    )
}

pub fn ytdlp_missing() -> AvrError {
    AvrError::new(
        ErrorCode::YtdlpMissing,
        "未找到 yt-dlp。",
        "在「工具与模型」页安装或指定 yt-dlp.exe 路径。",
    )
}

pub fn tool_not_ready(name: &str) -> AvrError {
    AvrError::new(
        ErrorCode::ToolNotReady,
        format!("工具未就绪: {name}"),
        "打开「工具与模型」页完成安装或手动指定路径。",
    )
}

pub fn process_failed(detail: &str) -> AvrError {
    AvrError::new(
        ErrorCode::ProcessFailed,
        format!("子进程执行失败: {detail}"),
        "查看 logs/ 目录获取详细输出。",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_is_preserved_and_not_duplicated_in_display() {
        let io = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "拒绝访问");
        let e = AvrError::io(io, "检查磁盘权限。");
        assert_eq!(e.to_string(), "[IO_ERROR] 文件读写失败");
        let src = StdError::source(&e).expect("source");
        assert_eq!(src.to_string(), "拒绝访问");
        let user = e.format_user();
        assert!(user.contains("原因: 拒绝访问") && user.contains("下一步"));
        // Clone 后 source 仍在。
        assert!(StdError::source(&e.clone()).is_some());
    }
}
