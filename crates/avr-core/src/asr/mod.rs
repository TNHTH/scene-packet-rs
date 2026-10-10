pub mod engine;
pub mod presets;

use crate::config::AppConfig;
use crate::error::AvrError;
use crate::models::{Transcript, TranscriptSource};
use crate::root::DataRoot;
use std::path::Path;

/// 选择 ASR preset（1:1 移植 V1 select_provider 逻辑，映射到 whisper preset）。
pub fn select_preset(default: &str, language: Option<&str>, platform: Option<&str>) -> String {
    if default == "whisper-cpp-cpu" || default == "whisper-cpp-cublas" {
        return default.to_string();
    }
    if let Some(lang) = language {
        if lang.to_lowercase().starts_with("zh") {
            return "whisper-cpp-cpu".to_string();
        }
        return "whisper-cpp-cublas".to_string();
    }
    if platform == Some("bilibili") || platform == Some("douyin") {
        return "whisper-cpp-cpu".to_string();
    }
    "whisper-cpp-cublas".to_string()
}

pub struct AsrService {
    root: DataRoot,
    config: AppConfig,
}

impl AsrService {
    pub fn new(root: DataRoot, config: AppConfig) -> Self {
        Self { root, config }
    }

    pub fn is_ready(&self, preset: &str) -> bool {
        engine::preset_ready(&self.root, &self.config, preset)
    }

    pub fn active_preset(&self, language: Option<&str>, platform: Option<&str>) -> String {
        let default = &self.config.asr.active_preset;
        if default != "auto" {
            return default.clone();
        }
        select_preset("auto", language, platform)
    }

    pub async fn transcribe(
        &self,
        audio: &Path,
        language: Option<&str>,
        platform: Option<&str>,
    ) -> Result<Transcript, AvrError> {
        let preset = self.active_preset(language, platform);
        if !self.is_ready(&preset) {
            return Err(crate::error::no_transcript_asr_not_configured());
        }
        let lang = language.unwrap_or(&self.config.asr.language);
        let segments = engine::run_preset(&self.root, &self.config, &preset, audio, lang).await?;
        Ok(Transcript {
            language: lang.to_string(),
            source: TranscriptSource::Asr,
            segments,
        })
    }
}
