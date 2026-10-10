//! 配置读写（TOML）。

use std::fs;

use serde::{Deserialize, Serialize};

use crate::error::{AvrError, ErrorCode};
use crate::root::DataRoot;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub extractor: ExtractorConfig,
    #[serde(default)]
    pub asr: AsrConfig,
    #[serde(default)]
    pub output: OutputConfig,
    #[serde(default)]
    pub tools: ToolsConfig,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub mcp: McpConfig,
}

/// MCP 服务配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpConfig {
    /// `response_mode=auto` 时，转写原文估算 token 数不超过此值就直接返回 full，否则返回 index。
    /// 默认 2000（与当前 compact 相当的保守值），**待效果集确定**。
    #[serde(default = "default_auto_full_max_tokens")]
    pub auto_full_max_tokens: usize,
    /// 是否把工具调用日志追加到 `<数据根>/logs/mcp-calls.jsonl`（默认关闭；stderr 总是写）。
    #[serde(default)]
    pub call_log: bool,
    /// 文件日志轮转阈值（字节），超过后改名为 `.1`（覆盖旧 `.1`）。默认 5MB。
    #[serde(default = "default_call_log_max_bytes")]
    pub call_log_max_bytes: u64,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            auto_full_max_tokens: default_auto_full_max_tokens(),
            call_log: false,
            call_log_max_bytes: default_call_log_max_bytes(),
        }
    }
}

fn default_auto_full_max_tokens() -> usize {
    2000
}

fn default_call_log_max_bytes() -> u64 {
    5 * 1024 * 1024
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractorConfig {
    #[serde(default = "default_ytdlp")]
    pub name: String,
    #[serde(default = "default_none")]
    pub cookies_from_browser: String,
    /// Netscape cookies.txt（抖音推荐；优先于 cookies_from_browser）
    #[serde(default)]
    pub cookies_file: String,
    /// yt-dlp --user-agent（抖音 Cookie 须与导出浏览器 UA 一致）
    #[serde(default)]
    pub user_agent: String,
}

fn default_ytdlp() -> String {
    "ytdlp".to_string()
}
fn default_none() -> String {
    "none".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsrConfig {
    #[serde(default = "default_auto")]
    pub default: String,
    #[serde(default = "default_whisper_preset")]
    pub active_preset: String,
    #[serde(default)]
    pub model_path: String,
}

fn default_auto() -> String {
    "auto".to_string()
}
fn default_whisper_preset() -> String {
    "whisper-cpp-cpu".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OutputConfig {
    #[serde(default)]
    pub cache_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolsConfig {
    #[serde(default)]
    pub ffmpeg_path: String,
    #[serde(default)]
    pub ffprobe_path: String,
    #[serde(default)]
    pub ytdlp_path: String,
    #[serde(default)]
    pub whisper_cli_path: String,
    #[serde(default)]
    pub whisper_model_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiConfig {
    #[serde(default = "default_true")]
    pub enable_transparency: bool,
    #[serde(default = "default_scene_threshold")]
    pub scene_threshold: f32,
    #[serde(default = "default_hint_limit")]
    pub visual_hint_limit: u32,
}

fn default_true() -> bool {
    true
}
fn default_scene_threshold() -> f32 {
    0.3
}
fn default_hint_limit() -> u32 {
    50
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            mcp: McpConfig::default(),
            extractor: ExtractorConfig {
                name: default_ytdlp(),
                cookies_from_browser: default_none(),
                cookies_file: String::new(),
                user_agent: String::new(),
            },
            asr: AsrConfig {
                default: default_auto(),
                active_preset: default_whisper_preset(),
                model_path: String::new(),
            },
            output: OutputConfig {
                cache_dir: String::new(),
            },
            tools: ToolsConfig::default(),
            ui: UiConfig {
                enable_transparency: true,
                scene_threshold: 0.3,
                visual_hint_limit: 50,
            },
        }
    }
}

impl Default for ExtractorConfig {
    fn default() -> Self {
        Self {
            name: default_ytdlp(),
            cookies_from_browser: default_none(),
            cookies_file: String::new(),
            user_agent: String::new(),
        }
    }
}

impl Default for AsrConfig {
    fn default() -> Self {
        Self {
            default: default_auto(),
            active_preset: default_whisper_preset(),
            model_path: String::new(),
        }
    }
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            enable_transparency: true,
            scene_threshold: 0.3,
            visual_hint_limit: 50,
        }
    }
}

pub fn load_config(root: &DataRoot) -> Result<AppConfig, AvrError> {
    let path = root.config_path();
    if !path.exists() {
        let cfg = AppConfig::default();
        save_config(root, &cfg)?;
        return Ok(cfg);
    }
    let content = fs::read_to_string(&path).map_err(io_err)?;
    let cfg: AppConfig = toml::from_str(&content).map_err(|e| {
        AvrError::new(
            ErrorCode::ConfigError,
            e.to_string(),
            "检查 config.toml 格式。",
        )
    })?;
    Ok(cfg)
}

pub fn save_config(root: &DataRoot, cfg: &AppConfig) -> Result<(), AvrError> {
    root.ensure_layout()?;
    let content = toml::to_string_pretty(cfg).map_err(|e| {
        AvrError::new(ErrorCode::ConfigError, "序列化配置失败", "检查配置内容。").with_source(e)
    })?;
    fs::write(root.config_path(), content).map_err(io_err)?;
    Ok(())
}

pub fn effective_cache_dir(root: &DataRoot, cfg: &AppConfig) -> std::path::PathBuf {
    if !cfg.output.cache_dir.is_empty() {
        std::path::PathBuf::from(&cfg.output.cache_dir)
    } else {
        root.cache_dir()
    }
}

fn io_err(e: std::io::Error) -> AvrError {
    AvrError::io(e, "检查磁盘权限。")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn save_and_load_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let root = DataRoot {
            path: tmp.path().to_path_buf(),
        };
        let cfg = AppConfig::default();
        save_config(&root, &cfg).unwrap();
        let loaded = load_config(&root).unwrap();
        assert_eq!(loaded.asr.active_preset, "whisper-cpp-cpu");
    }
}
