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
