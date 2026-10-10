//! 外部 ASR 命令引擎适配器。

use std::path::{Path, PathBuf};

use crate::config::AppConfig;
use crate::error::{asr_failed, no_transcript_asr_not_configured, AvrError};
use crate::models::TranscriptSegment;
use crate::process::{run, ProcessRequest};
use crate::root::DataRoot;
use crate::timeline::parse_srt;
use crate::tools::registry::ToolRegistry;

use crate::asr::presets;

pub fn preset_ready(root: &DataRoot, cfg: &AppConfig, preset_id: &str) -> bool {
    let preset = presets::get_preset(preset_id);
    let reg = ToolRegistry::new(root, cfg);
    if reg.whisper_cli_path().is_err() {
        return false;
    }
    let model = resolve_model(root, cfg, &preset);
    model.exists()
}

pub async fn run_preset(
    root: &DataRoot,
    cfg: &AppConfig,
    preset_id: &str,
    audio: &Path,
    language: &str,
) -> Result<Vec<TranscriptSegment>, AvrError> {
    let preset = presets::get_preset(preset_id);
    let reg = ToolRegistry::new(root, cfg);
    let cli = reg.whisper_cli_path()?;
    let model = resolve_model(root, cfg, &preset);
    if !model.exists() {
        return Err(no_transcript_asr_not_configured());
    }

    let out_base = audio.with_extension("").to_string_lossy().to_string() + "_asr";
    let args = preset.args(&model, audio, language, &out_base);

    let req = ProcessRequest {
        program: cli,
        args,
        cwd: audio.parent().map(PathBuf::from),
        timeout: Some(std::time::Duration::from_secs(7200)),
        expected_outputs: vec![PathBuf::from(format!("{out_base}.srt"))],
    };

    let output = run(&req).await.map_err(|e| asr_failed(&e.message))?;
    let srt_path = PathBuf::from(format!("{out_base}.srt"));
    if !srt_path.exists() {
        return Err(asr_failed(&output.stderr));
    }
    let content = std::fs::read_to_string(&srt_path).map_err(|e| asr_failed(&e.to_string()))?;
    Ok(parse_srt(&content, language).segments)
}

fn resolve_model(root: &DataRoot, cfg: &AppConfig, _preset: &super::presets::Preset) -> PathBuf {
    if !cfg.tools.whisper_model_path.is_empty() {
        return PathBuf::from(&cfg.tools.whisper_model_path);
    }
    if !cfg.asr.model_path.is_empty() {
        return PathBuf::from(&cfg.asr.model_path);
    }
    super::presets::default_model_path(&root.models_dir())
}
