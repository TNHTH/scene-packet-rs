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
    if !presets::is_supported(preset_id) {
        return false;
    }
    let reg = ToolRegistry::new(root, cfg);
    if reg.whisper_cli_path().is_err() {
        return false;
    }
    let model = reg.whisper_model_path();
    model.exists()
}

pub async fn run_preset(
    root: &DataRoot,
    cfg: &AppConfig,
    preset_id: &str,
    audio: &Path,
    language: &str,
) -> Result<Vec<TranscriptSegment>, AvrError> {
    if !presets::is_supported(preset_id) {
        return Err(asr_failed("不支持的 ASR preset"));
    }
    let preset = presets::get_preset(preset_id);
    let reg = ToolRegistry::new(root, cfg);
    let cli = reg.whisper_cli_path()?;
    let model = reg.whisper_model_path();
    if !model.exists() {
        return Err(no_transcript_asr_not_configured());
    }

    let out_base = audio.with_extension("").to_string_lossy().to_string() + "_asr";
    let args = preset.args(
        &model,
        audio,
        language,
        &out_base,
        &cfg.asr.zh_initial_prompt,
    );

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
    Ok(finish(parse_srt(&content, language).segments, language))
}

/// 转写后处理：zh 先逐字繁→简，再标疑似幻觉（P4-3）。
pub fn finish(mut segments: Vec<TranscriptSegment>, language: &str) -> Vec<TranscriptSegment> {
    if language == "zh" {
        for s in &mut segments {
            s.text = super::zh::t2s(&s.text);
        }
    }
    super::postprocess::postprocess(segments)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(t: &str) -> TranscriptSegment {
        TranscriptSegment {
            start: 0.0,
            end: 1.0,
            text: t.into(),
            suspect: None,
        }
    }

    #[test]
    fn t2s_only_for_zh() {
        assert_eq!(finish(vec![seg("長征五號")], "zh")[0].text, "长征五号");
        assert_eq!(finish(vec![seg("長征五號")], "yue")[0].text, "長征五號");
        assert_eq!(finish(vec![seg("長征五號")], "auto")[0].text, "長征五號");
    }
}
