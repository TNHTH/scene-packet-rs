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
    model.is_file()
        && (cfg.asr.vad_model_path.is_empty() || Path::new(&cfg.asr.vad_model_path).is_file())
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

    let final_srt =
        PathBuf::from(audio.with_extension("").to_string_lossy().to_string() + "_asr.srt");
    let out_base = audio
        .with_file_name(format!(".asr-{}", uuid::Uuid::new_v4()))
        .to_string_lossy()
        .into_owned();
    if cfg.asr.threads == Some(0) {
        return Err(asr_failed("threads 必须大于 0"));
    }
    let mut args = preset.args(&model, audio, language, &out_base);
    if let Some(n) = cfg.asr.threads {
        args.extend(["-t".into(), n.to_string()]);
    }
    if !cfg.asr.prompt.is_empty() {
        args.extend(["--prompt".into(), cfg.asr.prompt.clone()]);
    }
    if cfg.asr.suppress_non_speech {
        args.push("-sns".into());
    }
    if cfg.asr.no_fallback {
        args.push("-nf".into());
    }
    if preset.id == "whisper-cpp-cpu" {
        args.push("-ng".into());
    }
    if !cfg.asr.vad_model_path.is_empty() {
        if !Path::new(&cfg.asr.vad_model_path).is_file() {
            return Err(asr_failed("VAD 模型文件不存在"));
        }
        args.extend([
            "--vad".into(),
            "--vad-model".into(),
            cfg.asr.vad_model_path.clone(),
            "--vad-speech-pad-ms".into(),
            cfg.asr.vad_speech_pad_ms.to_string(),
            "--vad-min-silence-duration-ms".into(),
            cfg.asr.vad_min_silence_ms.to_string(),
        ]);
    }
    // 原始输出保留在与有效配置绑定的缓存目录中。

    let req = ProcessRequest {
        program: cli,
        args,
        cwd: None,
        timeout: Some(std::time::Duration::from_secs(7200)),
        expected_outputs: vec![PathBuf::from(format!("{out_base}.srt"))],
    };

    let output = run(&req).await.map_err(|e| asr_failed(&e.message))?;
    let srt_path = PathBuf::from(format!("{out_base}.srt"));
    if !srt_path.exists() {
        return Err(asr_failed(&output.stderr));
    }
    let content = std::fs::read_to_string(&srt_path).map_err(|e| asr_failed(&e.to_string()))?;
    let segments = parse_srt(&content, language).segments;
    std::fs::rename(&srt_path, &final_srt).map_err(|e| asr_failed(&e.to_string()))?;
    Ok(segments)
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

/// 指纹绑定有效选项与实际工具/模型内容；同路径替换也会产生新处理身份。
pub fn processing_fingerprint(root: &DataRoot, cfg: &AppConfig) -> Result<String, AvrError> {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(b"scenepacket-asr-v3");
    hash.update(serde_json::to_vec(&cfg.asr).map_err(|e| asr_failed(&e.to_string()))?);
    let registry = ToolRegistry::new(root, cfg);
    for path in [
        registry.whisper_cli_path().ok(),
        Some(resolve_model(
            root,
            cfg,
            &presets::get_preset(&cfg.asr.active_preset),
        )),
        (!cfg.asr.vad_model_path.is_empty()).then(|| PathBuf::from(&cfg.asr.vad_model_path)),
    ]
    .into_iter()
    .flatten()
    {
        hash.update(path.to_string_lossy().as_bytes());
        if path.is_file() {
            let mut file = std::fs::File::open(&path).map_err(|e| asr_failed(&e.to_string()))?;
            let mut chunk = [0u8; 65536];
            loop {
                let n = std::io::Read::read(&mut file, &mut chunk)
                    .map_err(|e| asr_failed(&e.to_string()))?;
                if n == 0 {
                    break;
                }
                hash.update(&chunk[..n]);
            }
        } else {
            hash.update(b"missing");
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}
