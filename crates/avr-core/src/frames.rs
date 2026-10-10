//! 视觉候选点与截帧（Phase B）。

use std::path::{Path, PathBuf};

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};

use crate::config::AppConfig;
use crate::error::AvrError;
use crate::ffmpeg::{capture_frame, detect_scenes, FfmpegContext};
use crate::models::Transcript;
use crate::timeline::format_timestamp;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisualHint {
    pub time: f64,
    pub reason: String,
    pub confidence: f32,
    pub nearest_text: String,
    pub segment_id: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameDelivery {
    Path,
    Inline,
    Both,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapturedFrame {
    pub time: f64,
    pub path: String,
    pub file_uri: String,
    pub markdown: String,
    pub width: u32,
    pub height: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base64: Option<String>,
}

const TEXT_CUES: [&str; 12] = [
    "如图",
    "这张图",
    "大家看",
    "看这里",
    "如下所示",
    "这个界面",
    "箭头",
    "表格",
    "公式",
    "屏幕",
    "PPT",
    "幻灯片",
];

pub fn suggest_visual_hints(
    transcript: &Transcript,
    scene_times: &[(f64, f32)],
    threshold_limit: u32,
) -> Vec<VisualHint> {
    let mut hints = Vec::new();

    for (i, seg) in transcript.segments.iter().enumerate() {
        for cue in TEXT_CUES {
            if seg.text.contains(cue) {
                hints.push(VisualHint {
                    time: seg.start,
                    reason: format!("文本线索: {cue}"),
                    confidence: 0.85,
                    nearest_text: seg.text.clone(),
                    segment_id: Some(i as u32),
                });
                break;
            }
        }
    }

    for (t, score) in scene_times {
        hints.push(VisualHint {
            time: *t,
            reason: "画面场景切换".into(),
            confidence: *score * 0.6,
            nearest_text: nearest_segment_text(transcript, *t),
            segment_id: None,
        });
    }

    hints.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap());
    hints.truncate(threshold_limit as usize);
    hints
}

fn nearest_segment_text(transcript: &Transcript, t: f64) -> String {
    transcript
        .segments
        .iter()
        .min_by(|a, b| {
            let da = (a.start - t).abs();
            let db = (b.start - t).abs();
            da.partial_cmp(&db).unwrap()
        })
        .map(|s| s.text.clone())
        .unwrap_or_default()
}

pub async fn extract_frames(
    ctx: &FfmpegContext,
    video: &Path,
    times: &[f64],
    frames_dir: &Path,
    width: u32,
    delivery: FrameDelivery,
) -> Result<Vec<CapturedFrame>, AvrError> {
    if width == 0 {
        return Err(crate::error::process_failed("图片宽度必须大于零"));
    }
    let probe = crate::ffmpeg::probe_media(ctx, video).await?;
    let times = validate_times(times, probe.duration)?;
    std::fs::create_dir_all(frames_dir)
        .map_err(|e| crate::error::process_failed(&e.to_string()))?;
    let mut out = Vec::new();
    for t in times {
        let name = format!("t_{:09.0}.jpg", t * 1000.0);
        let path = frames_dir.join(&name);
        capture_frame(ctx, video, t, &path, width).await?;
        let dimensions = crate::ffmpeg::probe_media(ctx, &path).await?;
        let base64 = match delivery {
            FrameDelivery::Path => None,
            _ => Some(STANDARD.encode(
                std::fs::read(&path).map_err(|e| crate::error::process_failed(&e.to_string()))?,
            )),
        };
        out.push(CapturedFrame {
            time: t,
            path: path.display().to_string(),
            file_uri: crate::paths::to_file_uri(&path),
            markdown: format!("![{}](images/{name})", format_timestamp(t)),
            width: dimensions.width,
            height: dimensions.height,
            base64,
        });
    }
    Ok(out)
}

pub async fn detect_scene_hints(
    ctx: &FfmpegContext,
    video: &Path,
    cfg: &AppConfig,
) -> Result<Vec<(f64, f32)>, AvrError> {
    detect_scenes(ctx, video, cfg.ui.scene_threshold).await
}

/// 兼容旧调用方，但只返回从零开始的完整时间轴。
pub async fn resolve_visual_source(
    ytdlp: &crate::extract::ytdlp::YtDlpExtractor,
    _ffmpeg: &FfmpegContext,
    url: &str,
    cache_dir: &Path,
    _timestamp: f64,
) -> Result<PathBuf, AvrError> {
    let (info, _) = ytdlp.extract_metadata_with_info(url).await?;
    if !info.duration.is_finite() || info.duration <= 0.0 {
        return Err(crate::error::process_failed(
            "远程视频时长未知，无法建立完整视觉源",
        ));
    }
    let proxy = cache_dir.join("visual_full.mp4");
    if !proxy.is_file() {
        ytdlp
            .download_section(url, 0.0, info.duration, &proxy)
            .await?;
    }
    Ok(proxy)
}

/// 自动候选限制在可解码的最后一帧；用户手动补帧保持原时间，失败会明确返回。
pub fn clamp_automatic_times(times: &mut [f64], extra: &[f64], probe: &crate::ffmpeg::MediaProbe) {
    let last = (probe.duration - 1.0 / probe.frame_rate).max(0.0);
    for time in times {
        if *time > last && !extra.iter().any(|t| (*t - *time).abs() < 0.001) {
            *time = (last * 1000.0).floor() / 1000.0;
        }
    }
}

/// 统一到输出精度，避免重复记录及同名截图相互覆盖。
pub fn validate_times(times: &[f64], duration: f64) -> Result<Vec<f64>, AvrError> {
    if !duration.is_finite() || duration <= 0.0 {
        return Err(crate::error::process_failed("视频时长无效"));
    }
    let mut millis = Vec::new();
    for t in times {
        if !t.is_finite() || *t < 0.0 || *t >= duration {
            return Err(crate::error::process_failed(
                "截图时间必须在 [0, 视频时长) 内",
            ));
        }
        let ms = (t * 1000.0).round() as u64;
        if ms as f64 / 1000.0 >= duration {
            return Err(crate::error::process_failed("截图时间舍入后超过视频边界"));
        }
        millis.push(ms);
    }
    millis.sort_unstable();
    millis.dedup();
    Ok(millis.into_iter().map(|t| t as f64 / 1000.0).collect())
}

/// 均匀覆盖先占预算，其余预算留给场景候选；明确指定的补帧不静默丢弃。
pub fn sampling_times(
    duration: f64,
    interval: f64,
    limit: usize,
    extra: &[f64],
    scenes: &[(f64, f32)],
) -> Result<Vec<f64>, AvrError> {
    if !duration.is_finite()
        || duration <= 0.0
        || !interval.is_finite()
        || interval <= 0.0
        || !(2..=10000).contains(&limit)
    {
        return Err(crate::error::process_failed(
            "采样间隔必须大于零，预算须在2到10000之间",
        ));
    }
    let extras = validate_times(extra, duration)?;
    if extras.len() + 2 > limit {
        return Err(crate::error::process_failed(
            "指定补帧超过预算，请增加 max-frames",
        ));
    }
    let remaining = limit - extras.len();
    let desired = (duration / interval).ceil().min(10000.0) as usize + 1;
    let count = desired.min(remaining).max(2);
    let last = (duration - 0.05).max(0.0);
    let mut times = extras;
    for i in 0..count {
        times.push((last * i as f64 / (count - 1) as f64 * 1000.0).floor() / 1000.0);
    }
    times = validate_times(&times, duration)?;
    for (t, _) in scenes {
        if times.len() >= limit {
            break;
        }
        let t = (*t + 0.1).min(last);
        if t >= 0.0 && t.is_finite() && times.iter().all(|x| (*x - t).abs() > 0.1) {
            times.push(t);
        }
    }
    validate_times(&times, duration)
}

/// 复用本地原片和已有完整代理，不把六秒剪辑错当作原视频时间轴。
pub async fn cached_visual_source(
    ctx: &crate::pipeline::PipelineContext,
    key: &str,
) -> Result<PathBuf, AvrError> {
    if !crate::media::valid_media_key(key) {
        return Err(crate::error::process_failed("无效 media_key"));
    }
    let cache = crate::config::effective_cache_dir(&ctx.root, &ctx.config).join(key);
    let (video, _, _) = crate::cache::load_cached_transcript(&cache)?
        .ok_or_else(|| crate::error::process_failed("没有此媒体缓存"))?;
    if video.platform == crate::models::Platform::Local {
        let path = PathBuf::from(&video.url);
        if path.is_file() {
            return Ok(path);
        }
        return Err(crate::error::process_failed("本地原视频已移动或不存在"));
    }
    for name in ["source.mp4", "visual_full.mp4"] {
        let path = cache.join(name);
        if path.is_file() {
            return Ok(path);
        }
    }
    let extractor = crate::extract::ytdlp::YtDlpExtractor::new(&ctx.root, &ctx.config)?;
    if video.duration <= 0.0 {
        return Err(crate::error::process_failed(
            "远程视频时长未知，无法安全建立完整视觉源",
        ));
    }
    let path = cache.join("visual_full.mp4");
    extractor
        .download_section(&video.url, 0.0, video.duration, &path)
        .await?;
    Ok(path)
}
