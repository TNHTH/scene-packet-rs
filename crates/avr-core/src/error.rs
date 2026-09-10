//! 业务错误（1:1 移植 V1 errors.py）。

use std::fmt;

use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Error, Clone)]
pub struct AvrError {
    pub code: ErrorCode,
    pub message: String,
    pub hint: String,
}

impl AvrError {
    pub fn new(code: ErrorCode, message: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            hint: hint.into(),
        }
    }

    pub fn format_user(&self) -> String {
        format!(
            "[{}] {}\n下一步: {}",
            self.code.as_str(),
            self.message,
            self.hint
        )
    }
}

impl fmt::Display for AvrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.format_user())
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
