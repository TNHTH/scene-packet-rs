//! ffmpeg / ffprobe 适配。

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::config::AppConfig;
use crate::error::{ffmpeg_missing, process_failed, AvrError};
use crate::process::{run, ProcessRequest};
use crate::root::DataRoot;
use crate::tools::registry::ToolRegistry;

#[derive(Debug, Deserialize)]
struct FfprobeFormat {
    duration: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FfprobeOutput {
    format: Option<FfprobeFormat>,
}

pub struct FfmpegContext {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

impl FfmpegContext {
    pub fn resolve(root: &DataRoot, cfg: &AppConfig) -> Result<Self, AvrError> {
        let reg = ToolRegistry::new(root, cfg);
        let ffmpeg = reg.ffmpeg_path()?;
        let ffprobe = reg.ffprobe_path()?;
        Ok(Self { ffmpeg, ffprobe })
    }
}

pub async fn probe_duration(ctx: &FfmpegContext, input: &Path) -> Result<f64, AvrError> {
    let req = ProcessRequest {
        program: ctx.ffprobe.clone(),
        args: vec![
            "-v".into(),
            "error".into(),
            "-show_entries".into(),
            "format=duration".into(),
            "-of".into(),
            "json".into(),
            input.display().to_string(),
        ],
        cwd: None,
        timeout: Some(Duration::from_secs(120)),
        expected_outputs: vec![],
    };
    let out = run(&req).await?;
    let parsed: FfprobeOutput =
        serde_json::from_str(&out.stdout).map_err(|e| process_failed(&e.to_string()))?;
    let dur = parsed
        .format
        .and_then(|f| f.duration)
        .and_then(|d| d.parse::<f64>().ok())
        .unwrap_or(0.0);
    Ok(dur)
}

/// 抽取 16kHz 单声道 wav。
pub async fn extract_audio_wav(
    ctx: &FfmpegContext,
    input: &Path,
    output: &Path,
) -> Result<PathBuf, AvrError> {
    if !ctx.ffmpeg.exists() {
        return Err(ffmpeg_missing());
    }
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let req = ProcessRequest {
        program: ctx.ffmpeg.clone(),
        args: vec![
            "-y".into(),
            "-i".into(),
            input.display().to_string(),
            "-vn".into(),
            "-acodec".into(),
            "pcm_s16le".into(),
            "-ar".into(),
            "16000".into(),
            "-ac".into(),
            "1".into(),
            output.display().to_string(),
        ],
        cwd: None,
        timeout: Some(Duration::from_secs(3600)),
        expected_outputs: vec![output.to_path_buf()],
    };
    run(&req).await?;
    Ok(output.to_path_buf())
}

/// 精确 seek 截帧。
pub async fn capture_frame(
    ctx: &FfmpegContext,
    input: &Path,
    timestamp: f64,
    output: &Path,
    width: u32,
) -> Result<PathBuf, AvrError> {
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let scale = format!("scale='min({width},iw)':-2");
    let req = ProcessRequest {
        program: ctx.ffmpeg.clone(),
        args: vec![
            "-y".into(),
            "-ss".into(),
            format!("{timestamp:.3}"),
            "-i".into(),
            input.display().to_string(),
            "-frames:v".into(),
            "1".into(),
            "-q:v".into(),
            "2".into(),
            "-vf".into(),
            scale,
            output.display().to_string(),
        ],
        cwd: None,
        timeout: Some(Duration::from_secs(120)),
        expected_outputs: vec![output.to_path_buf()],
    };
    run(&req).await?;
    Ok(output.to_path_buf())
}

/// scene 检测，返回 (时间, 分数) 列表。
/// 按长边与 JPEG 质量截一帧（P5-2）：长边缩到 `long_side`（不放大），`-q:v` 由 [`crate::keyframes::jpeg_qscale`] 换算。
pub async fn capture_frame_spec(
    ctx: &FfmpegContext,
    input: &Path,
    timestamp: f64,
    output: &Path,
    long_side: u32,
    quality: u8,
) -> Result<PathBuf, AvrError> {
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let req = ProcessRequest {
        program: ctx.ffmpeg.clone(),
        args: vec![
            "-y".into(),
            "-ss".into(),
            format!("{timestamp:.3}"),
            "-i".into(),
            input.display().to_string(),
            "-frames:v".into(),
            "1".into(),
            "-q:v".into(),
            crate::keyframes::jpeg_qscale(quality).to_string(),
            "-vf".into(),
            long_side_scale(long_side),
            output.display().to_string(),
        ],
        cwd: None,
        timeout: Some(Duration::from_secs(120)),
        expected_outputs: vec![output.to_path_buf()],
    };
    run(&req).await?;
    Ok(output.to_path_buf())
}

/// 长边不超过 `l` 的 scale 滤镜（保持比例、偶数尺寸、不放大）。
pub fn long_side_scale(l: u32) -> String {
    format!("scale='if(gte(iw,ih),min({l},iw),-2)':'if(gte(iw,ih),-2,min({l},ih))'")
}

pub async fn detect_scenes(
    ctx: &FfmpegContext,
    input: &Path,
    threshold: f32,
) -> Result<Vec<(f64, f32)>, AvrError> {
    let req = ProcessRequest {
        program: ctx.ffmpeg.clone(),
        args: vec![
            "-i".into(),
            input.display().to_string(),
            "-vf".into(),
            format!("select='gt(scene,{threshold})',showinfo"),
            "-f".into(),
            "null".into(),
            "-".into(),
        ],
        cwd: None,
        timeout: Some(Duration::from_secs(600)),
        expected_outputs: vec![],
    };
    let out = run(&req).await?;
    Ok(parse_showinfo(&out.stderr))
}

static PTS_TIME_RE: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"pts_time:([0-9.]+)").expect("valid regex"));

fn parse_showinfo(stderr: &str) -> Vec<(f64, f32)> {
    let mut results = Vec::new();
    for line in stderr.lines() {
        if line.contains("showinfo") {
            if let Some(m) = PTS_TIME_RE.captures(line) {
                let t = m[1].parse::<f64>().unwrap_or(0.0);
                results.push((t, 1.0));
            }
        }
    }
    results
}
