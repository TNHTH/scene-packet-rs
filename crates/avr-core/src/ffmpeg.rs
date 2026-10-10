//! ffmpeg / ffprobe 适配。

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

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
    if !timestamp.is_finite() || timestamp < 0.0 || width == 0 {
        return Err(process_failed("时间必须为非负有限数，宽度必须大于零"));
    }
    let temporary = output.with_file_name(format!(".{}.jpg", uuid::Uuid::new_v4()));
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
            temporary.display().to_string(),
        ],
        cwd: None,
        timeout: Some(Duration::from_secs(120)),
        expected_outputs: vec![temporary.clone()],
    };
    run(&req).await?;
    if std::fs::metadata(&temporary).map(|m| m.len()).unwrap_or(0) == 0 {
        return Err(process_failed("截帧未生成有效图片"));
    }
    std::fs::rename(&temporary, output).map_err(|e| process_failed(&e.to_string()))?;
    Ok(output.to_path_buf())
}

/// scene 检测，返回 (时间, 分数) 列表。
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
            format!("select='gt(scene,{threshold})',metadata=print"),
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

fn parse_showinfo(stderr: &str) -> Vec<(f64, f32)> {
    let mut results = Vec::new();
    let mut time = None;
    let time_re = regex::Regex::new(r"pts_time:([0-9.]+)").unwrap();
    for line in stderr.lines() {
        if let Some(cap) = time_re.captures(line) {
            time = cap[1].parse::<f64>().ok();
        }
        if let Some(value) = line.split("lavfi.scene_score=").nth(1) {
            if let (Some(t), Ok(score)) = (time.take(), value.trim().parse::<f32>()) {
                results.push((t, score));
            }
        }
    }
    results
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaProbe {
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub frame_rate: f64,
    pub has_audio: bool,
}
pub async fn probe_media(ctx: &FfmpegContext, input: &Path) -> Result<MediaProbe, AvrError> {
    let req = ProcessRequest {
        program: ctx.ffprobe.clone(),
        args: vec![
            "-v".into(),
            "error".into(),
            "-show_entries".into(),
            "format=duration:stream=codec_type,width,height,avg_frame_rate,duration".into(),
            "-of".into(),
            "json".into(),
            input.display().to_string(),
        ],
        cwd: None,
        timeout: Some(Duration::from_secs(120)),
        expected_outputs: vec![],
    };
    let out = run(&req).await?;
    let data: serde_json::Value =
        serde_json::from_str(&out.stdout).map_err(|e| process_failed(&e.to_string()))?;
    let streams = data["streams"].as_array().cloned().unwrap_or_default();
    let video = streams.iter().find(|s| s["codec_type"] == "video");
    let frame_rate = video
        .and_then(|s| s["avg_frame_rate"].as_str())
        .and_then(|s| s.split_once('/'))
        .and_then(|(a, b)| Some(a.parse::<f64>().ok()? / b.parse::<f64>().ok()?))
        .filter(|n| n.is_finite() && *n > 0.0)
        .unwrap_or(30.0);
    Ok(MediaProbe {
        duration: video
            .and_then(|v| v["duration"].as_str())
            .or_else(|| data["format"]["duration"].as_str())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0),
        frame_rate,
        width: video.and_then(|s| s["width"].as_u64()).unwrap_or(0) as u32,
        height: video.and_then(|s| s["height"].as_u64()).unwrap_or(0) as u32,
        has_audio: streams.iter().any(|s| s["codec_type"] == "audio"),
    })
}
