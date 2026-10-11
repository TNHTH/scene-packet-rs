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

    // total_cmp 对 NaN 也有全序，不会 panic。
    hints.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
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
            da.total_cmp(&db)
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
    std::fs::create_dir_all(frames_dir).ok();
    let mut out = Vec::new();
    for t in times {
        let name = format!("t_{:06.0}.jpg", t * 1000.0);
        let path = frames_dir.join(&name);
        capture_frame(ctx, video, *t, &path, width).await?;
        let uri = crate::paths::to_file_uri(&path);
        let ts = format_timestamp(*t);
        let markdown = format!("![{ts}](images/{name})");
        let base64_data = match delivery {
            FrameDelivery::Path => None,
            FrameDelivery::Inline | FrameDelivery::Both => {
                std::fs::read(&path).ok().map(|b| STANDARD.encode(&b))
            }
        };
        out.push(CapturedFrame {
            time: *t,
            path: path.display().to_string(),
            file_uri: uri,
            markdown,
            width,
            height: 0,
            base64: base64_data,
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

/// URL 视觉源四级 fallback 入口。
pub async fn resolve_visual_source(
    ytdlp: &crate::extract::ytdlp::YtDlpExtractor,
    _ffmpeg: &FfmpegContext,
    url: &str,
    cache_dir: &Path,
    timestamp: f64,
) -> Result<PathBuf, AvrError> {
    let local_video = cache_dir.join("source.mp4");
    if local_video.exists() {
        return Ok(local_video);
    }
    let proxy = cache_dir.join("visual_proxy.mp4");
    if proxy.exists() {
        return Ok(proxy);
    }

    let clip = cache_dir.join("clip.mp4");
    let start = (timestamp - 3.0).max(0.0);
    let end = timestamp + 3.0;
    if ytdlp.download_section(url, start, end, &clip).await.is_ok() && clip.exists() {
        return Ok(clip);
    }

    if ytdlp
        .download_section(url, 0.0, 3600.0, &proxy)
        .await
        .is_ok()
        && proxy.exists()
    {
        return Ok(proxy);
    }

    Err(crate::error::AvrError::new(
        crate::error::ErrorCode::VisualProxyFailed,
        "无法获取视频画面源",
        "检查网络与 Cookie 设置后重试。",
    ))
}

/// 自动候选限制在可解码的最后一帧；用户手动补帧保持原时间。
pub fn clamp_automatic_times(times: &mut [f64], extra: &[f64], probe: &crate::ffmpeg::MediaProbe) {
    let last = (probe.duration - 1.0 / probe.frame_rate).max(0.0);
    for time in times {
        if *time > last && !extra.iter().any(|t| (*t - *time).abs() < 0.001) {
            *time = (last * 1000.0).floor() / 1000.0;
        }
    }
}

/// 统一到毫秒，去掉重复，并拒绝落在时长之外的时间。
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{TranscriptSegment, TranscriptSource};

    fn transcript(starts: &[f64]) -> Transcript {
        Transcript {
            language: "zh".into(),
            source: TranscriptSource::Subtitle,
            segments: starts
                .iter()
                .map(|&s| TranscriptSegment {
                    start: s,
                    end: s + 1.0,
                    text: format!("seg {s}"),
                    suspect: None,
                })
                .collect(),
        }
    }

    /// 回归：场景分数或时间戳为 NaN 时不能 panic。
    #[test]
    fn nan_inputs_do_not_panic() {
        let t = transcript(&[0.0, f64::NAN, 5.0]);
        let scenes = [(1.0, f32::NAN), (f64::NAN, 0.9), (3.0, 0.5)];
        let hints = suggest_visual_hints(&t, &scenes, 10);
        assert_eq!(hints.len(), 3);
        // 非 NaN 分数按降序排列
        let finite: Vec<f32> = hints
            .iter()
            .map(|h| h.confidence)
            .filter(|c| !c.is_nan())
            .collect();
        assert!(finite.windows(2).all(|w| w[0] >= w[1]));
    }
}
